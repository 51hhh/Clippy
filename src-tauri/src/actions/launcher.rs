use crate::commands::AppState;
use crate::storage::BoundedImageData;
use serde::Serialize;
use tauri::Manager;

const LAUNCHER_LABEL: &str = "launcher";

#[derive(Debug, Serialize)]
pub(crate) struct LauncherError {
    code: &'static str,
}

impl LauncherError {
    fn new(code: &'static str) -> Self {
        Self { code }
    }

    fn forbidden() -> Self {
        Self::new("launcher_forbidden")
    }

    fn window() -> Self {
        Self::new("launcher_window_failed")
    }

    fn state() -> Self {
        Self::new("launcher_state_failed")
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LauncherImageSource {
    reference: super::OwnedImageReference,
    width: u32,
    height: u32,
    byte_length: usize,
    sensitive: bool,
}

/// 托盘和主窗口共用同一个建窗入口。隐藏窗口表示页面仍在首帧加载或正被截图流程临时隐藏，
/// 不能在这时提前显示空白 WebView。
pub(crate) fn open(
    app: &tauri::AppHandle,
    state: &AppState,
    clip_id: Option<i64>,
) -> Result<(), LauncherError> {
    if let Some(window) = app.get_webview_window(LAUNCHER_LABEL) {
        if window.is_visible().unwrap_or(false) {
            window.unminimize().map_err(|_| LauncherError::window())?;
            window.show().map_err(|_| LauncherError::window())?;
            window.set_focus().map_err(|_| LauncherError::window())?;
        }
        return Ok(());
    }

    let selection = match clip_id {
        None => super::LauncherImageSelection::Latest,
        Some(0) => super::LauncherImageSelection::None,
        Some(id) if id > 0 => super::LauncherImageSelection::Clip(id),
        Some(_) => return Err(LauncherError::new("launcher_invalid_context")),
    };
    state.action_runtime.set_launcher_image_selection(selection);

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
pub(crate) fn show_action_launcher(
    clip_id: Option<i64>,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<(), LauncherError> {
    if window.label() != "main" {
        return Err(LauncherError::forbidden());
    }
    open(window.app_handle(), &state, clip_id)
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

/// 读取主窗口选中的历史图片（托盘入口则取最新图片），校验后冻结为 Launcher 独占来源。
/// 返回值只有不透明引用和展示元数据，PNG、内容哈希与数据库身份不进入 WebView。
#[tauri::command]
pub(crate) async fn get_action_launcher_image_source(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<Option<LauncherImageSource>, LauncherError> {
    ensure_launcher(&window)?;
    let selection = state
        .action_runtime
        .launcher_image_selection()
        .map_err(|_| LauncherError::state())?;
    if selection == super::LauncherImageSelection::None {
        return Ok(None);
    }
    let caller_epoch = state
        .action_runtime
        .caller_epoch(LAUNCHER_LABEL)
        .map_err(|_| LauncherError::state())?;
    let storage = state.storage.clone();
    let loaded = tauri::async_runtime::spawn_blocking(move || {
        let storage = storage.lock().map_err(|_| LauncherError::state())?;
        let clip_id = match selection {
            super::LauncherImageSelection::Latest => storage
                .get_latest_image_id()
                .map_err(|_| LauncherError::new("launcher_image_source_failed"))?,
            super::LauncherImageSelection::Clip(id) => Some(id),
            super::LauncherImageSelection::None => None,
        };
        let Some(clip_id) = clip_id else {
            return Ok(None);
        };
        let snapshot = storage
            .get_bounded_image_snapshot(clip_id, crate::viewer::MAX_PNG_BYTES)
            .map_err(|_| LauncherError::new("launcher_image_source_failed"))?;
        let Some(snapshot) = snapshot else {
            return Ok(None);
        };
        let bytes = match snapshot.image {
            BoundedImageData::Bytes(bytes) => bytes,
            BoundedImageData::ClipNotFound | BoundedImageData::NotImage => return Ok(None),
            BoundedImageData::TooLarge => {
                return Err(LauncherError::new("launcher_image_too_large"));
            }
            BoundedImageData::Missing => {
                return Err(LauncherError::new("launcher_image_source_failed"));
            }
        };
        let (width, height) = crate::screenshot::png_dimensions(&bytes)
            .map_err(|_| LauncherError::new("launcher_image_invalid"))?;
        crate::viewer::validate_dimensions(width, height)
            .map_err(|_| LauncherError::new("launcher_image_too_large"))?;
        crate::pin::output::decode_source(&bytes)
            .map_err(|_| LauncherError::new("launcher_image_invalid"))?;
        Ok(Some((
            bytes,
            width,
            height,
            snapshot.is_sensitive,
            snapshot.content_hash,
        )))
    })
    .await
    .map_err(|_| LauncherError::new("launcher_image_worker_failed"))??;
    let Some((bytes, width, height, sensitive, content_hash)) = loaded else {
        return Ok(None);
    };
    let byte_length = bytes.len();
    let reference = state
        .action_runtime
        .register_owned_image_at_epoch(
            LAUNCHER_LABEL,
            caller_epoch,
            bytes,
            sensitive,
            Some(content_hash),
        )
        .map_err(|error| LauncherError::new(error.code()))?;
    Ok(Some(LauncherImageSource {
        reference,
        width,
        height,
        byte_length,
        sensitive,
    }))
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

    #[test]
    fn launcher_image_source_exposes_only_opaque_reference_and_display_metadata() {
        let value = serde_json::to_value(LauncherImageSource {
            reference: super::super::OwnedImageReference {
                source_id: "action-image-7".to_string(),
                source_version: 0,
            },
            width: 640,
            height: 480,
            byte_length: 1024,
            sensitive: true,
        })
        .unwrap();
        assert_eq!(value["reference"]["sourceId"], "action-image-7");
        assert_eq!(value["width"], 640);
        assert_eq!(value.as_object().unwrap().len(), 5);
        assert!(value.get("clipId").is_none());
        assert!(value.get("contentHash").is_none());
        assert!(value.get("png").is_none());
    }
}
