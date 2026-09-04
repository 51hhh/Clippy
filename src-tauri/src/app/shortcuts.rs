use crate::commands::{self, AppState};
use crate::models::AppConfig;
use crate::platform::DesktopSession;
use crate::window_controller;
use std::collections::HashSet;
use std::str::FromStr;
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShortcutAction {
    ToggleMain,
    PinCurrent,
    Capture,
}

/// 快捷键注册失败的事件负载。
///
/// 三个动作都必须能被界面看到：非 GNOME 的 Wayland 桌面上 gsettings 的
/// media-keys schema 根本不存在，只写日志等于让快捷键静默失效——用户按键没反应，
/// 设置页却显示得好好的。`session` 让前端能给出对应的处置建议。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ShortcutRegisterFailure {
    /// `global` / `pin` / `capture`
    pub action: String,
    pub shortcut: String,
    /// `wayland` / `x11` / `native`
    pub session: DesktopSession,
    /// 失败原因（底层命令的错误文本）
    pub reason: String,
}

impl ShortcutRegisterFailure {
    pub fn new(action: &str, shortcut: &str, session: DesktopSession, reason: String) -> Self {
        Self {
            action: action.to_string(),
            shortcut: shortcut.to_string(),
            session,
            reason,
        }
    }
}

/// 上报快捷键注册失败：日志 + 事件 + 状态。
///
/// 事件覆盖"设置页正开着"的场景；状态覆盖"启动阶段就失败"的场景——那时前端还没监听，
/// 事件会丢，设置页只能靠 `get_shortcut_failures` 主动查。
pub(crate) fn report_register_failure(
    app: &tauri::AppHandle,
    action: &str,
    shortcut: &str,
    session: DesktopSession,
    reason: impl std::fmt::Display,
) {
    let failure = ShortcutRegisterFailure::new(action, shortcut, session, reason.to_string());
    log::warn!(
        "快捷键注册失败[{}] {}: {}",
        failure.action,
        failure.shortcut,
        failure.reason
    );
    if let Some(state) = app.try_state::<AppState>() {
        match state.shortcut_failures.lock() {
            Ok(mut failures) => {
                failures.retain(|entry| entry.action != failure.action);
                failures.push(failure.clone());
            }
            Err(error) => log::warn!("记录快捷键失败状态失败: {error}"),
        }
    }
    let _ = app.emit("shortcut-register-failed", failure);
}

/// 某个动作重新注册成功：清掉它的失败记录，否则设置页会一直挂着过期的红字。
pub(crate) fn clear_register_failure(app: &tauri::AppHandle, actions: &[&str]) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    match state.shortcut_failures.lock() {
        Ok(mut failures) => failures.retain(|entry| !actions.contains(&entry.action.as_str())),
        Err(error) => log::warn!("清理快捷键失败状态失败: {error}"),
    };
}

/// 按注册结果记账：成功清记录，失败上报。调用点只需给出这次动作涉及的名字。
pub(crate) fn record_register_result(
    app: &tauri::AppHandle,
    actions: &[&str],
    shortcut: &str,
    session: DesktopSession,
    result: Result<(), String>,
) {
    match result {
        Ok(()) => clear_register_failure(app, actions),
        Err(reason) => report_register_failure(app, actions[0], shortcut, session, reason),
    }
}

/// 已有实例运行时聚焦主窗口。
pub(crate) fn on_second_instance(app: &tauri::AppHandle, _args: Vec<String>, _cwd: String) {
    if app.get_webview_window("main").is_some() {
        if let Some(state) = app.try_state::<AppState>() {
            state.paste_manager.capture_target();
        }
        let _ = window_controller::show_main_window(app);
    }
}

pub(crate) fn handle_registered(app: &tauri::AppHandle, shortcut: &Shortcut) {
    let Some(state) = app.try_state::<AppState>() else {
        log::warn!("快捷键触发时 AppState 未就绪");
        return;
    };
    let action = {
        let Ok(config) = state.config.lock() else {
            log::warn!("快捷键触发时无法读取配置");
            return;
        };
        shortcut_action(&config, shortcut)
    };

    match action {
        Some(ShortcutAction::ToggleMain) => toggle_main_window(app),
        Some(ShortcutAction::PinCurrent) => {
            let _ = app.emit("pin-current", ());
        }
        Some(ShortcutAction::Capture) => trigger_capture(app),
        None => log::warn!("收到未配置的快捷键事件: {}", shortcut),
    }
}

