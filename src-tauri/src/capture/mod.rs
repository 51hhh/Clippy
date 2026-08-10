mod manager;
mod overlay_windows;
mod types;
mod window_probe;

pub use manager::CaptureManager;
pub use types::{
    CaptureAction, CaptureActionResult, CaptureOverlayPayload, CaptureSelection,
    CaptureTranslationResult,
};

use crate::commands::AppState;
use crate::translation::types::TranslationError;
use base64::{engine::general_purpose::STANDARD, Engine};
use tauri::State;

pub(crate) fn handle_overlay_destroyed(
    app_handle: &tauri::AppHandle,
    state: &AppState,
    label: &str,
) {
    if let Some(session) = state.capture_manager.abort_if_overlay(label) {
        overlay_windows::close(app_handle, &session.overlay_labels);
        overlay_windows::restore(app_handle, &session.restore_labels);
    }
}

#[tauri::command]
pub async fn show_capture_overlay(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    show_capture_overlay_for_app(app_handle, &state).await
}

pub(crate) async fn show_capture_overlay_for_app(
    app_handle: tauri::AppHandle,
    state: &AppState,
) -> Result<(), String> {
    let restore_labels = overlay_windows::hide_sources(&app_handle);
    let frames = match tauri::async_runtime::spawn_blocking(|| {
        std::thread::sleep(std::time::Duration::from_millis(140));
        // 截图覆盖层只能由快捷键/显式命令触发，允许 Portal 在必要时询问用户。
        crate::screenshot::capture_monitor_frames(true)
    })
    .await
    {
        Ok(Ok(frames)) => frames,
        Ok(Err(error)) => {
            overlay_windows::restore(&app_handle, &restore_labels);
            return Err(format!("截图失败: {error}"));
        }
        Err(error) => {
            overlay_windows::restore(&app_handle, &restore_labels);
            return Err(format!("截图线程异常: {error}"));
        }
    };
    let specs = match state.capture_manager.begin(frames, restore_labels.clone()) {
        Ok(specs) => specs,
        Err(error) => {
            overlay_windows::restore(&app_handle, &restore_labels);
            return Err(error);
        }
    };
    if let Err(error) = overlay_windows::create(&app_handle, &specs) {
        if let Some(session) = state.capture_manager.abort() {
            overlay_windows::close(&app_handle, &session.overlay_labels);
        }
        overlay_windows::restore(&app_handle, &restore_labels);
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
pub fn get_capture_overlay(
    label: String,
    state: State<'_, AppState>,
) -> Result<CaptureOverlayPayload, String> {
    validate_overlay_label(&label)?;
    state.capture_manager.payload(&label)
}

#[tauri::command]
pub fn cancel_capture_overlay(
    session_id: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let session = state.capture_manager.finish(&session_id)?;
    overlay_windows::close(&app_handle, &session.overlay_labels);
    overlay_windows::restore(&app_handle, &session.restore_labels);
    Ok(())
}

#[tauri::command]
pub fn run_capture_action(
    action: CaptureAction,
    selection: CaptureSelection,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<CaptureActionResult, String> {
    let png = state.capture_manager.crop(&selection)?;
    let restore_sources = !matches!(action, CaptureAction::Edit);
    let result = execute_action(action, png, &app_handle, &state)?;
    let session = state.capture_manager.finish(&selection.session_id)?;
    overlay_windows::close(&app_handle, &session.overlay_labels);
    if restore_sources {
        // 高级编辑器已经接管焦点；其余动作应回到截图前的源窗口。
        overlay_windows::restore(&app_handle, &session.restore_labels);
    }
    Ok(result)
}

/// 显式执行“选区 -> 本地 OCR -> 文本翻译”。裁剪帧只进入 Tesseract，永不发送给 provider。
/// 此命令不会结束 CaptureSession，用户可继续复制、保存、Pin 或编辑同一选区。
#[tauri::command]
pub async fn translate_capture_selection(
    selection: CaptureSelection,
    source_language: Option<String>,
    target_language: Option<String>,
    request_id: Option<u64>,
    state: State<'_, AppState>,
) -> Result<CaptureTranslationResult, String> {
    let request_id =
        crate::translation::commands::reserve_request_id(&state.translation, request_id);
    let capture_manager = state.capture_manager.clone();
    let png = run_translation_blocking(move || {
        capture_manager
            .crop(&selection)
            .map_err(|_| TranslationError::CaptureUnavailable)
    })
    .await
    .map_err(translation_ipc_error)?;

    let source_text = run_translation_blocking(move || recognize_capture_text(&png))
        .await
        .map_err(translation_ipc_error)?;
    let translated = crate::translation::commands::translate_configured_text(
        state.translation.clone(),
        state.config.clone(),
        source_text.clone(),
        source_language,
        target_language,
        request_id,
    )
    .await
    .map_err(translation_ipc_error)?;

    Ok(CaptureTranslationResult::from_translation(
        source_text,
        translated,
    ))
}

async fn run_translation_blocking<T, F>(task: F) -> Result<T, TranslationError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, TranslationError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|_| TranslationError::Internal)?
}

fn recognize_capture_text(png: &[u8]) -> Result<String, TranslationError> {
    normalize_ocr_text(crate::ocr::recognize(png))
}

fn normalize_ocr_text(result: Result<String, String>) -> Result<String, TranslationError> {
    let text = result.map_err(|_| TranslationError::OcrFailed)?;
    if text.trim().is_empty() {
        Err(TranslationError::OcrFailed)
    } else {
        Ok(text)
    }
}

fn translation_ipc_error(error: TranslationError) -> String {
    error.ipc_message()
}

fn execute_action(
    action: CaptureAction,
    png: Vec<u8>,
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<CaptureActionResult, String> {
    match action {
        CaptureAction::Copy => {
            crate::image_io::copy_png_to_clipboard(&png)?;
            Ok(action_result("copy", None, None))
        }
        CaptureAction::Save => {
            let path = crate::image_io::save_png(&png, "clippy-screenshot")?;
            Ok(action_result(
                "save",
                Some(path.to_string_lossy().to_string()),
                None,
            ))
        }
        CaptureAction::Pin => {
            let label = crate::pin::create_screenshot_pin(png, app_handle, state)?;
            Ok(action_result("pin", None, Some(label)))
        }
        CaptureAction::Edit => {
            let (width, height) =
                crate::screenshot::png_dimensions(&png).map_err(|error| error.to_string())?;
            *state
                .latest_capture
                .lock()
                .map_err(|error| error.to_string())? =
                Some(crate::screenshot::CapturedScreenshot {
                    png_base64: STANDARD.encode(png),
                    width,
                    height,
                });
            crate::commands::open_capture_window(app_handle)?;
            Ok(action_result("edit", None, None))
        }
    }
}

fn action_result(
    action: &'static str,
    path: Option<String>,
    pin_label: Option<String>,
) -> CaptureActionResult {
    CaptureActionResult {
        action,
        path,
        pin_label,
    }
}

fn validate_overlay_label(label: &str) -> Result<(), String> {
    if label.starts_with("capture-overlay-")
        && label.len() <= 128
        && label
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        Ok(())
    } else {
        Err("无效的截图覆盖层标签".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_ocr_rejects_failures_and_blank_output_with_safe_error() {
        for result in [
            Err("private tesseract detail".to_string()),
            Ok("  \n".to_string()),
        ] {
            let error = normalize_ocr_text(result).unwrap_err();
            assert_eq!(error, TranslationError::OcrFailed);
            assert_eq!(
                error.ipc_message(),
                "translation.ocr_failed: Local OCR could not extract text from the image"
            );
        }
    }

    #[test]
    fn capture_ocr_preserves_recognized_text_for_translation() {
        assert_eq!(
            normalize_ocr_text(Ok("local OCR text".to_string())),
            Ok("local OCR text".to_string())
        );
    }
}
