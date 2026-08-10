//! 截图捕获与 PNG 编码
//!
//! Linux 捕获 fallback 参考 flashot（MIT）：优先 Wayland/wlroots，
//! 再尝试 XDG Portal、GNOME Shell，最后回退 xcap/XRandR。

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
#[cfg(test)]
use image::ImageBuffer;
use image::{
    codecs::png::{CompressionType, FilterType, PngEncoder},
    ExtendedColorType, GenericImageView, ImageEncoder, RgbaImage,
};
use serde::Serialize;
use std::sync::Arc;
use xcap::Monitor;

#[derive(Debug, Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone)]
struct MonitorInfo {
    id: u32,
    rect: Rect,
    scale_factor: f32,
}

#[derive(Debug, Clone)]
struct FrozenFrame {
    monitor_id: u32,
    rgba: Arc<[u8]>,
    width: u32,
    height: u32,
    scale_factor: f32,
}

#[derive(Debug, Clone)]
pub(crate) struct CapturedMonitorFrame {
    pub monitor_id: u32,
    pub x: i32,
    pub y: i32,
    pub logical_width: u32,
    pub logical_height: u32,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub rgba: Arc<[u8]>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedScreenshot {
    pub png_base64: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DesktopBounds {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ImageRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
enum Axis {
    X,
    Y,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
struct AxisSegment {
    start: i32,
    end: i32,
    offset: u32,
    scale: f32,
}

#[cfg(test)]
#[derive(Debug, Clone)]
struct AxisMapper {
    segments: Vec<AxisSegment>,
}

#[cfg(test)]
impl AxisMapper {
    fn from_frames(monitors: &[MonitorInfo], frames: &[FrozenFrame], axis: Axis) -> Result<Self> {
        let mut edges = Vec::with_capacity(monitors.len() * 2);
        for monitor in monitors {
            let (start, length) = monitor_axis_bounds(monitor, axis);
            edges.push(start);
            edges.push(start.saturating_add_unsigned(length));
        }
        edges.sort_unstable();
        edges.dedup();

        if edges.len() < 2 {
            bail!("显示器坐标轴为空");
        }

        let mut offset = 0_u32;
        let mut segments = Vec::with_capacity(edges.len().saturating_sub(1));
        for pair in edges.windows(2) {
            let start = pair[0];
            let end = pair[1];
            if end <= start {
                continue;
            }

            let scale = monitors
                .iter()
                .filter(|monitor| monitor_axis_overlaps(monitor, axis, start, end))
                .map(|monitor| {
                    let frame = frames.iter().find(|frame| frame.monitor_id == monitor.id);
                    monitor_axis_scale(monitor, frame, axis)
                })
                .filter(|scale| scale.is_finite() && *scale > 0.0)
                .fold(0.0_f32, f32::max);
            let scale = if scale > 0.0 { scale } else { 1.0 };
            segments.push(AxisSegment {
                start,
                end,
                offset,
                scale,
            });
            offset = offset.saturating_add(scaled_axis_length(end - start, scale));
        }

        if segments.is_empty() {
            bail!("显示器坐标轴没有有效区间");
        }

        Ok(Self { segments })
    }

    fn map(&self, coordinate: i32) -> u32 {
        let mut last_end = 0_u32;
        for segment in &self.segments {
            if coordinate < segment.start {
                return segment.offset;
            }
            let segment_end = segment.offset.saturating_add(scaled_axis_length(
                segment.end - segment.start,
                segment.scale,
            ));
            if coordinate <= segment.end {
                return segment.offset.saturating_add(scaled_axis_length(
                    coordinate - segment.start,
                    segment.scale,
                ));
            }
            last_end = segment_end;
        }
        last_end
    }
}

pub(crate) fn capture_monitor_frames() -> Result<Vec<CapturedMonitorFrame>> {
    let (monitors, frames) = capture_all_monitors()?;
    frames
        .into_iter()
        .map(|frame| {
            let monitor = monitors
                .iter()
                .find(|monitor| monitor.id == frame.monitor_id)
                .with_context(|| format!("缺少显示器 {} 的几何信息", frame.monitor_id))?;
            let scale_x = if monitor.rect.width > 0 {
                frame.width as f32 / monitor.rect.width as f32
            } else {
                frame.scale_factor
            };
            let scale_y = if monitor.rect.height > 0 {
                frame.height as f32 / monitor.rect.height as f32
            } else {
                frame.scale_factor
            };
            Ok(CapturedMonitorFrame {
                monitor_id: frame.monitor_id,
                x: monitor.rect.x,
                y: monitor.rect.y,
                logical_width: monitor.rect.width,
                logical_height: monitor.rect.height,
                pixel_width: frame.width,
                pixel_height: frame.height,
                scale_x,
                scale_y,
                rgba: frame.rgba,
            })
        })
        .collect()
}

pub fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .context("PNG 解码失败")
        .map(|img| img.dimensions())
}

pub fn decode_png_base64(input: &str) -> Result<Vec<u8>> {
    let payload = input
        .strip_prefix("data:image/png;base64,")
        .unwrap_or(input);
    STANDARD.decode(payload).context("PNG base64 解码失败")
}

pub fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let mut png = Vec::with_capacity(rgba.len().min(1024 * 1024));
    PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Adaptive)
        .write_image(rgba, width, height, ExtendedColorType::Rgba8)
        .context("PNG 编码失败")?;
    Ok(png)
}

#[cfg(target_os = "linux")]
fn capture_all_monitors() -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    if is_wayland_session() {
        match capture_all_wayland_monitors() {
            Ok(result) => return Ok(result),
            Err(e) => log::warn!("Wayland wlroots 截图失败，回退到 Portal: {e:#}"),
        }

        match capture_all_portal_monitors() {
            Ok(result) => return Ok(result),
            Err(e) => log::warn!("XDG Portal 截图失败，回退到 GNOME Shell: {e:#}"),
        }

        match capture_all_gnome_shell_monitors() {
            Ok(result) => return Ok(result),
            Err(e) => log::warn!("GNOME Shell 截图失败，回退到 xcap: {e:#}"),
        }
    }