/// Tauri 原生后端逐个动作注册并按动作记账。
///
/// 不用 `register_multiple`：它是全有或全无，一个键位被别的程序抓走就整批失败，
/// 而且失败信息里没有"是哪个动作"，只能笼统归给 `global`。逐个注册既能把失败归到
/// 具体动作，也能让没冲突的另外两个动作照常工作。
///
/// 返回 `Err` 只表示"一个键位都没注册上"（`unregister_all` 失败，或所有配置的动作都失败），
/// 调用方据此决定是否把状态退回"已暂停"；部分成功返回 `Ok`，失败细节在失败记录里。
pub(crate) fn register_tauri_shortcuts(
    handle: &tauri::AppHandle,
    config: &AppConfig,
) -> Result<(), String> {
    let global_shortcuts = handle.global_shortcut();
    global_shortcuts
        .unregister_all()
        .map_err(|e| e.to_string())?;

    let mut attempted = 0usize;
    let mut failed = 0usize;
    let mut first_error: Option<String> = None;
    for (action, raw, plan) in plan_tauri_registration(config) {
        let result = match plan {
            TauriRegistration::Unset | TauriRegistration::Shared => Ok(()),
            TauriRegistration::Invalid(reason) => Err(reason),
            TauriRegistration::Register(shortcut) => global_shortcuts
                .register(shortcut)
                .map_err(|e| e.to_string()),
        };
        if !raw.is_empty() {
            attempted += 1;
            if let Err(reason) = &result {
                failed += 1;
                first_error.get_or_insert_with(|| reason.clone());
            }
        }
        record_register_result(
            handle,
            &[action],
            &raw,
            crate::platform::current_session(),
            result,
        );
    }

    match first_error {
        // 全部失败：一个键位都没生效，让调用方能把状态退回暂停态并重试
        Some(reason) if failed == attempted => Err(reason),
        Some(reason) => {
            log::warn!("Tauri 全局快捷键部分注册失败: {reason}");
            Ok(())
        }
        None => {
            log::info!("Tauri 全局快捷键注册完成");
            Ok(())
        }
    }
}

/// 一个动作的 Tauri 注册计划。
#[derive(Debug)]
enum TauriRegistration {
    /// 未配置键位
    Unset,
    /// 与前一个动作共用同一键位，插件侧只需注册一次
    Shared,
    /// 需要向插件注册
    Register(Shortcut),
    /// 键位字符串无法解析
    Invalid(String),
}

fn action_shortcuts(config: &AppConfig) -> [(&'static str, &str); 3] {
    [
        ("global", config.global_shortcut.as_str()),
        ("pin", config.pin_shortcut.as_str()),
        ("capture", config.capture_shortcut.as_str()),
    ]
}

/// 纯计算：把配置里的三个键位映射成注册计划（去重、空值与解析失败都在这里定型）
fn plan_tauri_registration(config: &AppConfig) -> Vec<(&'static str, String, TauriRegistration)> {
    let mut ids = HashSet::new();
    action_shortcuts(config)
        .into_iter()
        .map(|(action, raw)| {
            let raw = raw.trim().to_string();
            let plan = if raw.is_empty() {
                TauriRegistration::Unset
            } else {
                match Shortcut::from_str(&raw) {
                    Err(error) => {
                        TauriRegistration::Invalid(format!("快捷键 `{raw}` 解析失败: {error}"))
                    }
                    Ok(shortcut) => {
                        if ids.insert(shortcut.id()) {
                            TauriRegistration::Register(shortcut)
                        } else {
                            TauriRegistration::Shared
                        }
                    }
                }
            };
            (action, raw, plan)
        })
        .collect()
}

pub(crate) fn toggle_main_window(handle: &tauri::AppHandle) {
    log::info!("全局快捷键触发 toggle_main_window");
    if let Some(window) = handle.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window_controller::hide_main_window(handle);
        } else {
            if let Some(state) = handle.try_state::<AppState>() {
                state.paste_manager.capture_target();
            }
            if let Err(error) = window_controller::show_main_window(handle) {
                log::warn!("显示主窗口失败: {error}");
            }
        }
    } else {
        log::warn!("找不到 main 窗口");
    }
}

