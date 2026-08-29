use super::model::{validate_label, PinEntry, PinPayload, PinSource, PinUpdate};
use super::window::{create_pin_window, fit_content_size, resize_pin_window};
use crate::commands::AppState;
use crate::models::ContentType;
use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};
use tauri::{Manager, State};

#[tauri::command]
pub fn pin_clip(
    id: i64,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let _transition = state
        .pin_transition
        .lock()
        .map_err(|error| error.to_string())?;
    let label = format!("pin-clip-{id}");
    if let Some(window) = app_handle.get_webview_window(&label) {
        if state.pin_manager.get(&label).is_ok() {
            window.show().map_err(|error| error.to_string())?;
            let _ = window.set_focus();
            return Ok(label);
        }
        // A previous creation failed after the native window was built. Do not
        // reuse an orphaned window with no payload entry.
        let _ = window.close();
        return Err("贴图窗口状态不完整，请重试".to_string());
    }

    let (item, image) = {
        let storage = state.storage.lock().map_err(|error| error.to_string())?;
        let item = storage
            .get_clip_by_id(id)
            .map_err(|error| error.to_string())?;
        let image = if item.content_type == ContentType::Image {
            storage
                .get_clip_image(id)
                .map_err(|error| error.to_string())?
        } else {
            None
        };
        (item, image)
    };
    let (width, height) = image
        .as_deref()
        .and_then(|png| crate::screenshot::png_dimensions(png).ok())
        .map(|(width, height)| (width as f64, height as f64))
        .unwrap_or((420.0, 280.0));
    let (content_width, content_height) = fit_content_size(&app_handle, width, height);
    state.pin_manager.insert(PinEntry {
        label: label.clone(),
        source: PinSource::Clip { item, image },
        content_width,
        content_height,
        scale: 1.0,
        opacity: 1.0,
        locked: false,
        position: None,
    })?;
    if let Err(error) = create_pin_window(&app_handle, &label, content_width, content_height) {
        let _ = state.pin_manager.remove(&label);
        return Err(crate::error::report("创建剪贴板贴图窗口失败", error));
    }
    Ok(label)
}

#[tauri::command]
pub fn pin_screenshot_image(
    png_base64: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let png =
        crate::screenshot::decode_png_base64(&png_base64).map_err(|error| error.to_string())?;
    create_screenshot_pin(png, &app_handle, &state)
}

