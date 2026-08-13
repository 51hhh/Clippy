use crate::commands::AppState;
use crate::models::{ClipItem, ContentType};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{Manager, PhysicalPosition, PhysicalSize, Position, Size, State};

const SHADOW_GUTTER: f64 = 12.0;
const CONTROLS_GUTTER: f64 = 44.0;
const TOOLBAR_GUTTER: f64 = 48.0;
const MIN_IMAGE_WIDTH: f64 = 180.0;
const MIN_IMAGE_HEIGHT: f64 = 120.0;

#[derive(Debug, Clone)]
enum PinSource {
    Clip {
        item: ClipItem,
        image: Option<Vec<u8>>,
    },
    Screenshot {
        png: Vec<u8>,
    },
}

#[derive(Debug, Clone)]
struct PinEntry {
    label: String,
    source: PinSource,
    content_width: f64,
    content_height: f64,
    scale: f64,
    opacity: f64,
    locked: bool,
    position: Option<PinPosition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PinPosition {
    pub x: i32,
    pub y: i32,
}

#[derive(Default)]
pub struct PinManager {
    entries: Mutex<HashMap<String, PinEntry>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinPayload {
    pub label: String,
    pub kind: &'static str,
    pub text: Option<String>,
    pub image_base64: Option<String>,
    pub content_width: f64,
    pub content_height: f64,
    pub scale: f64,
    pub opacity: f64,
    pub locked: bool,
    pub can_save: bool,
    pub can_edit: bool,
    pub position: Option<PinPosition>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinUpdate {
    pub scale: Option<f64>,
    pub opacity: Option<f64>,
    pub locked: Option<bool>,
}

impl PinManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn insert(&self, entry: PinEntry) -> Result<(), String> {
        self.entries
            .lock()
            .map_err(|error| error.to_string())?
            .insert(entry.label.clone(), entry);
        Ok(())
    }

    fn get(&self, label: &str) -> Result<PinEntry, String> {
        self.entries
            .lock()
            .map_err(|error| error.to_string())?
            .get(label)
            .cloned()
            .ok_or_else(|| "贴图不存在或已经关闭".to_string())
    }

    fn remove(&self, label: &str) -> Result<Option<PinEntry>, String> {
        Ok(self
            .entries
            .lock()
            .map_err(|error| error.to_string())?
            .remove(label))
    }

    fn update(&self, label: &str, update: &PinUpdate) -> Result<PinEntry, String> {
        let mut entries = self.entries.lock().map_err(|error| error.to_string())?;
        let entry = entries
            .get_mut(label)
            .ok_or_else(|| "贴图不存在或已经关闭".to_string())?;
        if let Some(scale) = update.scale {
            entry.scale = scale.clamp(0.25, 4.0);
        }
        if let Some(opacity) = update.opacity {
            entry.opacity = opacity.clamp(0.15, 1.0);
        }
        if let Some(locked) = update.locked {
            entry.locked = locked;
        }
        Ok(entry.clone())
    }

    fn remember_position(&self, label: &str, position: PhysicalPosition<i32>) {
        if !is_safe_pin_label(label) {
            return;
        }
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        if let Some(entry) = entries.get_mut(label) {
            entry.position = Some(PinPosition {
                x: position.x,
                y: position.y,
            });
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries
            .lock()
            .map(|entries| entries.len())
            .unwrap_or(0)
    }

    pub fn remove_window(&self, label: &str) {
        if is_safe_pin_label(label) {
            let _ = self.remove(label);
        }
    }
}

pub(crate) fn remember_pin_window_position(
    manager: &PinManager,
    label: &str,
    position: PhysicalPosition<i32>,
) {
    manager.remember_position(label, position);
}

#[tauri::command]
pub fn pin_clip(
    id: i64,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let label = format!("pin-clip-{id}");
    if let Some(window) = app_handle.get_webview_window(&label) {
        window.show().map_err(|error| error.to_string())?;
        let _ = window.set_focus();
        return Ok(label);
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
        return Err(error);
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
        return Err(error);
    }
    Ok(label)
}

#[tauri::command]
pub fn get_pin_payload(label: String, state: State<'_, AppState>) -> Result<PinPayload, String> {
    validate_label(&label)?;
    payload_from_entry(state.pin_manager.get(&label)?)
}

#[tauri::command]
pub fn pin_ready(label: String, app_handle: tauri::AppHandle) -> Result<(), String> {
    validate_label(&label)?;
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
    let entry = state.pin_manager.update(&label, &update)?;
    if update.scale.is_some() {
        resize_pin_window(&app_handle, &entry)?;
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

fn create_pin_window(
    app: &tauri::AppHandle,
    label: &str,
    content_width: f64,
    content_height: f64,
) -> Result<(), String> {
    let (outer_width, outer_height) = outer_size(content_width, content_height, 1.0);
    let window = tauri::WebviewWindowBuilder::new(
        app,
        label,
        tauri::WebviewUrl::App(format!("pin.html?label={label}").into()),
    )
    .title("")
    .inner_size(outer_width, outer_height)
    .decorations(false)
    .always_on_top(true)
    .transparent(true)
    .shadow(false)
    .skip_taskbar(true)
    .resizable(false)
    .visible(false)
    .center()
    .build()
    .map_err(|error| error.to_string())?;
    position_new_pin_window(app, &window, outer_width, outer_height)?;
    crate::pin_window::configure_pin_window(&window);
    Ok(())
}

fn resize_pin_window(app: &tauri::AppHandle, entry: &PinEntry) -> Result<(), String> {
    let window = app
        .get_webview_window(&entry.label)
        .ok_or_else(|| "贴图窗口不存在".to_string())?;
    let monitor = window
        .current_monitor()
        .map_err(|error| error.to_string())?
        .or(window
            .primary_monitor()
            .map_err(|error| error.to_string())?);
    let (logical_width, logical_height) =
        outer_size(entry.content_width, entry.content_height, entry.scale);
    let Some(monitor) = monitor else {
        return window
            .set_size(tauri::LogicalSize::new(logical_width, logical_height))
            .map_err(|error| error.to_string());
    };
    let scale_factor = monitor.scale_factor().max(0.1);
    let work = monitor.work_area();
    let requested = PhysicalSize::new(
        (logical_width * scale_factor).round() as u32,
        (logical_height * scale_factor).round() as u32,
    );
    let size = PhysicalSize::new(
        requested
            .width
            .min(work.size.width.saturating_sub(16).max(1)),
        requested
            .height
            .min(work.size.height.saturating_sub(16).max(1)),
    );
    let old_size = window.outer_size().unwrap_or(size);
    let old_position = entry
        .position
        .map(|position| PhysicalPosition::new(position.x, position.y))
        .or_else(|| window.outer_position().ok())
        .unwrap_or(work.position);
    let centered = PhysicalPosition::new(
        old_position.x + (old_size.width as i32 - size.width as i32) / 2,
        old_position.y + (old_size.height as i32 - size.height as i32) / 2,
    );
    let position = clamp_pin_position(centered, size, work);
    window
        .set_size(Size::Physical(size))
        .map_err(|error| error.to_string())?;
    window
        .set_position(Position::Physical(position))
        .map_err(|error| error.to_string())
}

fn position_new_pin_window(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    logical_width: f64,
    logical_height: f64,
) -> Result<(), String> {
    let cursor = app.cursor_position().ok();
    let monitor = cursor
        .and_then(|position| {
            app.monitor_from_point(position.x, position.y)
                .ok()
                .flatten()
        })
        .or(app.primary_monitor().map_err(|error| error.to_string())?);
    let Some(monitor) = monitor else {
        return Ok(());
    };
    let scale = monitor.scale_factor().max(0.1);
    let size = PhysicalSize::new(
        (logical_width * scale).round().max(1.0) as u32,
        (logical_height * scale).round().max(1.0) as u32,
    );
    let work = monitor.work_area();
    let raw = cursor
        .map(|position| {
            PhysicalPosition::new(
                position.x.round() as i32 + 12,
                position.y.round() as i32 + 12,
            )
        })
        .unwrap_or_else(|| {
            PhysicalPosition::new(
                work.position.x + (work.size.width.saturating_sub(size.width) / 2) as i32,
                work.position.y + (work.size.height.saturating_sub(size.height) / 2) as i32,
            )
        });
    window
        .set_position(Position::Physical(clamp_pin_position(raw, size, work)))
        .map_err(|error| error.to_string())
}

fn clamp_pin_position(
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    work: &tauri::PhysicalRect<i32, u32>,
) -> PhysicalPosition<i32> {
    let max_x = work.position.x + work.size.width.saturating_sub(size.width) as i32;
    let max_y = work.position.y + work.size.height.saturating_sub(size.height) as i32;
    PhysicalPosition::new(
        position
            .x
            .clamp(work.position.x, max_x.max(work.position.x)),
        position
            .y
            .clamp(work.position.y, max_y.max(work.position.y)),
    )
}

fn fit_content_size(app: &tauri::AppHandle, width: f64, height: f64) -> (f64, f64) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|cursor| app.monitor_from_point(cursor.x, cursor.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let (max_width, max_height) = monitor
        .map(|monitor| {
            let work = monitor.work_area();
            let scale = monitor.scale_factor().max(0.1);
            (
                work.size.width as f64 / scale * 0.72 - CONTROLS_GUTTER,
                work.size.height as f64 / scale * 0.72 - TOOLBAR_GUTTER,
            )
        })
        .unwrap_or((900.0, 700.0));
    fit_dimensions(width, height, max_width, max_height)
}

fn fit_dimensions(width: f64, height: f64, max_width: f64, max_height: f64) -> (f64, f64) {
    let width = width.max(1.0);
    let height = height.max(1.0);
    let maximum_scale = (max_width.max(1.0) / width).min(max_height.max(1.0) / height);
    let desired_scale = 1.0_f64
        .max(MIN_IMAGE_WIDTH / width)
        .max(MIN_IMAGE_HEIGHT / height);
    let scale = desired_scale.min(maximum_scale).max(0.01);
    (width * scale, height * scale)
}

fn outer_size(content_width: f64, content_height: f64, scale: f64) -> (f64, f64) {
    (
        content_width * scale + SHADOW_GUTTER * 2.0 + CONTROLS_GUTTER,
        content_height * scale + SHADOW_GUTTER * 2.0 + TOOLBAR_GUTTER,
    )
}

fn validate_label(label: &str) -> Result<(), String> {
    if is_safe_pin_label(label) {
        Ok(())
    } else {
        Err("无效的贴图窗口标签".to_string())
    }
}

fn is_safe_pin_label(label: &str) -> bool {
    label.starts_with("pin-")
        && label.len() <= 96
        && label
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screenshot_entry(label: &str) -> PinEntry {
        PinEntry {
            label: label.to_string(),
            source: PinSource::Screenshot { png: vec![1, 2, 3] },
            content_width: 320.0,
            content_height: 180.0,
            scale: 1.0,
            opacity: 1.0,
            locked: false,
            position: None,
        }
    }

    #[test]
    fn fits_large_images_without_changing_aspect_ratio() {
        assert_eq!(
            fit_dimensions(3840.0, 2160.0, 900.0, 700.0),
            (900.0, 506.25)
        );
    }

    #[test]
    fn sizing_preserves_small_and_extreme_aspect_ratios() {
        assert_eq!(fit_dimensions(120.0, 80.0, 900.0, 700.0), (180.0, 120.0));
        assert_eq!(fit_dimensions(1.0, 1000.0, 900.0, 700.0), (0.7, 700.0));
    }

    #[test]
    fn pin_labels_reject_path_and_query_characters() {
        assert!(is_safe_pin_label("pin-image-123"));
        assert!(!is_safe_pin_label("pin-../../secret"));
        assert!(!is_safe_pin_label("pin-id?x=1"));
    }

    #[test]
    fn outer_size_reserves_controls_and_shadow() {
        assert_eq!(outer_size(400.0, 300.0, 1.0), (468.0, 372.0));
        assert_eq!(outer_size(400.0, 300.0, 0.5), (268.0, 222.0));
    }

    #[test]
    fn manager_tracks_position_and_releases_destroyed_window_state() {
        let manager = PinManager::new();
        manager.insert(screenshot_entry("pin-image-test")).unwrap();

        manager.remember_position("pin-image-test", PhysicalPosition::new(-420, 36));
        assert_eq!(
            manager.get("pin-image-test").unwrap().position,
            Some(PinPosition { x: -420, y: 36 })
        );
        assert_eq!(manager.len(), 1);

        manager.remove_window("pin-image-test");
        assert_eq!(manager.len(), 0);
        assert!(manager.get("pin-image-test").is_err());
    }

    #[test]
    fn pin_position_clamps_to_negative_origin_work_area() {
        let work = tauri::PhysicalRect {
            position: PhysicalPosition::new(-1920, 24),
            size: PhysicalSize::new(1920, 1056),
        };
        assert_eq!(
            clamp_pin_position(
                PhysicalPosition::new(-100, 1000),
                PhysicalSize::new(500, 400),
                &work,
            ),
            PhysicalPosition::new(-500, 680)
        );
        assert_eq!(
            clamp_pin_position(
                PhysicalPosition::new(-5000, -5000),
                PhysicalSize::new(4000, 2000),
                &work,
            ),
            work.position
        );
    }
}
