use super::model::{
    validate_label, PinEntry, PinOrigin, PinPayload, PinSource, PinState, PinUpdate,
};
use super::window::{
    create_pin_window, fit_content_size, origin_content_size, resize_pin_window, reveal_pin_window,
};
use crate::commands::AppState;
use crate::models::ContentType;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::sync::Arc;
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
    // 这张图是不是我们自己截下来复制进剪贴板的？是的话它带着原始矩形，
    // 贴图就该回到那块屏幕、按那个尺寸；别处来的图片查不到，走常规缩放与光标定位。
    let origin = image
        .as_deref()
        .and_then(|png| state.pin_origins.lookup(png));
    let (content_width, content_height) = match origin {
        Some(origin) => origin_content_size(&app_handle, origin),
        None => fit_content_size(&app_handle, width, height),
    };
    state.pin_manager.insert(PinEntry {
        label: label.clone(),
        source: Arc::new(PinSource::Clip { item, image }),
        content_width,
        content_height,
        scale: 1.0,
        opacity: 1.0,
        locked: false,
        position: None,
        origin,
    })?;
    if let Err(error) =
        create_pin_window(&app_handle, &label, content_width, content_height, origin)
    {
        let _ = state.pin_manager.remove(&label);
        return Err(crate::error::report("创建剪贴板贴图窗口失败", error));
    }
    Ok(label)
}

/// `origin` 是这张图在屏幕上原本占的矩形（逻辑像素）。截图覆盖层知道选区落在哪，
/// 于是贴图能贴回原处、原尺寸；不知道来源的图片传 `None`，落回光标附近。
pub(crate) fn create_screenshot_pin(
    png: Vec<u8>,
    origin: Option<PinOrigin>,
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
    let origin = origin.and_then(PinOrigin::sanitized);
    let (content_width, content_height) = match origin {
        Some(origin) => origin_content_size(app_handle, origin),
        None => fit_content_size(app_handle, width as f64, height as f64),
    };
    state.pin_manager.insert(PinEntry {
        label: label.clone(),
        source: Arc::new(PinSource::Screenshot { png }),
        content_width,
        content_height,
        scale: 1.0,
        opacity: 1.0,
        locked: false,
        position: None,
        origin,
    })?;
    if let Err(error) = create_pin_window(app_handle, &label, content_width, content_height, origin)
    {
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
    let entry = state.pin_manager.get(&label)?;
    let window = app_handle
        .get_webview_window(&label)
        .ok_or_else(|| "贴图窗口不存在".to_string())?;
    // 平台适配（置顶 + 缩放锁）在建窗时就做过了，这里只负责显示与摆位；
    // 重复调用会给 zoom-level 挂上第二个回调。
    reveal_pin_window(&app_handle, &window, &entry).map_err(|error| error.to_string())
}

/// 改缩放/不透明度/锁定，应答只带变了的那几个字段（见 `PinState`）。
///
/// **这是每帧都会走的路**：滚轮缩放时前端按 rAF 合并后仍是一帧一次。同步命令跑在
/// 主线程上，所以这条路上不能有整张图的 base64、也不能有多余的文件读与 D-Bus 握手
/// （摆位那侧的缓存见 `capture::shell_extension::place_window`）。
#[tauri::command]
pub fn update_pin(
    label: String,
    update: PinUpdate,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<PinState, String> {
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
    Ok(state_from_entry(&entry))
}

#[tauri::command]
pub fn copy_pin(label: String, state: State<'_, AppState>) -> Result<(), String> {
    validate_label(&label)?;
    let entry = state.pin_manager.get(&label)?;
    match &*entry.source {
        PinSource::Clip { item, .. } => crate::commands::write_clip_to_clipboard(item.id, &state),
        // 这里**不设** skip hash：watcher 哈希的是它自己从剪贴板 RGBA 重新编出来的 PNG，
        // 和我们手上这串字节几乎不可能相同，设了也永远匹配不上（历史上就是这样白算了一次
        // 全图 sha256）。后果只是这张图重新进一次历史——`insert_clip` 按哈希去重，
        // 已有的那条只会被顶到最前面，没有重复存储。
        PinSource::Screenshot { png } => crate::image_io::copy_png_to_clipboard(png),
    }
}

#[tauri::command]
pub fn save_pin(label: String, state: State<'_, AppState>) -> Result<String, String> {
    validate_label(&label)?;
    let png = image_bytes(&state.pin_manager.get(&label)?)?;
    let path = crate::image_io::save_png(&png, "clippy-pin", &state.save_target())?;
    Ok(path.to_string_lossy().to_string())
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

fn state_from_entry(entry: &PinEntry) -> PinState {
    PinState {
        label: entry.label.clone(),
        content_width: entry.content_width,
        content_height: entry.content_height,
        scale: entry.scale,
        opacity: entry.opacity,
        locked: entry.locked,
        position: entry.position,
    }
}

fn payload_from_entry(entry: PinEntry) -> Result<PinPayload, String> {
    let (kind, text, image_base64, can_save) = match &*entry.source {
        PinSource::Clip { item, image } => match item.content_type {
            ContentType::Image => (
                "image",
                None,
                image.as_ref().map(|png| STANDARD.encode(png)),
                true,
            ),
            ContentType::Text | ContentType::Html => {
                ("text", item.text_content.clone(), None, false)
            }
        },
        PinSource::Screenshot { png } => ("image", None, Some(STANDARD.encode(png)), true),
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
        position: entry.position,
    })
}

fn image_bytes(entry: &PinEntry) -> Result<Vec<u8>, String> {
    match &*entry.source {
        PinSource::Clip {
            image: Some(png), ..
        }
        | PinSource::Screenshot { png } => Ok(png.clone()),
        _ => Err("文本贴图不能保存或编辑为图片".to_string()),
    }
}
