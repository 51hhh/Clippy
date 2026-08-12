use super::AppState;
use crate::config::save_config;
use crate::models::AppConfig;
use std::sync::atomic::Ordering;
use tauri::{Emitter, Manager, State};

const WINDOW_WIDTH_DEFAULT: f64 = 380.0;
const WINDOW_WIDTH_PANEL: f64 = 400.0;
const WINDOW_HEIGHT: f64 = 500.0;

fn calc_window_width(state: &State<AppState>) -> f64 {
    let preview_visible = state.preview_visible.lock().map(|v| *v).unwrap_or(false);
    let codec_visible = state.codec_visible.lock().map(|v| *v).unwrap_or(false);
    WINDOW_WIDTH_DEFAULT
        + if preview_visible {
            WINDOW_WIDTH_PANEL
        } else {
            0.0
        }
        + if codec_visible {
            WINDOW_WIDTH_PANEL
        } else {
            0.0
        }
}

/// 切换预览面板并调整主窗口宽度。
#[tauri::command]
pub fn set_preview_visible(
    visible: bool,
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    if let Ok(mut preview_visible) = state.preview_visible.lock() {
        *preview_visible = visible;
    }
    let width = calc_window_width(&state);
    crate::window_controller::resize_main_window(&app_handle, width, WINDOW_HEIGHT)?;
    Ok(())
}

/// 切换编解码面板并调整主窗口宽度。
#[tauri::command]
pub fn set_codec_visible(
    visible: bool,
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    if let Ok(mut codec_visible) = state.codec_visible.lock() {
        *codec_visible = visible;
    }
    let width = calc_window_width(&state);
    crate::window_controller::resize_main_window(&app_handle, width, WINDOW_HEIGHT)?;
    Ok(())
}

/// 读取当前应用配置。
#[tauri::command]
pub fn get_config(state: State<AppState>) -> Result<AppConfig, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    Ok(config.clone())
}

/// 更新应用配置并持久化到磁盘。
#[tauri::command]
pub fn update_config(
    new_config: AppConfig,
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    let global_changed = config.global_shortcut != new_config.global_shortcut;
    let pin_changed = config.pin_shortcut != new_config.pin_shortcut;
    let capture_changed = config.capture_shortcut != new_config.capture_shortcut;
    *config = new_config;
    save_config(&state.config_path, &config);
    let _ = app_handle.emit("config-changed", &*config);

    if global_changed || pin_changed || capture_changed {
        if crate::gsettings_shortcuts::is_wayland() {
            if global_changed {
                if let Err(e) = crate::gsettings_shortcuts::update_binding(&config.global_shortcut)
                {
                    log::warn!("更新主窗口快捷键失败: {}", e);
                }
            }
            if pin_changed {
                if let Err(e) = crate::gsettings_shortcuts::update_pin_binding(&config.pin_shortcut)
                {
                    log::warn!("更新 Pin 快捷键失败: {}", e);
                }
            }
            if capture_changed {
                if let Err(e) =
                    crate::gsettings_shortcuts::update_capture_binding(&config.capture_shortcut)
                {
                    log::warn!("更新截图快捷键失败: {}", e);
                }
            }
        } else if let Err(e) = crate::register_x11_shortcuts(&app_handle, &config) {
            log::warn!("更新 X11 快捷键失败: {}", e);
        }
    }
    Ok(())
}

/// 动态更新全局快捷键并持久化配置。
#[tauri::command]
pub fn update_shortcut(
    new_shortcut: String,
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    log::info!("更新全局快捷键: {}", new_shortcut);

    if crate::gsettings_shortcuts::is_wayland() {
        crate::gsettings_shortcuts::update_binding(&new_shortcut)?;
    } else {
        let mut next = {
            let config = state.config.lock().map_err(|e| e.to_string())?;
            config.clone()
        };
        next.global_shortcut = new_shortcut.clone();
        crate::register_x11_shortcuts(&app_handle, &next)?;
        log::info!("快捷键注册成功: {}", new_shortcut);
    }

    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    config.global_shortcut = new_shortcut;
    save_config(&state.config_path, &config);
    Ok(())
}

