//! 非 GNOME Wayland 的 GlobalShortcuts Portal 后端。
//!
//! Portal 的快捷键绑定属于长生命周期 session，而且每个 session 只能调用一次
//! `BindShortcuts`。因此不能把它伪装成 Tauri 插件的逐键 register/unregister API：
//! 配置变化或录制快捷键时关闭整段 session，恢复时再用完整配置建立新 session。

use crate::app::shortcuts::{clear_register_failure, handle_portal_action};
use crate::models::AppConfig;
use crate::platform::DesktopSession;
use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
use ashpd::desktop::{CreateSessionOptions, Session};
use futures_util::{Stream, StreamExt};
use std::collections::HashSet;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::time::Duration;
use tauri::AppHandle;
use tokio::sync::{mpsc, watch};

const PAUSE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone)]
struct PortalBinding {
    action: &'static str,
    shortcut: String,
    description: &'static str,
    trigger: String,
}

struct PortalPlan {
    valid: Vec<PortalBinding>,
    invalid: Vec<(&'static str, String, String)>,
    unset: Vec<&'static str>,
}

enum WorkerCommand {
    Activate {
        generation: u64,
        config: Box<AppConfig>,
    },
    Pause(std_mpsc::SyncSender<Result<(), String>>),
}

struct ActivePortal {
    _proxy: GlobalShortcuts,
    session: Session<GlobalShortcuts>,
    activations: Pin<Box<dyn Stream<Item = String> + Send>>,
    bindings: Vec<PortalBinding>,
}

/// 向唯一 Portal worker 发送会话变更；worker 独占 session，避免旧会话和新会话同时响应。
pub struct PortalShortcutManager {
    sender: mpsc::UnboundedSender<WorkerCommand>,
    cancellation: watch::Sender<u64>,
    generation: AtomicU64,
}

impl PortalShortcutManager {
    pub fn new(app: AppHandle) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        let (cancellation, cancellation_receiver) = watch::channel(0);
        tauri::async_runtime::spawn(run_worker(app, receiver, cancellation_receiver));
        Self {
            sender,
            cancellation,
            generation: AtomicU64::new(0),
        }
    }

    /// 异步重建绑定。Bind 可能打开系统确认界面，不能阻塞 Tauri setup 或设置保存命令。
    pub fn activate(&self, config: AppConfig) -> Result<(), String> {
        let generation = self.next_generation();
        self.sender
            .send(WorkerCommand::Activate {
                generation,
                config: Box::new(config),
            })
            .map_err(|_| "GlobalShortcuts Portal worker 已停止".to_string())
    }

    /// 录制按键前必须等旧 session 真正关闭，否则 compositor 仍会吃掉待录制组合键。
    pub fn pause(&self) -> Result<(), String> {
        self.next_generation();
        let (sender, receiver) = std_mpsc::sync_channel(1);
        self.sender
            .send(WorkerCommand::Pause(sender))
            .map_err(|_| "GlobalShortcuts Portal worker 已停止".to_string())?;
        receiver
            .recv_timeout(PAUSE_TIMEOUT)
            .map_err(|_| "关闭 GlobalShortcuts Portal 会话超时".to_string())?
    }

    fn next_generation(&self) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        self.cancellation.send_replace(generation);
        generation
    }
}

async fn run_worker(
    app: AppHandle,
    mut receiver: mpsc::UnboundedReceiver<WorkerCommand>,
    mut cancellation: watch::Receiver<u64>,
) {
    let mut active: Option<ActivePortal> = None;

    loop {
        if active.is_none() {
            let Some(command) = receiver.recv().await else {
                return;
            };
            if !handle_command(&app, &mut active, command, &mut cancellation).await {
                return;
            }
            continue;
        }

        let portal = active.as_mut().expect("active checked above");
        tokio::select! {
            biased;
            command = receiver.recv() => {
                let Some(command) = command else {
                    let _ = close_active(&mut active).await;
                    return;
                };
                if !handle_command(&app, &mut active, command, &mut cancellation).await {
                    return;
                }
            }
            shortcut_id = portal.activations.next() => {
                match shortcut_id {
                    Some(shortcut_id) => handle_portal_action(&app, &shortcut_id),
                    None => {
                        let bindings = portal.bindings.clone();
                        let close_error = close_active(&mut active).await.err();
                        let reason = close_error.map_or_else(
                            || "GlobalShortcuts Portal 激活信号流已结束".to_string(),
                            |error| format!("GlobalShortcuts Portal 激活信号流已结束；{error}"),
                        );
                        report_binding_failure(&app, &bindings, &reason);
                    }
                }
            }
        }
    }
}