    capture_all_xcap_monitors()
}

#[cfg(not(target_os = "linux"))]
fn capture_all_monitors() -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    capture_all_xcap_monitors()
}

fn capture_all_xcap_monitors() -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    let monitors = Monitor::all().context("无法枚举显示器")?;
    let mut infos = Vec::new();
    let mut frames = Vec::new();

    for mon in monitors.iter() {
        let info = monitor_info(mon)?;
        let img = mon.capture_image().context("无法捕获显示器")?;
        let frame_width = img.width();
        let frame_height = img.height();
        frames.push(FrozenFrame {
            monitor_id: info.id,
            rgba: Arc::from(img.into_raw()),
            width: frame_width,
            height: frame_height,
            scale_factor: info.scale_factor,
        });
        infos.push(info);
    }

    Ok((infos, frames))
}

#[cfg(target_os = "linux")]
fn capture_all_wayland_monitors() -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    let conn = wayland_connection()?;
    let outputs = conn.get_all_outputs().to_vec();
    if outputs.is_empty() {
        bail!("Wayland compositor 未报告输出");
    }

    let mut infos = Vec::with_capacity(outputs.len());
    let mut frames = Vec::with_capacity(outputs.len());
    for (index, output) in outputs.iter().enumerate() {
        let info = monitor_info_from_wayland_output(index, output);
        let image = conn
            .screenshot_single_output(output, false)
            .with_context(|| format!("无法捕获 Wayland 输出 {}", output.name))?;
        let rgba_image = image.to_rgba8();
        frames.push(FrozenFrame {
            monitor_id: info.id,
            width: rgba_image.width(),
            height: rgba_image.height(),
            rgba: Arc::from(rgba_image.into_raw()),
            scale_factor: info.scale_factor,
        });
        infos.push(info);
    }

    Ok((infos, frames))
}