pub(crate) fn create_screenshot_pin(
    png: Vec<u8>,
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<String, String> {
    let _transition = state
        .pin_transition
        .lock()
        .map_err(|error| error.to_string())?;
    let (width, height) =
        crate::screenshot::png_dimensions(&png).map_err(|error| error.to_string())?;
    let label = format!("pin-image-{}", crate::image_io::unique_image_id());
    let (content_width, content_height) = fit_content_size(app_handle, width as f64, height as f64);
    state.pin_manager.insert(PinEntry {
        label: label.clone(),
        source: PinSource::Screenshot { png },
        content_width,
        content_height,
        scale: 1.0,
        opacity: 1.0,
        locked: false,
        position: None,
    })?;
    if let Err(error) = create_pin_window(app_handle, &label, content_width, content_height) {
        let _ = state.pin_manager.remove(&label);
        return Err(crate::error::report("创建截图贴图窗口失败", error));
    }
    Ok(label)
}

#[tauri::command]
pub fn get_pin_payload(label: String, state: State<'_, AppState>) -> Result<PinPayload, String> {
    validate_label(&label)?;
    payload_from_entry(state.pin_manager.get(&label)?)
}

#[tauri::command]
pub fn pin_ready(
    label: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _transition = state
        .pin_transition
        .lock()
        .map_err(|error| error.to_string())?;
    validate_label(&label)?;
    state.pin_manager.get(&label)?;
    let window = app_handle
        .get_webview_window(&label)
        .ok_or_else(|| "贴图窗口不存在".to_string())?;
    crate::pin_window::configure_pin_window(&window);
    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn update_pin(
    label: String,
    update: PinUpdate,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<PinPayload, String> {
    validate_label(&label)?;
    let _transition = state
        .pin_transition
        .lock()
        .map_err(|error| error.to_string())?;
    let previous = state.pin_manager.get(&label)?;
    let entry = state.pin_manager.update(&label, &update)?;
    if update.scale.is_some() {
        if let Err(error) = resize_pin_window(&app_handle, &entry) {
            if let Err(rollback_error) = resize_pin_window(&app_handle, &previous) {
                log::warn!("回滚贴图窗口尺寸失败: {rollback_error}");
            }
            state.pin_manager.replace(previous)?;
            // 缩放途中窗口被关掉属于正常竞争，不是故障。
            let context = "缩放贴图窗口失败";
            return Err(if error.is_gone() {
                crate::error::note(context, error)
            } else {
                crate::error::report(context, error)
            });
        }
    }
    payload_from_entry(entry)
}

#[tauri::command]
pub fn copy_pin(label: String, state: State<'_, AppState>) -> Result<(), String> {
    validate_label(&label)?;
    let entry = state.pin_manager.get(&label)?;
    match entry.source {
        PinSource::Clip { item, .. } => crate::commands::write_clip_to_clipboard(item.id, &state),
        PinSource::Screenshot { png } => {
            let hash = format!("{:x}", Sha256::new_with_prefix(&png).finalize());
            state.watcher.set_skip_hash(hash);
            crate::image_io::copy_png_to_clipboard(&png)
        }
    }
}

#[tauri::command]
pub fn save_pin(label: String, state: State<'_, AppState>) -> Result<String, String> {
    validate_label(&label)?;
    let png = image_bytes(&state.pin_manager.get(&label)?)?;
    let path = crate::image_io::save_png(&png, "clippy-pin")?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn edit_pin(
    label: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    validate_label(&label)?;
    let png = image_bytes(&state.pin_manager.get(&label)?)?;
    let (width, height) =
        crate::screenshot::png_dimensions(&png).map_err(|error| error.to_string())?;
    crate::commands::queue_capture_for_editor(
        &app_handle,
        &state,
        STANDARD.encode(png),
        width,
        height,
    )
}

#[tauri::command]
pub fn close_pin(
    label: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    validate_label(&label)?;
    let _transition = state
        .pin_transition
        .lock()
        .map_err(|error| error.to_string())?;
    if let Some(window) = app_handle.get_webview_window(&label) {
        window.close().map_err(|error| error.to_string())?;
    }
    let _ = state.pin_manager.remove(&label)?;
    Ok(())
}

fn payload_from_entry(entry: PinEntry) -> Result<PinPayload, String> {
    let (kind, text, image_base64, can_save, can_edit) = match &entry.source {
        PinSource::Clip { item, image } => match item.content_type {
            ContentType::Image => (
                "image",
                None,
                image.as_ref().map(|png| STANDARD.encode(png)),
                true,
                true,
            ),
            ContentType::Text | ContentType::Html => {
                ("text", item.text_content.clone(), None, false, false)
            }
        },
        PinSource::Screenshot { png } => ("image", None, Some(STANDARD.encode(png)), true, true),
    };
    Ok(PinPayload {
        label: entry.label,
        kind,
        text,
        image_base64,
        content_width: entry.content_width,
        content_height: entry.content_height,
        scale: entry.scale,
        opacity: entry.opacity,
        locked: entry.locked,
        can_save,
        can_edit,
        position: entry.position,
    })
}

fn image_bytes(entry: &PinEntry) -> Result<Vec<u8>, String> {
    match &entry.source {
        PinSource::Clip {
            image: Some(png), ..
        }
        | PinSource::Screenshot { png } => Ok(png.clone()),
        _ => Err("文本贴图不能保存或编辑为图片".to_string()),
    }
}