async fn handle_command(
    app: &AppHandle,
    active: &mut Option<ActivePortal>,
    command: WorkerCommand,
    cancellation: &mut watch::Receiver<u64>,
) -> bool {
    match command {
        WorkerCommand::Activate { generation, config } => {
            if let Err(error) = close_active(active).await {
                log::warn!("重建 Portal 快捷键前关闭旧会话失败: {error}");
            }
            activate_config(app, active, &config, generation, cancellation).await;
            true
        }
        WorkerCommand::Pause(response) => {
            let result = close_active(active).await;
            let _ = response.send(result);
            true
        }
    }
}

async fn activate_config(
    app: &AppHandle,
    active: &mut Option<ActivePortal>,
    config: &AppConfig,
    generation: u64,
    cancellation: &mut watch::Receiver<u64>,
) {
    let plan = plan_portal_registration(config);
    clear_register_failure(app, &plan.unset);
    for (action, shortcut, reason) in plan.invalid {
        crate::record_register_result(
            app,
            &[action],
            &shortcut,
            DesktopSession::Wayland,
            Err(reason),
        );
    }
    if plan.valid.is_empty() {
        return;
    }

    // `borrow_and_update` 同时把当前代次标记为已观察；若只用 `borrow`，后面的
    // `changed()` 会把创建 manager 时的旧版本误当成一次新取消，首个 Bind 会自我终止。
    if !begin_generation(cancellation, generation) {
        return;
    }
    match bind_session(app, plan.valid, generation, cancellation).await {
        Ok(portal) => *active = Some(portal),
        Err(BindFailure::Failed(bindings, reason)) => {
            report_binding_failure(app, &bindings, &reason);
        }
        Err(BindFailure::Cancelled) => {}
    }
}

enum BindFailure {
    Failed(Vec<PortalBinding>, String),
    Cancelled,
}

async fn bind_session(
    app: &AppHandle,
    bindings: Vec<PortalBinding>,
    generation: u64,
    cancellation: &mut watch::Receiver<u64>,
) -> Result<ActivePortal, BindFailure> {
    let proxy = GlobalShortcuts::new().await.map_err(|error| {
        BindFailure::Failed(
            bindings.clone(),
            format!("GlobalShortcuts Portal 不可用: {error}"),
        )
    })?;
    let session = proxy
        .create_session(CreateSessionOptions::default())
        .await
        .map_err(|error| {
            BindFailure::Failed(
                bindings.clone(),
                format!("创建 GlobalShortcuts 会话失败: {error}"),
            )
        })?;

    if *cancellation.borrow() != generation {
        if let Err(error) = session.close().await {
            log::warn!("取消 Portal 快捷键时关闭新会话失败: {error}");
        }
        return Err(BindFailure::Cancelled);
    }

    let setup = async {
        let activations = proxy
            .receive_activated()
            .await
            .map_err(|error| format!("监听 GlobalShortcuts 激活事件失败: {error}"))?
            .map(|event| event.shortcut_id().to_string());
        let requested = bindings
            .iter()
            .map(|binding| {
                NewShortcut::new(binding.action, binding.description)
                    .preferred_trigger(Some(binding.trigger.as_str()))
            })
            .collect::<Vec<_>>();
        let mut bind = Box::pin(proxy.bind_shortcuts(
            &session,
            &requested,
            None,
            BindShortcutsOptions::default(),
        ));
        let response = tokio::select! {
            result = &mut bind => Some(result),
            _ = cancellation.changed() => None,
        };
        drop(bind);
        let Some(response) = response else {
            return Err(None);
        };
        let response = response
            .map_err(|error| Some(format!("提交 GlobalShortcuts 绑定失败: {error}")))?
            .response()
            .map_err(|error| Some(format!("GlobalShortcuts 绑定被拒绝: {error}")))?;
        let accepted = response
            .shortcuts()
            .iter()
            .map(|shortcut| shortcut.id())
            .collect::<HashSet<_>>();

        let (active_bindings, rejected_bindings) = partition_bindings(&bindings, &accepted);
        for binding in &active_bindings {
            crate::record_register_result(
                app,
                &[binding.action],
                &binding.shortcut,
                DesktopSession::Wayland,
                Ok(()),
            );
        }
        for binding in &rejected_bindings {
            crate::record_register_result(
                app,
                &[binding.action],
                &binding.shortcut,
                DesktopSession::Wayland,
                Err("GlobalShortcuts Portal 未接受该动作".to_string()),
            );
        }
        if active_bindings.is_empty() {
            return Err(Some("GlobalShortcuts Portal 未接受任何快捷键".to_string()));
        }

        Ok((Box::pin(activations), active_bindings))
    }
    .await;

    match setup {
        Ok((activations, active_bindings)) => Ok(ActivePortal {
            _proxy: proxy,
            session,
            activations,
            bindings: active_bindings,
        }),
        Err(reason) => {
            if let Err(error) = session.close().await {
                log::warn!("关闭未完成的 GlobalShortcuts Portal 会话失败: {error}");
            }
            match reason {
                Some(reason) => Err(BindFailure::Failed(bindings, reason)),
                None => Err(BindFailure::Cancelled),
            }
        }
    }
}

