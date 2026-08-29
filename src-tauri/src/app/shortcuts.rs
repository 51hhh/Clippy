use crate::commands::{self, AppState};
use crate::models::AppConfig;
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
    /// `wayland` / `x11`
    pub session: String,
    /// 失败原因（底层命令的错误文本）
    pub reason: String,
}

impl ShortcutRegisterFailure {
    pub fn new(action: &str, shortcut: &str, wayland: bool, reason: String) -> Self {
        Self {
            action: action.to_string(),
            shortcut: shortcut.to_string(),
            session: if wayland { "wayland" } else { "x11" }.to_string(),
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
    wayland: bool,
    reason: impl std::fmt::Display,
) {
    let failure = ShortcutRegisterFailure::new(action, shortcut, wayland, reason.to_string());
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
    wayland: bool,
    result: Result<(), String>,
) {
    match result {
        Ok(()) => clear_register_failure(app, actions),
        Err(reason) => report_register_failure(app, actions[0], shortcut, wayland, reason),
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

pub(crate) fn register_x11_shortcuts(
    handle: &tauri::AppHandle,
    config: &AppConfig,
) -> Result<(), String> {
    let shortcuts = configured_shortcuts(config)?;
    let global_shortcuts = handle.global_shortcut();
    global_shortcuts
        .unregister_all()
        .map_err(|e| e.to_string())?;
    if !shortcuts.is_empty() {
        global_shortcuts
            .register_multiple(shortcuts)
            .map_err(|e| e.to_string())?;
    }
    log::info!("X11 快捷键注册完成");
    Ok(())
}

pub(crate) fn toggle_main_window(handle: &tauri::AppHandle) {
    log::info!("全局快捷键触发 toggle_main_window");
    if let Some(window) = handle.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
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
        if let Err(error) = commands::show_capture_editor_for_app(handle).await {
            log::warn!("截图快捷键触发失败: {}", error);
        }
    });
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

fn configured_shortcuts(config: &AppConfig) -> Result<Vec<Shortcut>, String> {
    let mut ids = HashSet::new();
    let mut shortcuts = Vec::new();
    for raw in [
        config.global_shortcut.as_str(),
        config.pin_shortcut.as_str(),
        config.capture_shortcut.as_str(),
    ] {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let shortcut =
            Shortcut::from_str(raw).map_err(|error| format!("快捷键 `{raw}` 解析失败: {error}"))?;
        if ids.insert(shortcut.id()) {
            shortcuts.push(shortcut);
        }
    }
    Ok(shortcuts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_shortcuts_deduplicate_and_skip_empty_values() {
        let config = AppConfig {
            global_shortcut: "Alt+V".to_string(),
            pin_shortcut: "Alt+V".to_string(),
            capture_shortcut: String::new(),
            ..AppConfig::default()
        };

        let shortcuts = configured_shortcuts(&config).unwrap();
        assert_eq!(shortcuts.len(), 1);
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
}
