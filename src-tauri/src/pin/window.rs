use super::error::PinError;
use super::model::{window_marker, PinEntry, PinOrigin};
use tauri::{LogicalPosition, Manager, PhysicalPosition, PhysicalSize, Position, Size};

/// 内容区四周留给投影的空隙。同时也是内容区相对窗口原点的左/上偏移
/// （见 `src/react/pin/pin.css` 的 `.pin-media { inset: 12px 56px 60px 12px }`），
/// 所以"把内容区盖在原始矩形上"就是把窗口摆到原始矩形减去这个偏移的位置。
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
    origin: Option<PinOrigin>,
) -> Result<(), PinError> {
    let (outer_width, outer_height) = outer_size(content_width, content_height, 1.0);
    let window = tauri::WebviewWindowBuilder::new(
        app,
        label,
        tauri::WebviewUrl::App(format!("pin.html?label={label}").into()),
    )
    // 标题只做 GNOME Shell 扩展的查找键，界面上看不到（无装饰 + 不进任务栏）。
    .title(window_marker(label))
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
    .map_err(PinError::window)?;
    if let Err(error) = position_new_pin_window(app, &window, outer_width, outer_height, origin) {
        if let Err(close_error) = window.close() {
            log::warn!("关闭定位失败的贴图窗口失败: {close_error}");
        }
        return Err(error);
    }
    crate::pin_window::configure_pin_window(&window);
    Ok(())
}

/// 显示贴图窗口，并让它落到该去的位置、压在别的窗口上面。
///
/// 顺序不能换：Wayland 下只有窗口真的映射之后 Shell 里才有对应的 MetaWindow，
/// 扩展才找得到它，所以摆放必须在 `show()` 之后。
pub(super) fn reveal_pin_window(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    entry: &PinEntry,
) -> Result<(), PinError> {
    window.show().map_err(PinError::window)?;
    window.set_focus().map_err(PinError::window)?;
    keep_pin_above(window, pin_target_position(app, entry));
    Ok(())
}

/// 把窗口摆到 `logical`（逻辑像素，窗口左上角）并置顶。`None` 表示只置顶、不动位置。
///
/// Wayland 协议里客户端既无权决定自己窗口的位置、也无权置顶，Mutter 把
/// `set_position` / `set_always_on_top` 静默忽略——这正是"贴图出现在屏幕中间"和
/// "贴图被别的窗口盖住"的原因。只有 GNOME Shell 扩展进得去 Shell 里调
/// `MetaWindow.move_frame()` / `make_above()`。扩展不可用（没装、装了还没注销生效、
/// 不是 GNOME）时退回 Tauri 自己那套：在 X11 上它本来就管用。
///
/// 两条路都失败只意味着"位置或层级不理想"，绝不能让贴图本身失败。
pub(super) fn keep_pin_above(window: &tauri::WebviewWindow, logical: Option<LogicalPosition<f64>>) {
    let marker = window_marker(window.label());
    let target = logical.map(|position| (position.x.round() as i32, position.y.round() as i32));
    match crate::capture::shell_extension_place_window(&marker, target, true) {
        Ok(true) => return,
        Ok(false) => log::info!("GNOME Shell 扩展没在会话里找到贴图窗口 {marker}"),
        // 非 GNOME Wayland、未安装、未注销生效都走到这里，是常态而不是故障。
        Err(reason) => log::debug!("贴图窗口不经扩展摆放: {reason}"),
    }
    if let Err(error) = window.set_always_on_top(true) {
        log::warn!("贴图窗口置顶失败: {error}");
    }
    if let Some(position) = logical {
        if let Err(error) = window.set_position(Position::Logical(position)) {
            log::warn!("贴图窗口定位失败: {error}");
        }
    }
}

/// 贴图窗口该待的逻辑坐标：让内容区正好盖住图片原本所在的那块屏幕。
///
/// 没有原始矩形（从剪贴板历史贴的图、别的程序复制来的图）时返回 `None`，
/// 位置交给创建时的光标定位与合成器自己的摆放。
fn pin_target_position(app: &tauri::AppHandle, entry: &PinEntry) -> Option<LogicalPosition<f64>> {
    let origin = entry.origin?;
    let (outer_width, outer_height) =
        outer_size(entry.content_width, entry.content_height, entry.scale);
    let target = LogicalPosition::new(origin.x - SHADOW_GUTTER, origin.y - SHADOW_GUTTER);
    Some(clamp_logical_position(
        app,
        target,
        outer_width,
        outer_height,
    ))
}

/// 把逻辑坐标钳进"包含它的那块显示器"的逻辑工作区。找不到显示器就原样返回——
/// 摆得不完美也比因为查不到几何而放弃摆放要好。
fn clamp_logical_position(
    app: &tauri::AppHandle,
    position: LogicalPosition<f64>,
    width: f64,
    height: f64,
) -> LogicalPosition<f64> {
    let Some(area) = logical_work_area(app, position) else {
        return position;
    };
    LogicalPosition::new(
        clamp_span(position.x, area.x, area.width, width),
        clamp_span(position.y, area.y, area.height, height),
    )
}

/// 把 `value` 钳进 `[start, start + span - size]`，窗口比工作区还大时退化为贴边。
pub(super) fn clamp_span(value: f64, start: f64, span: f64, size: f64) -> f64 {
    value.clamp(start, (start + span - size).max(start))
}

/// 逻辑坐标系里的显示器工作区。
///
/// Tauri 的显示器几何是物理像素，而原始矩形与扩展给的窗口坐标都是逻辑像素；
/// 多屏混合缩放时不能用同一个系数换算整个桌面，所以逐屏折算再挑包含目标点的那块。
/// 一块都不包含时返回第一块（目标点在屏幕外，钳一下总比不管要好）。
struct LogicalWorkArea {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale: f64,
}

