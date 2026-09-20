use crate::commands::AppState;
use serde::Serialize;
use tauri::Manager;

const LAUNCHER_LABEL: &str = "launcher";

#[derive(Debug, Serialize)]
pub(crate) struct LauncherError {
    code: &'static str,
}

impl LauncherError {
    fn forbidden() -> Self {
        Self {
            code: "launcher_forbidden",
        }
    }

    fn window() -> Self {
        Self {
            code: "launcher_window_failed",
        }
    }

    fn state() -> Self {
        Self {
            code: "launcher_state_failed",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LauncherSettings {
    theme: String,
    language: String,
    translation_source_language: String,
    translation_target_language: String,
}

/// 托盘和主窗口共用同一个建窗入口。隐藏窗口表示页面仍在首帧加载或正被截图流程临时隐藏，
/// 不能在这时提前显示空白 WebView。
pub(crate) fn open(app: &tauri::AppHandle) -> Result<(), LauncherError> {
    if let Some(window) = app.get_webview_window(LAUNCHER_LABEL) {
        if window.is_visible().unwrap_or(false) {
            window.unminimize().map_err(|_| LauncherError::window())?;
            window.show().map_err(|_| LauncherError::window())?;
            window.set_focus().map_err(|_| LauncherError::window())?;
        }
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(
        app,
        LAUNCHER_LABEL,
        tauri::WebviewUrl::App("launcher.html".into()),
    )
    .title(crate::window_controller::native_text(app).launcher_title)
    .inner_size(640.0, 520.0)
    .min_inner_size(420.0, 360.0)
    .center()
    .resizable(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false)
    .build()
    .map_err(|_| LauncherError::window())?;
    Ok(())
}

fn ensure_launcher(window: &tauri::WebviewWindow) -> Result<(), LauncherError> {
    if window.label() == LAUNCHER_LABEL {
        Ok(())
    } else {
        Err(LauncherError::forbidden())
    }
}

#[tauri::command]
pub(crate) fn show_action_launcher(window: tauri::WebviewWindow) -> Result<(), LauncherError> {
    if window.label() != "main" {
        return Err(LauncherError::forbidden());
    }
    open(window.app_handle())
}

#[tauri::command]
pub(crate) fn get_action_launcher_settings(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<LauncherSettings, LauncherError> {
    ensure_launcher(&window)?;
    let config = state.config.lock().map_err(|_| LauncherError::state())?;
    Ok(LauncherSettings {
        theme: config.theme.clone(),
        language: config.language.clone(),
        translation_source_language: config.translation_source_language.clone(),
        translation_target_language: config.translation_target_language.clone(),
    })
}

#[tauri::command]
pub(crate) fn action_launcher_ready(window: tauri::WebviewWindow) -> Result<(), LauncherError> {
    ensure_launcher(&window)?;
    window.show().map_err(|_| LauncherError::window())?;
    window.set_focus().map_err(|_| LauncherError::window())
}

#[tauri::command]
pub(crate) fn start_action_launcher_drag(
    window: tauri::WebviewWindow,
) -> Result<(), LauncherError> {
    ensure_launcher(&window)?;
    window.start_dragging().map_err(|_| LauncherError::window())
}

#[tauri::command]
pub(crate) fn close_action_launcher(window: tauri::WebviewWindow) -> Result<(), LauncherError> {
    ensure_launcher(&window)?;
    window.destroy().map_err(|_| LauncherError::window())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_settings_serialize_without_provider_credentials() {
        let value = serde_json::to_value(LauncherSettings {
            theme: "dark".to_string(),
            language: "zh-CN".to_string(),
            translation_source_language: "auto".to_string(),
            translation_target_language: "en".to_string(),
        })
        .unwrap();
        assert_eq!(value["translationSourceLanguage"], "auto");
        assert_eq!(value["translationTargetLanguage"], "en");
        assert_eq!(value.as_object().unwrap().len(), 4);
    }
}
