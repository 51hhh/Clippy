mod action_lifecycle;
mod error;
mod manager;
mod overlay_windows;
mod types;
mod window_probe;

pub use error::CaptureError;
pub use manager::CaptureManager;
pub use types::{
    CaptureAction, CaptureActionResult, CaptureOverlayPayload, CaptureSelection,
    CaptureTranslationResult,
};

use crate::commands::AppState;
use crate::translation::types::TranslationError;
use tauri::State;

/// 覆盖层提交回来的 PNG 上限。全屏 4K 的标注 PNG 大约十几 MB，
/// 64 MiB 足够宽松，同时挡住畸形/恶意载荷把内存吃光。
const MAX_COMMIT_PNG_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn handle_overlay_destroyed(
    app_handle: &tauri::AppHandle,
    state: &AppState,
    label: &str,
) {
    if let Some(session) = state.capture_manager.abort_if_overlay(label) {
        overlay_windows::close(app_handle, &session.overlay_labels());
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
            return Err(capture_failure(CaptureError::Screenshot(error.to_string())));
        }
        Err(error) => {
            overlay_windows::restore(&app_handle, &restore_labels);
            return Err(capture_failure(CaptureError::ThreadPanic(
                error.to_string(),
            )));
        }
    };
    let specs = match state.capture_manager.begin(frames, restore_labels.clone()) {
        Ok(specs) => specs,
        Err(error) => {
            overlay_windows::restore(&app_handle, &restore_labels);
            return Err(capture_failure(error));
        }
    };
    if let Err(error) = overlay_windows::create(&app_handle, &specs) {
        if let Some(session) = state.capture_manager.abort() {
            overlay_windows::close(&app_handle, &session.overlay_labels());
        }
        overlay_windows::restore(&app_handle, &restore_labels);
        return Err(capture_failure(error));
    }
    Ok(())
}

#[tauri::command]
pub fn get_capture_overlay(
    label: String,
    state: State<'_, AppState>,
) -> Result<CaptureOverlayPayload, String> {
    validate_overlay_label(&label)?;
    Ok(state.capture_manager.payload(&label)?)
}

/// 覆盖层画完首帧后调用，后端这才把窗口显示出来。
///
/// 之前是建窗就 `show()`，于是 webview 加载 + 取 payload + 解 PNG 的整段时间里
/// 用户盯着一整屏白色（webview 默认底色），画面才姗姗出现。现在窗口先隐藏，
/// 由前端决定显示时机；兜底定时器见 `overlay_windows::READY_FALLBACK_MS`。
#[tauri::command]
pub fn mark_capture_overlay_ready(
    label: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    validate_overlay_label(&label)?;
    let cursor = app_handle
        .cursor_position()
        .ok()
        .map(|cursor| (cursor.x, cursor.y));
    let plan = state.capture_manager.reveal(&label, cursor)?;
    overlay_windows::reveal(&app_handle, &label, plan.take_focus)?;
    Ok(())
}

#[tauri::command]
pub fn cancel_capture_overlay(
    session_id: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let session = state.capture_manager.finish(&session_id)?;
    overlay_windows::close(&app_handle, &session.overlay_labels());
    overlay_windows::restore(&app_handle, &session.restore_labels);
    Ok(())
}