#[cfg(target_os = "linux")]
fn capture_all_portal_monitors() -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    let monitors = enumerate_wayland_monitors()
        .or_else(|e| {
            log::warn!("Portal 截图无法复用 Wayland 几何，尝试 xcap 几何: {e:#}");
            enumerate_xcap_monitors()
        })
        .context("无法枚举 Portal 截图显示器")?;

    let screenshot = request_portal_screenshot(false)
        .or_else(|e| {
            log::warn!("非交互 Portal 截图失败，尝试交互模式: {e:#}");
            request_portal_screenshot(true)
        })
        .context("无法请求 Portal 截图")?;
    let path = portal_screenshot_uri_to_path(screenshot.uri().as_str())?;
    let bytes =
        std::fs::read(&path).with_context(|| format!("无法读取 Portal 截图 {}", path.display()))?;
    let image = image::load_from_memory(&bytes)
        .context("无法解码 Portal 截图")?
        .to_rgba8();

    split_portal_screenshot(monitors, image)
}

#[cfg(target_os = "linux")]
fn capture_all_gnome_shell_monitors() -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    let monitors = enumerate_wayland_monitors()
        .or_else(|e| {
            log::warn!("GNOME Shell 截图无法复用 Wayland 几何，尝试 xcap 几何: {e:#}");
            enumerate_xcap_monitors()
        })
        .context("无法枚举 GNOME Shell 截图显示器")?;

    let path = request_gnome_shell_screenshot().context("无法请求 GNOME Shell 截图")?;
    let bytes = std::fs::read(&path)
        .with_context(|| format!("无法读取 GNOME Shell 截图 {}", path.display()))?;
    let image = image::load_from_memory(&bytes)
        .context("无法解码 GNOME Shell 截图")?
        .to_rgba8();
    if let Err(e) = std::fs::remove_file(&path) {
        log::warn!("删除 GNOME Shell 临时截图失败 {}: {e}", path.display());
    }

    split_portal_screenshot(monitors, image)
}

fn enumerate_xcap_monitors() -> Result<Vec<MonitorInfo>> {
    let monitors = Monitor::all().context("无法枚举显示器")?;
    monitors.iter().map(monitor_info).collect()
}

#[cfg(target_os = "linux")]
fn enumerate_wayland_monitors() -> Result<Vec<MonitorInfo>> {
    let conn = wayland_connection()?;
    let monitors: Vec<_> = conn
        .get_all_outputs()
        .iter()
        .enumerate()
        .map(|(index, output)| monitor_info_from_wayland_output(index, output))
        .collect();

    if monitors.is_empty() {
        bail!("Wayland compositor 未报告输出");
    }

    Ok(monitors)
}

#[cfg(target_os = "linux")]
fn request_portal_screenshot(interactive: bool) -> Result<ashpd::desktop::screenshot::Screenshot> {
    let request = async {
        ashpd::desktop::screenshot::Screenshot::request()
            .interactive(interactive)
            .modal(false)
            .send()
            .await?
            .response()
    };

    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(request).context("Portal 截图请求失败"),
        Err(_) => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("无法创建 Portal 截图 runtime")?;
            runtime.block_on(request).context("Portal 截图请求失败")
        }
    }
}

#[cfg(target_os = "linux")]
fn request_gnome_shell_screenshot() -> Result<std::path::PathBuf> {
    let path = std::env::temp_dir()
        .join("clippy")
        .join(format!("gnome-shell-screenshot-{}.png", unique_suffix()));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("无法创建 {}", parent.display()))?;
    }

    let connection = zbus::blocking::Connection::session().context("无法连接 D-Bus")?;
    let proxy = zbus::blocking::Proxy::new(
        &connection,
        "org.gnome.Shell.Screenshot",
        "/org/gnome/Shell/Screenshot",
        "org.gnome.Shell.Screenshot",
    )
    .context("无法连接 org.gnome.Shell.Screenshot")?;

    let filename = path.to_string_lossy().to_string();
    let (success, used_filename): (bool, String) = proxy
        .call("Screenshot", &(false, false, filename.as_str()))
        .context("GNOME Shell Screenshot 调用失败")?;
    if !success {
        bail!("GNOME Shell Screenshot 返回 success=false");
    }

    let used_path = if used_filename.is_empty() {
        path
    } else {
        std::path::PathBuf::from(used_filename)
    };
    if !used_path.exists() {
        bail!(
            "GNOME Shell Screenshot 返回 {}，但文件不存在",
            used_path.display()
        );
    }

    Ok(used_path)
}

