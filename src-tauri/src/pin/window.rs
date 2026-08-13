use super::model::PinEntry;
use tauri::{Manager, PhysicalPosition, PhysicalSize, Position, Size};

const SHADOW_GUTTER: f64 = 12.0;
const CONTROLS_GUTTER: f64 = 44.0;
const TOOLBAR_GUTTER: f64 = 48.0;
const MIN_IMAGE_WIDTH: f64 = 180.0;
const MIN_IMAGE_HEIGHT: f64 = 120.0;

pub(super) fn create_pin_window(
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

pub(super) fn resize_pin_window(app: &tauri::AppHandle, entry: &PinEntry) -> Result<(), String> {
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

pub(super) fn clamp_pin_position(
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

pub(super) fn fit_content_size(app: &tauri::AppHandle, width: f64, height: f64) -> (f64, f64) {
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

pub(super) fn fit_dimensions(
    width: f64,
    height: f64,
    max_width: f64,
    max_height: f64,
) -> (f64, f64) {
    let width = width.max(1.0);
    let height = height.max(1.0);
    let maximum_scale = (max_width.max(1.0) / width).min(max_height.max(1.0) / height);
    let desired_scale = 1.0_f64
        .max(MIN_IMAGE_WIDTH / width)
        .max(MIN_IMAGE_HEIGHT / height);
    let scale = desired_scale.min(maximum_scale).max(0.01);
    (width * scale, height * scale)
}

pub(super) fn outer_size(content_width: f64, content_height: f64, scale: f64) -> (f64, f64) {
    (
        content_width * scale + SHADOW_GUTTER * 2.0 + CONTROLS_GUTTER,
        content_height * scale + SHADOW_GUTTER * 2.0 + TOOLBAR_GUTTER,
    )
}