/// 检查指定快捷键是否已被注册。
#[tauri::command]
pub fn check_shortcut_conflict(
    shortcut: String,
    app_handle: tauri::AppHandle,
) -> Result<bool, String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    Ok(app_handle
        .global_shortcut()
        .is_registered(shortcut.as_str()))
}

/// 打开设置窗口，并确保载入最新页面状态。
#[tauri::command]
pub fn show_settings(app_handle: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("settings") {
        let _ = window.close();
    }
    tauri::WebviewWindowBuilder::new(
        &app_handle,
        "settings",
        tauri::WebviewUrl::App("settings.html".into()),
    )
    .title("Clippy Settings")
    .inner_size(720.0, 560.0)
    .min_inner_size(480.0, 400.0)
    .center()
    .resizable(true)
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 暂停全局快捷键，供快捷键录制使用。
#[tauri::command]
pub fn pause_shortcuts(app_handle: tauri::AppHandle, state: State<AppState>) -> Result<(), String> {
    {
        let _transition = state
            .shortcut_transition
            .lock()
            .map_err(|e| e.to_string())?;
        if !state.shortcuts_paused.load(Ordering::Acquire) {
            pause_shortcuts_for_platform(&app_handle)?;
            state.shortcuts_paused.store(true, Ordering::Release);
        }
    }

    // 原生关闭可能早于 pause command 完成；窗口已销毁时在这里补偿恢复。
    if app_handle.get_webview_window("settings").is_none() {
        resume_shortcuts_for_app(&app_handle, &state)?;
    }
    Ok(())
}

fn pause_shortcuts_for_platform(app_handle: &tauri::AppHandle) -> Result<(), String> {
    if crate::gsettings_shortcuts::is_wayland() {
        crate::gsettings_shortcuts::pause().map_err(|e| e.to_string())
    } else {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        app_handle
            .global_shortcut()
            .unregister_all()
            .map_err(|e| e.to_string())
    }
}

/// 幂等恢复全局快捷键，供 IPC 与设置窗口销毁兜底共用。
pub(crate) fn resume_shortcuts_for_app(
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<(), String> {
    let _transition = state
        .shortcut_transition
        .lock()
        .map_err(|e| e.to_string())?;
    if !claim_shortcuts_resume(&state.shortcuts_paused) {
        return Ok(());
    }

    let config = {
        match state.config.lock() {
            Ok(config) => config.clone(),
            Err(error) => {
                state.shortcuts_paused.store(true, Ordering::Release);
                return Err(error.to_string());
            }
        }
    };

    let result = if crate::gsettings_shortcuts::is_wayland() {
        crate::gsettings_shortcuts::resume(
            &config.global_shortcut,
            &config.pin_shortcut,
            &config.capture_shortcut,
        )
    } else {
        crate::register_x11_shortcuts(app_handle, &config)
    };
    if result.is_err() {
        state.shortcuts_paused.store(true, Ordering::Release);
    }
    result
}

fn claim_shortcuts_resume(shortcuts_paused: &std::sync::atomic::AtomicBool) -> bool {
    shortcuts_paused.swap(false, Ordering::AcqRel)
}

/// 恢复全局快捷键。
#[tauri::command]
pub fn resume_shortcuts(
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    resume_shortcuts_for_app(&app_handle, &state)
}

/// 检测安装类型：AppImage 支持自动更新，deb/dev 不支持。
#[tauri::command]
pub fn get_install_type() -> String {
    if std::env::var("APPIMAGE").is_ok() {
        "appimage".into()
    } else {
        "deb".into()
    }
}

/// 当前可执行文件是否位于 cargo target 产物目录。
#[tauri::command]
pub fn is_dev_binary() -> bool {
    match std::env::current_exe() {
        Ok(path) => {
            let path = path.to_string_lossy();
            path.contains("/target/debug/") || path.contains("/target/release/")
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::claim_shortcuts_resume;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn resume_claim_is_idempotent() {
        let paused = AtomicBool::new(true);

        assert!(claim_shortcuts_resume(&paused));
        assert!(!claim_shortcuts_resume(&paused));
        assert!(!paused.load(Ordering::Acquire));
    }
}
