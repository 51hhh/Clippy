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

/// GNOME Wayland 的首选取像素路径：Mutter 的 PipeWire 屏幕流，不经过 PNG。
#[cfg(target_os = "linux")]
mod screencast;

/// 几何诊断报告：报障时把数字摊开，不含像素与窗口标题。
pub(crate) mod diagnostics;

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

/// 桌面逻辑点所在显示器的**真实**缩放：一个逻辑像素等于几个设备像素。
///
/// **不要拿 `Monitor::scale_factor()` 当它用。** WebKitGTK 走的是 GTK3，而 GTK3 不支持
/// `wp_fractional_scale_v1`，于是 Mutter 只能给它一个**整数**缩放：本机两块屏真实缩放是
/// 1.3333 与 1.5，GDK 报的都是 2（实测 `gdk_monitor_get_scale_factor` == 2），客户端按 2×
/// 画，Mutter 再把缓冲区缩到真实倍率。所以 Tauri 的 physical 尺寸是"2× 的缓冲区像素"，
/// 而不是屏幕上的设备像素。
///
/// 谁需要这个数：要把**图片像素**换算成 CSS 像素的地方（贴图窗口的内容尺寸）。
/// 按 GDK 的 2 算会让图片显示得比原来小三分之一，按 1 算（也就是"像素当 CSS 像素用"）
/// 会把图片拉大 1.3333 倍——后者就是"贴出来的截图比原来大一圈而且发糊"的成因。
///
/// 坐标是**桌面逻辑坐标**（与 `GetWindows`、截图选区同一坐标系；Tauri 的 physical
/// 除以 `scale_factor()` 就是它）。点不在任何输出里、或不是 Wayland 会话就返回 `None`，
/// 调用方退回 GDK 那个数——X11/其它平台上它本来就是真的。
#[cfg(target_os = "linux")]
pub(crate) fn desktop_scale_at(x: f64, y: f64) -> Option<f64> {
    let monitors = backends::enumerate_wayland_monitors()
        .inspect_err(|error| log::debug!("查询显示器真实缩放失败: {error:#}"))
        .ok()?;
    monitors
        .iter()
        .find(|monitor| {
            x >= monitor.rect.x as f64
                && y >= monitor.rect.y as f64
                && x < monitor.rect.x as f64 + monitor.rect.width as f64
                && y < monitor.rect.y as f64 + monitor.rect.height as f64
        })
        .map(|monitor| f64::from(monitor.scale_factor))
        .filter(|scale| *scale > 0.0)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn desktop_scale_at(_x: f64, _y: f64) -> Option<f64> {
    // 其它平台的 winit/GDK 报的就是真实缩放，调用方直接用它自己那份。
    None
}

/// 逐屏取画面要的那组区域：每块屏的逻辑矩形加它自己的缩放。
///
/// 只给诊断用（`capture::timing_diagnostics`）：那里要在**不走截图链路**的前提下拿到区域，
/// 好把逐屏那条路单独计时。真正跑截图的那条路在 `backends` 里自己枚举，因为它还要
/// 每块屏的 id，而且要把镜像屏并成一次请求。
#[cfg(all(test, target_os = "linux"))]
pub(crate) fn logical_monitor_areas() -> Result<Vec<crate::capture::CaptureArea>> {
    Ok(backends::enumerate_wayland_monitors()?
        .iter()
        .map(|monitor| crate::capture::CaptureArea {
            x: monitor.rect.x,
            y: monitor.rect.y,
            width: monitor.rect.width,
            height: monitor.rect.height,
            scale: f64::from(monitor.scale_factor),
        })
        .collect())
}

/// 读 PNG 的宽高，**只解文件头**。
///
/// 曾经这里是 `load_from_memory`：为了两个 u32 把整张图解成位图，1080p 要 19.6 ms，
/// 全屏多屏图更久（docs/bench-baseline.md）。而调用它的地方几乎都只想知道尺寸——
/// 贴图窗口算多大、来源表按尺寸预筛、保存前确认这是张 PNG。IHDR 就在头 33 字节里。
///
/// 代价是它不再顺带证明"整张图都能解出来"。需要那个保证的地方（信任边界上、
/// 前端提交进来的载荷）调 [`validate_png`]，把这笔开销花在明处。
pub fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png)
        .into_dimensions()
        .context("PNG 头解析失败")
}

/// 确认这串字节是**整张**都能解出来的 PNG，返回宽高。
///
/// 只有信任边界上该付这个钱：覆盖层提交的 base64 不可信，而后面的 copy/save/pin
/// 全都假设手里是合法 PNG，坏在这一步比坏在剪贴板里好。
pub fn validate_png(bytes: &[u8]) -> Result<(u32, u32)> {
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
use backends::{dedupe_monitor_areas, load_area_tile, split_portal_screenshot};
#[cfg(test)]
use backends::{
    monitor_union, normalize_monitor_geometry, portal_screenshot_uri_to_path, scaled_monitor_rect,
};

#[cfg(test)]
mod tests;