fn trigger_capture(handle: &tauri::AppHandle) {
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = commands::trigger_capture_overlay(handle).await {
            log::warn!("截图快捷键触发失败: {}", error);
        }
    });
}

/// GlobalShortcuts Portal 使用稳定动作 ID，而不是平台相关的物理键码。
#[cfg(target_os = "linux")]
pub(crate) fn handle_portal_action(app: &tauri::AppHandle, shortcut_id: &str) {
    match shortcut_id {
        "global" => toggle_main_window(app),
        "pin" => {
            let _ = app.emit("pin-current", ());
        }
        "capture" => trigger_capture(app),
        other => log::warn!("收到未知 Portal 快捷键动作: {other}"),
    }
}

fn shortcut_matches(pressed: &Shortcut, configured: &str) -> bool {
    let configured = configured.trim();
    if configured.is_empty() {
        return false;
    }
    Shortcut::from_str(configured)
        .map(|shortcut| shortcut.id() == pressed.id())
        .unwrap_or(false)
}

fn shortcut_action(config: &AppConfig, pressed: &Shortcut) -> Option<ShortcutAction> {
    if shortcut_matches(pressed, &config.global_shortcut) {
        Some(ShortcutAction::ToggleMain)
    } else if shortcut_matches(pressed, &config.pin_shortcut) {
        Some(ShortcutAction::PinCurrent)
    } else if shortcut_matches(pressed, &config.capture_shortcut) {
        Some(ShortcutAction::Capture)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_kinds(config: &AppConfig) -> Vec<(&'static str, String)> {
        plan_tauri_registration(config)
            .into_iter()
            .map(|(action, _, plan)| {
                let kind = match plan {
                    TauriRegistration::Unset => "unset".to_string(),
                    TauriRegistration::Shared => "shared".to_string(),
                    TauriRegistration::Register(_) => "register".to_string(),
                    TauriRegistration::Invalid(reason) => format!("invalid: {reason}"),
                };
                (action, kind)
            })
            .collect()
    }

    #[test]
    fn plan_deduplicates_and_skips_empty_values() {
        let config = AppConfig {
            global_shortcut: "Alt+V".to_string(),
            pin_shortcut: "Alt+V".to_string(),
            capture_shortcut: String::new(),
            ..AppConfig::default()
        };

        assert_eq!(
            plan_kinds(&config),
            vec![
                ("global", "register".to_string()),
                ("pin", "shared".to_string()),
                ("capture", "unset".to_string()),
            ]
        );
    }

    #[test]
    fn plan_isolates_an_unparsable_shortcut() {
        // 一个动作的键位写坏了，另外两个仍要照常注册（旧的 register_multiple 是全有或全无）
        let config = AppConfig {
            global_shortcut: "Alt+V".to_string(),
            pin_shortcut: "NotAKey+".to_string(),
            capture_shortcut: "Ctrl+Shift+A".to_string(),
            ..AppConfig::default()
        };

        let kinds = plan_kinds(&config);
        assert_eq!(kinds[0].1, "register");
        assert!(kinds[1]
            .1
            .starts_with("invalid: 快捷键 `NotAKey+` 解析失败"));
        assert_eq!(kinds[2].1, "register");
    }

    #[test]
    fn plan_covers_every_action_exactly_once() {
        let actions: Vec<&str> = plan_tauri_registration(&AppConfig::default())
            .into_iter()
            .map(|(action, _, _)| action)
            .collect();
        assert_eq!(actions, vec!["global", "pin", "capture"]);
    }

    #[test]
    fn shortcut_action_uses_configured_priority() {
        let config = AppConfig::default();
        let pressed = Shortcut::from_str(&config.capture_shortcut).unwrap();
        assert_eq!(
            shortcut_action(&config, &pressed),
            Some(ShortcutAction::Capture)
        );
    }

    #[test]
    fn native_registration_failure_keeps_its_platform_session() {
        let failure = ShortcutRegisterFailure::new(
            "capture",
            "Ctrl+Shift+A",
            DesktopSession::Native,
            "reserved".to_string(),
        );
        let json = serde_json::to_value(failure).unwrap();
        assert_eq!(json["session"], "native");
    }
}