fn partition_bindings(
    bindings: &[PortalBinding],
    accepted: &HashSet<&str>,
) -> (Vec<PortalBinding>, Vec<PortalBinding>) {
    bindings
        .iter()
        .cloned()
        .partition(|binding| accepted.contains(binding.action))
}

fn begin_generation(cancellation: &mut watch::Receiver<u64>, generation: u64) -> bool {
    *cancellation.borrow_and_update() == generation
}

async fn close_active(active: &mut Option<ActivePortal>) -> Result<(), String> {
    let Some(portal) = active.take() else {
        return Ok(());
    };
    portal
        .session
        .close()
        .await
        .map_err(|error| format!("关闭 GlobalShortcuts Portal 会话失败: {error}"))
}

fn report_binding_failure(app: &AppHandle, bindings: &[PortalBinding], reason: &str) {
    for binding in bindings {
        crate::record_register_result(
            app,
            &[binding.action],
            &binding.shortcut,
            DesktopSession::Wayland,
            Err(reason.to_string()),
        );
    }
}

/// worker 本身不可用时，调用方仍要把三个动作逐项写进失败状态，不能只留一条日志。
pub(crate) fn report_config_failure(app: &AppHandle, config: &AppConfig, reason: &str) {
    for (action, shortcut) in [
        ("global", config.global_shortcut.as_str()),
        ("pin", config.pin_shortcut.as_str()),
        ("capture", config.capture_shortcut.as_str()),
    ] {
        if shortcut.trim().is_empty() {
            clear_register_failure(app, &[action]);
        } else {
            crate::record_register_result(
                app,
                &[action],
                shortcut,
                DesktopSession::Wayland,
                Err(reason.to_string()),
            );
        }
    }
}

fn plan_portal_registration(config: &AppConfig) -> PortalPlan {
    let configured = [
        (
            "global",
            config.global_shortcut.as_str(),
            "Show or hide Clippy",
        ),
        (
            "pin",
            config.pin_shortcut.as_str(),
            "Pin the current clipboard item",
        ),
        (
            "capture",
            config.capture_shortcut.as_str(),
            "Capture a screen region",
        ),
    ];
    let mut valid = Vec::new();
    let mut invalid = Vec::new();
    let mut unset = Vec::new();
    for (action, shortcut, description) in configured {
        let shortcut = shortcut.trim();
        if shortcut.is_empty() {
            unset.push(action);
            continue;
        }
        match to_portal_trigger(shortcut) {
            Ok(trigger) => valid.push(PortalBinding {
                action,
                shortcut: shortcut.to_string(),
                description,
                trigger,
            }),
            Err(reason) => invalid.push((action, shortcut.to_string(), reason)),
        }
    }
    PortalPlan {
        valid,
        invalid,
        unset,
    }
}

/// Tauri/global-hotkey 格式转 XDG Shortcuts 规范：修饰键固定为 CTRL/ALT/SHIFT/LOGO，
/// 主键使用去掉 `XKB_KEY_` 前缀后的 keysym 名。
fn to_portal_trigger(shortcut: &str) -> Result<String, String> {
    let parts = shortcut
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let Some((key, modifiers)) = parts.split_last() else {
        return Err("Portal 快捷键不能为空".to_string());
    };
    let mut normalized = Vec::new();
    for modifier in modifiers {
        let modifier = match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "cmdorctrl" | "commandorcontrol" => "CTRL",
            "alt" | "option" => "ALT",
            "shift" => "SHIFT",
            "super" | "meta" | "cmd" | "command" | "win" => "LOGO",
            other => return Err(format!("Portal 不支持修饰键 `{other}`")),
        };
        if !normalized.contains(&modifier) {
            normalized.push(modifier);
        }
    }
    if normalized.is_empty() {
        return Err("Portal 快捷键至少需要一个修饰键".to_string());
    }
    let key = portal_keysym(key).ok_or_else(|| format!("Portal 不支持主键 `{key}`"))?;
    normalized.push(key.as_str());
    Ok(normalized.join("+"))
}

