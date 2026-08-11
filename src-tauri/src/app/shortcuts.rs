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
