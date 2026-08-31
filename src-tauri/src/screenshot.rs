//! 截图捕获与 PNG 编码
//!
//! Linux 捕获 fallback 参考 flashot（MIT）：优先 Wayland/wlroots，
//! 再尝试 XDG Portal、GNOME Shell，最后回退 xcap/XRandR。

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{
    codecs::png::{CompressionType, FilterType, PngEncoder},
    ExtendedColorType, GenericImageView, ImageEncoder,
};
use std::sync::Arc;

mod backends;
mod geometry_check;

/// fixture 的格式定义，诊断的输出侧与回归测试的输入侧共用。
#[cfg(target_os = "linux")]
mod layout_format;

/// 显示器配置即数据：一个 json 一种环境，见 `tests/fixtures/monitor-layouts/`。
#[cfg(all(test, target_os = "linux"))]
mod layout_fixtures;

#[cfg(test)]
mod test_geometry;

#[cfg(test)]
use test_geometry::compose_desktop_image;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// 捕获冻结帧。
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

use backends::capture_all_monitors;
#[cfg(all(test, target_os = "linux"))]
use backends::split_portal_screenshot;
#[cfg(test)]
use backends::{
    monitor_union, normalize_monitor_geometry, portal_screenshot_uri_to_path, scaled_monitor_rect,
};

#[cfg(test)]
mod tests;