/// 覆盖层内完成裁剪与标注之后的唯一提交入口。
///
/// PNG 由覆盖层的画布渲染（`renderExport` 已经把裁剪、图像调整和矢量标注合成进去），
/// 后端不再自己裁一遍——否则标注会被丢掉。会话在执行动作前先被认领，
/// 并发取消或连点两次都不会产生两份副作用。
#[tauri::command]
pub fn commit_capture_action(
    action: CaptureAction,
    session_id: String,
    png_base64: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<CaptureActionResult, String> {
    let png = decode_commit_png(&png_base64);
    let result = action_lifecycle::complete_capture_action(
        png,
        || state.capture_manager.finish(&session_id),
        |session| overlay_windows::close(&app_handle, &session.overlay_labels()),
        |session| overlay_windows::restore(&app_handle, &session.restore_labels),
        |png| execute_action(action, png, &app_handle, &state),
    );
    // 覆盖层在动作之前就关掉了，错误已经没有窗口可以显示——只能留在日志里，
    // 否则"点了对钩什么都没发生"完全无从排查。
    if let Err(error) = &result {
        log::warn!("截图提交失败: {error}");
    }
    result
}

/// 解码并校验覆盖层提交的 PNG。先看 base64 长度再解码，避免为一个畸形载荷先分配几百 MB。
fn decode_commit_png(png_base64: &str) -> Result<Vec<u8>, CaptureError> {
    // base64 每 4 个字符出 3 字节，用这个上界提前拒绝。
    if png_base64.len() / 4 * 3 > MAX_COMMIT_PNG_BYTES {
        return Err(CaptureError::CommitPayloadTooLarge);
    }
    let png = crate::screenshot::decode_png_base64(png_base64)
        .map_err(|_| CaptureError::CommitPayloadInvalid)?;
    if png.len() > MAX_COMMIT_PNG_BYTES {
        return Err(CaptureError::CommitPayloadTooLarge);
    }
    // 必须真的能解成图像：后续 copy/save/pin 都假设手里是合法 PNG。
    crate::screenshot::png_dimensions(&png).map_err(|_| CaptureError::CommitPayloadInvalid)?;
    Ok(png)
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

    // 选区译文不属于任何剪贴板条目，历史里以 clip_id = 0 记录。
    crate::translation::commands::record_translations(
        state.storage.clone(),
        None,
        source_text.clone(),
        vec![crate::translation::types::ServiceTranslation::from_result(
            translated.provider,
            Ok(translated.clone()),
        )],
    )
    .await;

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

/// 选区翻译与文本翻译共用同一个错误出口，保证日志标识和对外文案不分叉。
fn translation_ipc_error(error: TranslationError) -> String {
    crate::translation::commands::ipc_error(error)
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
            let path = crate::image_io::save_png(&png, "clippy-screenshot", &state.save_target())?;
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

/// 覆盖层无法建立时源窗口已经恢复，用户只会看到"截图没打开"，
/// 所以失败原因必须留在日志里才能排障。
fn capture_failure(error: CaptureError) -> String {
    let context = "打开截图覆盖层失败";
    // 快捷键连按会撞上仍在进行的会话，这是正常竞争而不是故障。
    if matches!(error, CaptureError::SessionBusy) {
        crate::error::note(context, error)
    } else {
        crate::error::report(context, error)
    }
}

fn validate_overlay_label(label: &str) -> Result<(), CaptureError> {
    if label.starts_with("capture-overlay-")
        && label.len() <= 128
        && label
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        Ok(())
    } else {
        Err(CaptureError::OverlayLabelInvalid)
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
    fn commit_payload_must_be_a_real_png_within_the_size_limit() {
        assert_eq!(
            decode_commit_png("not base64!!").unwrap_err().code(),
            "commit_payload_invalid"
        );
        // 合法 base64 但不是 PNG
        assert_eq!(
            decode_commit_png("aGVsbG8=").unwrap_err().code(),
            "commit_payload_invalid"
        );
        // 长度上界在解码之前就拦住，不为畸形载荷分配内存
        let oversized = "A".repeat(MAX_COMMIT_PNG_BYTES / 3 * 4 + 8);
        assert_eq!(
            decode_commit_png(&oversized).unwrap_err().code(),
            "commit_payload_too_large"
        );
    }

    #[test]
    fn commit_payload_accepts_the_data_url_form_the_canvas_produces() {
        let png = crate::screenshot::encode_png(&[255, 0, 0, 255], 1, 1).unwrap();
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png);
        assert_eq!(decode_commit_png(&encoded).unwrap(), png);
        assert_eq!(
            decode_commit_png(&format!("data:image/png;base64,{encoded}")).unwrap(),
            png
        );
    }

    #[test]
    fn capture_ocr_preserves_recognized_text_for_translation() {
        assert_eq!(
            normalize_ocr_text(Ok("local OCR text".to_string())),
            Ok("local OCR text".to_string())
        );
    }
}