#[cfg(target_os = "linux")]
fn split_portal_screenshot(
    monitors: Vec<MonitorInfo>,
    image: RgbaImage,
) -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    if monitors.is_empty() {
        bail!("Portal 截图没有可映射显示器");
    }

    let image_width = image.width();
    let image_height = image.height();
    if image_width == 0 || image_height == 0 {
        bail!("Portal 截图为空");
    }

    let desktop = monitor_union(&monitors)?;
    let scale_x = image_width as f32 / desktop.width as f32;
    let scale_y = image_height as f32 / desktop.height as f32;
    if !scale_x.is_finite() || scale_x <= 0.0 || !scale_y.is_finite() || scale_y <= 0.0 {
        bail!("Portal 截图缩放无效: {scale_x}x{scale_y}");
    }

    let rgba = image.into_raw();
    let mut adjusted_monitors = Vec::with_capacity(monitors.len());
    let mut frames = Vec::with_capacity(monitors.len());

    for monitor in monitors {
        let crop = scaled_monitor_rect(
            &monitor.rect,
            &desktop,
            scale_x,
            scale_y,
            image_width,
            image_height,
        )?;
        let frame_rgba = crop_rgba(&rgba, image_width, crop)?;
        let scale_factor = portal_frame_scale_factor(&monitor, crop);
        let mut adjusted_monitor = monitor;
        adjusted_monitor.scale_factor = scale_factor;

        frames.push(FrozenFrame {
            monitor_id: adjusted_monitor.id,
            rgba: Arc::from(frame_rgba),
            width: crop.width,
            height: crop.height,
            scale_factor,
        });
        adjusted_monitors.push(adjusted_monitor);
    }

    Ok((adjusted_monitors, frames))
}

#[cfg(test)]
fn compose_desktop_image(
    monitors: &[MonitorInfo],
    frames: &[FrozenFrame],
) -> Result<(Vec<u8>, u32, u32)> {
    if monitors.is_empty() || frames.is_empty() {
        bail!("没有可用截图帧");
    }

    let x_mapper = AxisMapper::from_frames(monitors, frames, Axis::X)?;
    let y_mapper = AxisMapper::from_frames(monitors, frames, Axis::Y)?;
    let width = frames
        .iter()
        .filter_map(|frame| {
            let monitor = monitors.iter().find(|m| m.id == frame.monitor_id)?;
            let x = x_mapper.map(monitor.rect.x);
            Some(x.saturating_add(frame.width))
        })
        .max()
        .unwrap_or(1);
    let height = frames
        .iter()
        .filter_map(|frame| {
            let monitor = monitors.iter().find(|m| m.id == frame.monitor_id)?;
            let y = y_mapper.map(monitor.rect.y);
            Some(y.saturating_add(frame.height))
        })
        .max()
        .unwrap_or(1);

    if width == 0 || height == 0 {
        bail!("组合截图为空");
    }

    let mut canvas = ImageBuffer::from_pixel(width, height, image::Rgba([0, 0, 0, 0]));
    for frame in frames {
        let Some(monitor) = monitors.iter().find(|m| m.id == frame.monitor_id) else {
            continue;
        };
        let x = x_mapper.map(monitor.rect.x);
        let y = y_mapper.map(monitor.rect.y);
        let Some(frame_image) = RgbaImage::from_raw(frame.width, frame.height, frame.rgba.to_vec())
        else {
            log::warn!(
                "截图帧尺寸和像素数据不匹配，跳过 monitor {}",
                frame.monitor_id
            );
            continue;
        };
        image::imageops::overlay(&mut canvas, &frame_image, i64::from(x), i64::from(y));
    }

    Ok((canvas.into_raw(), width, height))
}

#[cfg(test)]
fn monitor_axis_bounds(monitor: &MonitorInfo, axis: Axis) -> (i32, u32) {
    match axis {
        Axis::X => (monitor.rect.x, monitor.rect.width),
        Axis::Y => (monitor.rect.y, monitor.rect.height),
    }
}

