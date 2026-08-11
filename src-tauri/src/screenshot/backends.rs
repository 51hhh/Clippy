use super::{DesktopBounds, FrozenFrame, ImageRect, MonitorInfo, Rect};
use anyhow::{anyhow, bail, Context, Result};
use image::RgbaImage;
use std::sync::Arc;
use xcap::Monitor;

#[cfg(target_os = "linux")]
pub(super) struct TemporaryScreenshotFile {
    path: std::path::PathBuf,
}

#[cfg(target_os = "linux")]
impl TemporaryScreenshotFile {
    pub(super) fn new(path: std::path::PathBuf) -> Self {
        Self { path }
    }

    pub(super) fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub(super) fn replace_path(&mut self, path: std::path::PathBuf) {
        self.remove_current();
        self.path = path;
    }

    fn remove_current(&self) {
        if let Err(error) = std::fs::remove_file(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!(
                    "删除 GNOME Shell 临时截图失败 {}: {error}",
                    self.path.display()
                );
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for TemporaryScreenshotFile {
    fn drop(&mut self) {
        self.remove_current();
    }
}

#[cfg(target_os = "linux")]
pub(super) fn capture_all_monitors(
    allow_interactive_portal: bool,
) -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    if is_wayland_session() {
        match capture_all_wayland_monitors() {
            Ok(result) => return Ok(result),
            Err(e) => log::warn!("Wayland wlroots 截图失败，回退到 Portal: {e:#}"),
        }

        match capture_all_portal_monitors(allow_interactive_portal) {
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
pub(super) fn capture_all_monitors(
    _allow_interactive_portal: bool,
) -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
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
fn capture_all_portal_monitors(
    allow_interactive_portal: bool,
) -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    let monitors = enumerate_wayland_monitors()
        .or_else(|e| {
            log::warn!("Portal 截图无法复用 Wayland 几何，尝试 xcap 几何: {e:#}");
            enumerate_xcap_monitors()
        })
        .context("无法枚举 Portal 截图显示器")?;

    let screenshot = request_portal_screenshot(false)
        .or_else(|error| {
            if allow_interactive_portal {
                log::warn!("非交互 Portal 截图失败，尝试用户触发的交互模式: {error:#}");
                request_portal_screenshot(true)
            } else {
                Err(error)
            }
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

    let screenshot = request_gnome_shell_screenshot().context("无法请求 GNOME Shell 截图")?;
    let bytes = std::fs::read(screenshot.path())
        .with_context(|| format!("无法读取 GNOME Shell 截图 {}", screenshot.path().display()))?;
    let image = image::load_from_memory(&bytes)
        .context("无法解码 GNOME Shell 截图")?
        .to_rgba8();

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
fn request_gnome_shell_screenshot() -> Result<TemporaryScreenshotFile> {
    let directory = std::env::temp_dir().join("clippy");
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("无法创建 {}", directory.display()))?;
    crate::private_files::restrict_directory(&directory)
        .with_context(|| format!("无法收紧 {} 的目录权限", directory.display()))?;
    let path = directory.join(format!("gnome-shell-screenshot-{}.png", unique_suffix()));
    let mut screenshot = TemporaryScreenshotFile::new(path);

    let connection = zbus::blocking::Connection::session().context("无法连接 D-Bus")?;
    let proxy = zbus::blocking::Proxy::new(
        &connection,
        "org.gnome.Shell.Screenshot",
        "/org/gnome/Shell/Screenshot",
        "org.gnome.Shell.Screenshot",
    )
    .context("无法连接 org.gnome.Shell.Screenshot")?;

    let filename = screenshot.path().to_string_lossy().to_string();
    let (success, used_filename): (bool, String) = proxy
        .call("Screenshot", &(false, false, filename.as_str()))
        .context("GNOME Shell Screenshot 调用失败")?;
    if !success {
        bail!("GNOME Shell Screenshot 返回 success=false");
    }

    let used_path = if used_filename.is_empty() {
        screenshot.path().to_path_buf()
    } else {
        std::path::PathBuf::from(used_filename)
    };
    validate_gnome_shell_screenshot_path(&directory, &used_path)?;
    if used_path != screenshot.path() {
        screenshot.replace_path(used_path);
    }
    if !screenshot.path().exists() {
        bail!(
            "GNOME Shell Screenshot 返回 {}，但文件不存在",
            screenshot.path().display()
        );
    }
    crate::private_files::restrict_file(screenshot.path()).with_context(|| {
        format!(
            "无法收紧 GNOME Shell 临时截图权限 {}",
            screenshot.path().display()
        )
    })?;

    Ok(screenshot)
}

#[cfg(target_os = "linux")]
pub(super) fn validate_gnome_shell_screenshot_path(
    directory: &std::path::Path,
    path: &std::path::Path,
) -> Result<()> {
    if path.parent() != Some(directory) {
        bail!("GNOME Shell Screenshot 返回的文件不在私有临时目录中");
    }
    Ok(())
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

pub(super) fn monitor_union(monitors: &[MonitorInfo]) -> Result<DesktopBounds> {
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
pub(super) fn scaled_monitor_rect(
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
pub(super) fn portal_screenshot_uri_to_path(uri: &str) -> Result<std::path::PathBuf> {
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
