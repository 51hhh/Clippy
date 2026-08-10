mod manager;
mod overlay_windows;
mod types;
mod window_probe;

pub use manager::CaptureManager;
pub use types::{CaptureAction, CaptureActionResult, CaptureOverlayPayload, CaptureSelection};

use crate::commands::AppState;
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
        crate::screenshot::capture_monitor_frames()
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
    let result = execute_action(action, png, &app_handle, &state)?;
    let session = state.capture_manager.finish(&selection.session_id)?;
    overlay_windows::close(&app_handle, &session.overlay_labels);
    Ok(result)
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