#[cfg(test)]
fn monitor_axis_overlaps(monitor: &MonitorInfo, axis: Axis, start: i32, end: i32) -> bool {
    let (monitor_start, monitor_length) = monitor_axis_bounds(monitor, axis);
    let monitor_end = monitor_start.saturating_add_unsigned(monitor_length);
    monitor_start < end && monitor_end > start
}

#[cfg(test)]
fn monitor_axis_scale(monitor: &MonitorInfo, frame: Option<&FrozenFrame>, axis: Axis) -> f32 {
    if let Some(frame) = frame {
        let (_, monitor_length) = monitor_axis_bounds(monitor, axis);
        let frame_length = match axis {
            Axis::X => frame.width,
            Axis::Y => frame.height,
        };
        if monitor_length > 0 && frame_length > 0 {
            return frame_length as f32 / monitor_length as f32;
        }
        if frame.scale_factor.is_finite() && frame.scale_factor > 0.0 {
            return frame.scale_factor;
        }
    }

    monitor.scale_factor
}

#[cfg(test)]
fn scaled_axis_length(value: i32, scale_factor: f32) -> u32 {
    if value <= 0 {
        return 0;
    }
    ((value as f32) * scale_factor.max(f32::EPSILON)).round() as u32
}

fn monitor_union(monitors: &[MonitorInfo]) -> Result<DesktopBounds> {
    let min_x = monitors
        .iter()
        .map(|monitor| monitor.rect.x)
        .min()
        .context("缺少显示器 x 坐标")?;
    let min_y = monitors
        .iter()
        .map(|monitor| monitor.rect.y)
        .min()
        .context("缺少显示器 y 坐标")?;
    let max_x = monitors
        .iter()
        .map(|monitor| monitor.rect.x as i64 + monitor.rect.width as i64)
        .max()
        .context("缺少显示器右边界")?;
    let max_y = monitors
        .iter()
        .map(|monitor| monitor.rect.y as i64 + monitor.rect.height as i64)
        .max()
        .context("缺少显示器下边界")?;

    let width = max_x - min_x as i64;
    let height = max_y - min_y as i64;
    if width <= 0 || height <= 0 {
        bail!("显示器组合区域为空");
    }

    Ok(DesktopBounds {
        x: min_x,
        y: min_y,
        width: width as u32,
        height: height as u32,
    })
}