fn portal_keysym(key: &str) -> Option<String> {
    let lower = key.to_ascii_lowercase();
    let mapped = match lower.as_str() {
        "space" => "space",
        "tab" => "Tab",
        "enter" => "Return",
        "escape" | "esc" => "Escape",
        "backspace" => "BackSpace",
        "delete" => "Delete",
        "insert" => "Insert",
        "home" => "Home",
        "end" => "End",
        "pageup" => "Page_Up",
        "pagedown" => "Page_Down",
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        "-" => "minus",
        "=" => "equal",
        "[" => "bracketleft",
        "]" => "bracketright",
        "\\" => "backslash",
        ";" => "semicolon",
        "'" => "apostrophe",
        "," => "comma",
        "." => "period",
        "/" => "slash",
        "`" => "grave",
        "numadd" => "KP_Add",
        "numsub" => "KP_Subtract",
        "nummult" => "KP_Multiply",
        "numdiv" => "KP_Divide",
        "numdec" => "KP_Decimal",
        _ if lower.len() == 1
            && lower
                .chars()
                .all(|character| character.is_ascii_alphanumeric()) =>
        {
            return Some(lower)
        }
        _ if lower.starts_with('f')
            && lower[1..]
                .parse::<u8>()
                .is_ok_and(|number| (1..=35).contains(&number)) =>
        {
            return Some(key.to_ascii_uppercase())
        }
        _ if let Some(number) = lower.strip_prefix("num")
            && number.len() == 1
            && number.chars().all(|character| character.is_ascii_digit()) =>
        {
            return Some(format!("KP_{number}"))
        }
        _ => return None,
    };
    Some(mapped.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tauri_shortcuts_become_xdg_triggers() {
        assert_eq!(to_portal_trigger("Ctrl+Shift+S").unwrap(), "CTRL+SHIFT+s");
        assert_eq!(to_portal_trigger("Super+V").unwrap(), "LOGO+v");
        assert_eq!(to_portal_trigger("Alt+Enter").unwrap(), "ALT+Return");
        assert_eq!(to_portal_trigger("Ctrl+NumAdd").unwrap(), "CTRL+KP_Add");
    }

    #[test]
    fn punctuation_uses_xkb_keysym_names() {
        assert_eq!(
            to_portal_trigger("Ctrl+Shift+=").unwrap(),
            "CTRL+SHIFT+equal"
        );
        assert_eq!(to_portal_trigger("Alt+[").unwrap(), "ALT+bracketleft");
    }

    #[test]
    fn unsupported_or_modifier_only_shortcuts_are_rejected_per_action() {
        assert!(to_portal_trigger("Shift").is_err());
        assert!(to_portal_trigger("Hyper+V").is_err());
        assert!(to_portal_trigger("Ctrl+媒体键").is_err());
    }

    #[test]
    fn registration_plan_keeps_valid_actions_when_one_is_invalid() {
        let config = AppConfig {
            global_shortcut: "Super+V".to_string(),
            pin_shortcut: "Hyper+P".to_string(),
            capture_shortcut: String::new(),
            ..AppConfig::default()
        };
        let plan = plan_portal_registration(&config);
        assert_eq!(plan.valid.len(), 1);
        assert_eq!(plan.valid[0].action, "global");
        assert_eq!(plan.invalid.len(), 1);
        assert_eq!(plan.invalid[0].0, "pin");
        assert_eq!(plan.unset, vec!["capture"]);
    }

    #[test]
    fn portal_subset_is_not_mistaken_for_complete_success() {
        let bindings = plan_portal_registration(&AppConfig::default()).valid;
        let accepted = HashSet::from(["global", "capture"]);
        let (active, rejected) = partition_bindings(&bindings, &accepted);
        assert_eq!(
            active
                .iter()
                .map(|binding| binding.action)
                .collect::<Vec<_>>(),
            vec!["global", "capture"]
        );
        assert_eq!(rejected.len(), 1);
        assert_eq!(rejected[0].action, "pin");
    }

    #[test]
    fn current_generation_is_acknowledged_before_waiting_for_cancellation() {
        let (sender, mut receiver) = watch::channel(0);
        sender.send_replace(1);
        assert!(receiver.has_changed().unwrap());
        assert!(begin_generation(&mut receiver, 1));
        assert!(!receiver.has_changed().unwrap());

        sender.send_replace(2);
        assert!(receiver.has_changed().unwrap());
        assert!(!begin_generation(&mut receiver, 1));
    }
}