fn logical_work_area(
    app: &tauri::AppHandle,
    position: LogicalPosition<f64>,
) -> Option<LogicalWorkArea> {
    let monitors = app.available_monitors().ok()?;
    let mut fallback = None;
    for monitor in &monitors {
        let scale = monitor.scale_factor().max(0.1);
        let work = monitor.work_area();
        let area = LogicalWorkArea {
            x: work.position.x as f64 / scale,
            y: work.position.y as f64 / scale,
            width: work.size.width as f64 / scale,
            height: work.size.height as f64 / scale,
            scale,
        };
        if position.x >= area.x
            && position.x < area.x + area.width
            && position.y >= area.y
            && position.y < area.y + area.height
        {
            return Some(area);
        }
        fallback = fallback.or(Some(area));
    }
    fallback
}

pub(super) fn resize_pin_window(app: &tauri::AppHandle, entry: &PinEntry) -> Result<(), PinError> {
    let window = app
        .get_webview_window(&entry.label)
        .ok_or(PinError::WindowMissing)?;
    let monitor = window
        .current_monitor()
        .map_err(PinError::window)?
        .or(window.primary_monitor().map_err(PinError::window)?);
    let (logical_width, logical_height) =
        outer_size(entry.content_width, entry.content_height, entry.scale);
    let Some(monitor) = monitor else {
        return window
            .set_size(tauri::LogicalSize::new(logical_width, logical_height))
            .map_err(PinError::window);
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
        .map_err(PinError::window)?;
    window
        .set_position(Position::Physical(position))
        .map_err(PinError::window)?;
    // 缩放后位置也变了，Wayland 上 set_position 是空操作，得再走一次扩展；
    // 顺带重新置顶——改尺寸有可能把窗口带回普通层。
    keep_pin_above(
        &window,
        Some(LogicalPosition::new(
            position.x as f64 / scale_factor,
            position.y as f64 / scale_factor,
        )),
    );
    Ok(())
}

fn position_new_pin_window(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    logical_width: f64,
    logical_height: f64,
    origin: Option<PinOrigin>,
) -> Result<(), PinError> {
    let cursor = app.cursor_position().ok();
    // 有原始矩形就照它摆（截图贴回原处），否则跟着光标——两种情况都要先找到
    // 目标点所在的显示器，因为工作区与缩放都是按屏算的。
    let anchor = origin
        .map(|origin| {
            let target = LogicalPosition::new(origin.x - SHADOW_GUTTER, origin.y - SHADOW_GUTTER);
            let scale = logical_scale_near(app, target);
            PhysicalPosition::new((target.x * scale).round(), (target.y * scale).round())
        })
        .or(cursor.map(|position| {
            PhysicalPosition::new(position.x.round() + 12.0, position.y.round() + 12.0)
        }));
    let monitor = anchor
        .and_then(|position| {
            app.monitor_from_point(position.x, position.y)
                .ok()
                .flatten()
        })
        .or(app.primary_monitor().map_err(PinError::window)?);
    let Some(monitor) = monitor else {
        return Ok(());
    };
    let scale = monitor.scale_factor().max(0.1);
    let size = PhysicalSize::new(
        (logical_width * scale).round().max(1.0) as u32,
        (logical_height * scale).round().max(1.0) as u32,
    );
    let work = monitor.work_area();
    let raw = anchor
        .map(|position| PhysicalPosition::new(position.x as i32, position.y as i32))
        .unwrap_or_else(|| {
            PhysicalPosition::new(
                work.position.x + (work.size.width.saturating_sub(size.width) / 2) as i32,
                work.position.y + (work.size.height.saturating_sub(size.height) / 2) as i32,
            )
        });
    window
        .set_position(Position::Physical(clamp_pin_position(raw, size, work)))
        .map_err(PinError::window)
}

/// 目标逻辑点所在显示器的缩放系数。逻辑坐标要换成物理坐标才能喂给
/// `monitor_from_point` / `set_position`。
fn logical_scale_near(app: &tauri::AppHandle, position: LogicalPosition<f64>) -> f64 {
    logical_work_area(app, position)
        .map(|area| area.scale)
        .unwrap_or(1.0)
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

/// 有原始矩形时的内容区尺寸：就用原始尺寸，**不做** `fit_content_size` 的放大。
///
/// "贴回原尺寸"意味着一个 60×30 的小选区就该显示成 60×30；`fit_dimensions` 会把它撑到
/// 至少 180×120 以保证界面可用，那正好破坏了原尺寸。只在窗口连工作区都装不下时
/// 才按比例缩小——否则贴图会有一部分永远在屏幕外。
pub(super) fn origin_content_size(app: &tauri::AppHandle, origin: PinOrigin) -> (f64, f64) {
    let Some(area) = logical_work_area(app, LogicalPosition::new(origin.x, origin.y)) else {
        return (origin.width, origin.height);
    };
    let max_width = (area.width - SHADOW_GUTTER * 2.0 - CONTROLS_GUTTER).max(1.0);
    let max_height = (area.height - SHADOW_GUTTER * 2.0 - TOOLBAR_GUTTER).max(1.0);
    // 上限 1.0：只缩不放，"原尺寸"就是原尺寸。下限 0.01 防止极端矩形算出 0。
    let shrink = (max_width / origin.width)
        .min(max_height / origin.height)
        .clamp(0.01, 1.0);
    (origin.width * shrink, origin.height * shrink)
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