#[cfg(target_os = "linux")]
fn scaled_monitor_rect(
    rect: &Rect,
    desktop: &DesktopBounds,
    scale_x: f32,
    scale_y: f32,
    image_width: u32,
    image_height: u32,
) -> Result<ImageRect> {
    let left = scaled_edge(rect.x, desktop.x, scale_x, image_width);
    let top = scaled_edge(rect.y, desktop.y, scale_y, image_height);
    let right = scaled_edge(
        rect.x.saturating_add_unsigned(rect.width),
        desktop.x,
        scale_x,
        image_width,
    );
    let bottom = scaled_edge(
        rect.y.saturating_add_unsigned(rect.height),
        desktop.y,
        scale_y,
        image_height,
    );

    if right <= left || bottom <= top {
        bail!("缩放后的显示器区域为空");
    }

    Ok(ImageRect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

#[cfg(target_os = "linux")]
fn scaled_edge(edge: i32, origin: i32, scale: f32, max: u32) -> u32 {
    (((edge - origin) as f32) * scale)
        .round()
        .clamp(0.0, max as f32) as u32
}

#[cfg(target_os = "linux")]
fn crop_rgba(rgba: &[u8], image_width: u32, crop: ImageRect) -> Result<Vec<u8>> {
    let row_bytes = crop.width as usize * 4;
    let mut out = Vec::with_capacity(row_bytes * crop.height as usize);
    for row in 0..crop.height {
        let y = crop.y + row;
        let start = (y * image_width + crop.x) as usize * 4;
        let end = start + row_bytes;
        let slice = rgba
            .get(start..end)
            .with_context(|| format!("Portal 截图裁剪第 {row} 行越界"))?;
        out.extend_from_slice(slice);
    }
    Ok(out)
}

#[cfg(target_os = "linux")]
fn portal_frame_scale_factor(monitor: &MonitorInfo, crop: ImageRect) -> f32 {
    let width_scale = if monitor.rect.width > 0 {
        crop.width as f32 / monitor.rect.width as f32
    } else {
        0.0
    };
    let height_scale = if monitor.rect.height > 0 {
        crop.height as f32 / monitor.rect.height as f32
    } else {
        0.0
    };
    let scale = width_scale.max(height_scale);

    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        monitor.scale_factor
    }
}

fn monitor_info(mon: &Monitor) -> Result<MonitorInfo> {
    let id = mon.id().context("无法读取显示器 id")?;
    let scale = mon.scale_factor().context("无法读取显示器缩放")?;
    let x = mon.x().context("无法读取显示器 x 坐标")?;
    let y = mon.y().context("无法读取显示器 y 坐标")?;
    let width = mon.width().context("无法读取显示器宽度")?;
    let height = mon.height().context("无法读取显示器高度")?;

    Ok(MonitorInfo {
        id,
        rect: Rect {
            x,
            y,
            width,
            height,
        },
        scale_factor: scale,
    })
}

#[cfg(target_os = "linux")]
fn monitor_info_from_wayland_output(
    index: usize,
    output: &libwayshot_xcap::output::OutputInfo,
) -> MonitorInfo {
    let region = output.logical_region.inner;

    MonitorInfo {
        id: stable_wayland_output_id(index, output),
        rect: Rect {
            x: region.position.x,
            y: region.position.y,
            width: region.size.width,
            height: region.size.height,
        },
        scale_factor: wayland_output_scale_factor(output),
    }
}

#[cfg(target_os = "linux")]
fn stable_wayland_output_id(index: usize, output: &libwayshot_xcap::output::OutputInfo) -> u32 {
    let key = if output.name.is_empty() {
        output.description.as_str()
    } else {
        output.name.as_str()
    };
    let mut hash = 0x811c9dc5_u32;
    for byte in key
        .as_bytes()
        .iter()
        .copied()
        .chain([0xff])
        .chain(index.to_le_bytes())
    {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

#[cfg(target_os = "linux")]
fn wayland_output_scale_factor(output: &libwayshot_xcap::output::OutputInfo) -> f32 {
    let logical = output.logical_region.inner.size;
    let physical = output.physical_size;

    let width_scale = if logical.width > 0 {
        physical.width as f32 / logical.width as f32
    } else {
        0.0
    };
    let height_scale = if logical.height > 0 {
        physical.height as f32 / logical.height as f32
    } else {
        0.0
    };
    let scale = width_scale.max(height_scale);

    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

#[cfg(target_os = "linux")]
fn wayland_connection() -> Result<libwayshot_xcap::WayshotConnection> {
    std::panic::catch_unwind(libwayshot_xcap::WayshotConnection::new)
        .map_err(|_| anyhow!("Wayland 输出发现 panic"))?
        .context("无法连接 Wayland compositor")
}

#[cfg(target_os = "linux")]
fn is_wayland_session() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|session| session.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false)
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

#[cfg(target_os = "linux")]
fn portal_screenshot_uri_to_path(uri: &str) -> Result<std::path::PathBuf> {
    let rest = uri
        .strip_prefix("file://")
        .ok_or_else(|| anyhow!("Portal 截图 URI 不是本地 file URI: {uri}"))?;

    let path = if let Some(path) = rest.strip_prefix('/') {
        format!("/{path}")
    } else if let Some(path) = rest.strip_prefix("localhost/") {
        format!("/{path}")
    } else {
        let authority = rest.split('/').next().unwrap_or(rest);
        bail!("Portal 截图 URI authority 不支持 `{authority}`: {uri}");
    };

    let decoded = urlencoding::decode(&path)
        .with_context(|| format!("Portal 截图 URI path 不是合法 UTF-8: {uri}"))?;
    Ok(std::path::PathBuf::from(decoded.into_owned()))
}

fn unique_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{millis}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_file_uri_decodes_to_local_path() {
        let path = portal_screenshot_uri_to_path("file:///tmp/Clippy%20Shot.png").unwrap();
        assert_eq!(path, std::path::PathBuf::from("/tmp/Clippy Shot.png"));
    }

    #[test]
    fn encode_png_compresses_simple_screenshot_data() {
        let width = 64;
        let height = 64;
        let mut rgba = Vec::with_capacity(width * height * 4);
        for _ in 0..(width * height) {
            rgba.extend_from_slice(&[240, 240, 240, 255]);
        }

        let png = encode_png(&rgba, width as u32, height as u32).unwrap();

        assert!(png.len() < rgba.len() / 4);
        assert_eq!(png_dimensions(&png).unwrap(), (width as u32, height as u32));
    }

    #[test]
    fn portal_mapping_splits_horizontal_monitors() {
        let monitors = vec![
            MonitorInfo {
                id: 1,
                rect: Rect {
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 50,
                },
                scale_factor: 1.0,
            },
            MonitorInfo {
                id: 2,
                rect: Rect {
                    x: 100,
                    y: 0,
                    width: 100,
                    height: 50,
                },
                scale_factor: 1.0,
            },
        ];
        let desktop = monitor_union(&monitors).unwrap();

        assert_eq!(
            scaled_monitor_rect(&monitors[0].rect, &desktop, 2.0, 2.0, 400, 100).unwrap(),
            ImageRect {
                x: 0,
                y: 0,
                width: 200,
                height: 100,
            }
        );
        assert_eq!(
            scaled_monitor_rect(&monitors[1].rect, &desktop, 2.0, 2.0, 400, 100).unwrap(),
            ImageRect {
                x: 200,
                y: 0,
                width: 200,
                height: 100,
            }
        );
    }

    #[test]
    fn compose_places_left_1x_right_2x_monitors_without_gap() {
        let monitors = horizontal_monitors(1.0, 2.0);
        let frames = vec![
            solid_frame(1, 100, 50, 1.0, [255, 0, 0, 255]),
            solid_frame(2, 200, 100, 2.0, [0, 0, 255, 255]),
        ];

        let (rgba, width, height) = compose_desktop_image(&monitors, &frames).unwrap();

        assert_eq!((width, height), (300, 100));
        assert_eq!(pixel_at(&rgba, width, 99, 25), [255, 0, 0, 255]);
        assert_eq!(pixel_at(&rgba, width, 100, 25), [0, 0, 255, 255]);
        assert_eq!(pixel_at(&rgba, width, 299, 25), [0, 0, 255, 255]);
    }

    #[test]
    fn compose_places_left_2x_right_1x_monitors_without_overlap() {
        let monitors = horizontal_monitors(2.0, 1.0);
        let frames = vec![
            solid_frame(1, 200, 100, 2.0, [255, 0, 0, 255]),
            solid_frame(2, 100, 50, 1.0, [0, 0, 255, 255]),
        ];

        let (rgba, width, height) = compose_desktop_image(&monitors, &frames).unwrap();

        assert_eq!((width, height), (300, 100));
        assert_eq!(pixel_at(&rgba, width, 199, 25), [255, 0, 0, 255]);
        assert_eq!(pixel_at(&rgba, width, 200, 25), [0, 0, 255, 255]);
        assert_eq!(pixel_at(&rgba, width, 299, 25), [0, 0, 255, 255]);
    }

    fn horizontal_monitors(left_scale: f32, right_scale: f32) -> Vec<MonitorInfo> {
        vec![
            MonitorInfo {
                id: 1,
                rect: Rect {
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 50,
                },
                scale_factor: left_scale,
            },
            MonitorInfo {
                id: 2,
                rect: Rect {
                    x: 100,
                    y: 0,
                    width: 100,
                    height: 50,
                },
                scale_factor: right_scale,
            },
        ]
    }

    fn solid_frame(
        monitor_id: u32,
        width: u32,
        height: u32,
        scale_factor: f32,
        color: [u8; 4],
    ) -> FrozenFrame {
        FrozenFrame {
            monitor_id,
            rgba: Arc::from(solid_rgba(width, height, color)),
            width,
            height,
            scale_factor,
        }
    }

    fn solid_rgba(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..width * height {
            rgba.extend_from_slice(&color);
        }
        rgba
    }

    fn pixel_at(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let start = ((y * width + x) * 4) as usize;
        [
            rgba[start],
            rgba[start + 1],
            rgba[start + 2],
            rgba[start + 3],
        ]
    }
}
