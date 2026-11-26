use anyhow::{ensure, Result};
use clippy_core::clipboard::validate_image_layout;
use image::RgbaImage;
use std::{
    sync::{
        mpsc::{self, SyncSender},
        Arc,
    },
    thread::{self, JoinHandle},
};

#[cfg(target_os = "linux")]
pub mod gnome;
#[cfg(any(target_os = "linux", target_os = "macos", test))]
mod latest;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "macos", test))]
mod macos_frame;
#[cfg(target_os = "linux")]
pub mod pipewire_frame;
#[cfg(target_os = "linux")]
mod portal;
#[cfg(target_os = "linux")]
mod screencast;
#[cfg(target_os = "linux")]
mod screencast_stream;
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
mod wayland_outputs;
#[cfg(target_os = "linux")]
pub mod wayland_scroll;
#[cfg(target_os = "linux")]
mod x11;

#[cfg(target_os = "linux")]
pub use portal::system_screenshot;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesktopRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone)]
pub struct FrozenPixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

impl From<RgbaImage> for FrozenPixels {
    fn from(image: RgbaImage) -> Self {
        Self {
            width: image.width(),
            height: image.height(),
            rgba: Arc::from(image.into_raw()),
        }
    }
}

pub struct FrozenOutput {
    pub name: String,
    pub geometry: DesktopRect,
    pub pixels: FrozenPixels,
}

pub struct FrozenCapture {
    pub outputs: Vec<FrozenOutput>,
    pub output_count: usize,
    pub source: CaptureSource,
}

impl FrozenCapture {
    fn direct(outputs: Vec<FrozenOutput>) -> Self {
        Self {
            output_count: outputs.len(),
            outputs,
            source: CaptureSource::default(),
        }
    }
}

#[derive(Default)]
pub struct CaptureSource(#[cfg(target_os = "linux")] Option<wayland::PortalHandoff>);

impl CaptureSource {
    pub fn uses_portal(&self) -> bool {
        #[cfg(target_os = "linux")]
        return self.0.is_some();
        #[cfg(not(target_os = "linux"))]
        false
    }

    pub fn overlay_closed(&mut self, _closed: std::time::Instant) -> Result<()> {
        #[cfg(target_os = "linux")]
        if let Some(handoff) = &mut self.0 {
            handoff.overlay_closed(_closed)?;
        }
        Ok(())
    }
}

pub struct Region {
    pub output: String,
    pub geometry: DesktopRect,
    pub source_size: (u32, u32),
    pub bounds: [f32; 4],
    pub source: CaptureSource,
}

#[derive(Debug)]
pub struct ScreenSharingCancelled;

impl std::fmt::Display for ScreenSharingCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Screen sharing cancelled")
    }
}

impl std::error::Error for ScreenSharingCancelled {}

#[derive(Clone, Default)]
pub struct RegionCancellation {
    #[cfg(target_os = "linux")]
    signal: tokio::sync::watch::Sender<bool>,
}

impl RegionCancellation {
    pub fn cancel(&self) {
        #[cfg(target_os = "linux")]
        self.signal.send_replace(true);
    }

    #[cfg(target_os = "linux")]
    async fn cancelled(&self) {
        let mut signal = self.signal.subscribe();
        let _ = signal.wait_for(|cancelled| *cancelled).await;
    }

    #[cfg(target_os = "linux")]
    fn check_active(&self) -> Result<()> {
        ensure!(!*self.signal.borrow(), ScreenSharingCancelled);
        Ok(())
    }
}

type Reader<'a> = dyn FnMut() -> Result<RgbaImage> + 'a;

type FrameReply = SyncSender<Result<RgbaImage>>;

pub struct RegionReader {
    commands: Option<SyncSender<FrameReply>>,
    thread: Option<JoinHandle<()>>,
}

impl RegionReader {
    pub fn open(region: Region, cancellation: RegionCancellation) -> Result<Self> {
        let (commands, requests) = mpsc::sync_channel::<FrameReply>(1);
        let (ready, started) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("clippy-region-source".into())
            .spawn(move || {
                let mut ready = Some(ready);
                let result = with_region_reader(region, &cancellation, |read| {
                    ready.take().expect("采集源只打开一次").send(Ok(()))?;
                    while let Ok(response) = requests.recv() {
                        if response.send(read()).is_err() {
                            break;
                        }
                    }
                    Ok(())
                });
                if let Some(ready) = ready {
                    let _ = ready.send(result);
                }
            })?;
        if let Err(error) = started.recv()? {
            let _ = thread.join();
            return Err(error);
        }
        Ok(Self {
            commands: Some(commands),
            thread: Some(thread),
        })
    }

    pub fn read(&mut self) -> Result<RgbaImage> {
        let (reply, image) = mpsc::sync_channel(1);
        self.commands
            .as_ref()
            .expect("采集源处于读状态")
            .send(reply)?;
        image.recv()?
    }
}

impl Drop for RegionReader {
    fn drop(&mut self) {
        self.commands.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn with_region_reader<T>(
    region: Region,
    _cancellation: &RegionCancellation,
    consume: impl FnOnce(&mut Reader<'_>) -> Result<T>,
) -> Result<T> {
    let Region {
        output,
        geometry,
        source_size,
        bounds: [left, top, right, bottom],
        source: _source,
    } = region;
    #[cfg(target_os = "linux")]
    if let Some(handoff) = _source.0 {
        return handoff.with_region(
            &output,
            geometry,
            source_size,
            [left, top, right, bottom],
            _cancellation,
            consume,
        );
    }
    #[cfg(target_os = "linux")]
    if crate::session::is_wayland() {
        if crate::session::is_gnome() {
            let (area, crop, expected_size) =
                gnome_region(geometry, source_size, [left, top, right, bottom])?;
            return gnome::with_area(output, area, |read| {
                consume(&mut || {
                    let frame = read()?;
                    ensure!(
                        (frame.width, frame.height) == expected_size,
                        "Capture output changed size: expected {}x{}, received {}x{}",
                        expected_size.0,
                        expected_size.1,
                        frame.width,
                        frame.height
                    );
                    crop_rgba(&frame.rgba, frame.width, frame.height, crop)
                })
            });
        }
        return wayland::with_region(
            &output,
            geometry,
            source_size,
            [left, top, right, bottom],
            _cancellation,
            consume,
        );
    }
    #[cfg(target_os = "linux")]
    {
        let rectangle = x11_region_rect(geometry, [left, top, right, bottom])?;
        let source = x11::Source::open()?;
        consume(&mut || source.read(rectangle))
    }
    #[cfg(target_os = "windows")]
    {
        let monitor = resolve_native_monitor(&output, geometry)?;
        let pixels = pixel_rect(source_size.0, source_size.1, [left, top, right, bottom])?;
        let mut source = monitor.wgc_region_reader(source_size, pixels)?;
        consume(&mut || source.read().map_err(Into::into))
    }
    #[cfg(target_os = "macos")]
    {
        let monitor = resolve_native_monitor(&output, geometry)?;
        let bounds = [left, top, right, bottom];
        if macos::stream_supported() {
            let mut source = macos::ScreenStream::open(monitor.id()?, source_size, bounds)?;
            return consume(&mut || source.read());
        }
        consume(&mut || {
            let image = monitor.capture_image()?;
            ensure!(
                image.dimensions() == source_size,
                "Capture output changed size"
            );
            crop_rgba(image.as_raw(), source_size.0, source_size.1, bounds)
        })
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn resolve_native_monitor(output: &str, geometry: DesktopRect) -> Result<xcap::Monitor> {
    let mut name_found = false;
    for monitor in xcap::Monitor::all()? {
        if monitor.name().is_ok_and(|name| name == output) {
            name_found = true;
            if (monitor.x()?, monitor.y()?) == (geometry.x, geometry.y) {
                return Ok(monitor);
            }
        }
    }
    ensure!(name_found, "Capture output is unavailable");
    anyhow::bail!("Capture output changed position")
}

#[cfg(target_os = "linux")]
fn gnome_region(
    geometry: DesktopRect,
    source_size: (u32, u32),
    bounds: [f32; 4],
) -> Result<(DesktopRect, [f32; 4], (u32, u32))> {
    // 缩放屏保留整屏流，避免把源像素边界再次舍入为 Mutter 的逻辑坐标。
    if source_size != (geometry.width, geometry.height) {
        return Ok((geometry, bounds, source_size));
    }
    let area = offset_rect(
        geometry,
        pixel_rect(geometry.width, geometry.height, bounds)?,
    )?;
    Ok((area, [0.0, 0.0, 1.0, 1.0], (area.width, area.height)))
}

#[cfg(target_os = "linux")]
pub(crate) fn x11_region_rect(geometry: DesktopRect, bounds: [f32; 4]) -> Result<DesktopRect> {
    offset_rect(
        geometry,
        pixel_rect(geometry.width, geometry.height, bounds)?,
    )
}

#[cfg(target_os = "linux")]
fn offset_rect(geometry: DesktopRect, [left, top, right, bottom]: [u32; 4]) -> Result<DesktopRect> {
    Ok(DesktopRect {
        x: geometry
            .x
            .checked_add(i32::try_from(left)?)
            .ok_or_else(|| anyhow::anyhow!("Capture region x overflow"))?,
        y: geometry
            .y
            .checked_add(i32::try_from(top)?)
            .ok_or_else(|| anyhow::anyhow!("Capture region y overflow"))?,
        width: right - left,
        height: bottom - top,
    })
}

impl FrozenPixels {
    pub fn crop(&self, left: f32, top: f32, right: f32, bottom: f32) -> Result<RgbaImage> {
        crop_rgba(
            &self.rgba,
            self.width,
            self.height,
            [left, top, right, bottom],
        )
    }
}

/// 裁剪已验证布局的连续 RGBA 帧，使采集和 UI 缓冲共享同一套映射。
pub fn crop_rgba(rgba: &[u8], width: u32, height: u32, bounds: [f32; 4]) -> Result<RgbaImage> {
    let [left, top, right, bottom] = pixel_rect(width, height, bounds)?;
    let mut pixels = Vec::with_capacity((right - left) as usize * (bottom - top) as usize * 4);
    let stride = width as usize * 4;
    for y in top..bottom {
        let row = y as usize * stride;
        pixels.extend_from_slice(&rgba[row + left as usize * 4..row + right as usize * 4]);
    }
    Ok(RgbaImage::from_raw(right - left, bottom - top, pixels).expect("已经按 RGBA 选区逐行拷贝"))
}

/// 覆盖层坐标按窗口宽高归一化，选区始终映射到冻结帧的源像素。
pub fn pixel_rect(width: u32, height: u32, bounds: [f32; 4]) -> Result<[u32; 4]> {
    ensure!(
        bounds.iter().all(|value| value.is_finite()),
        "Invalid screenshot selection"
    );
    let [x1, y1, x2, y2] = bounds.map(|value| value.clamp(0.0, 1.0));
    let left = (x1.min(x2) * width as f32).floor() as u32;
    let top = (y1.min(y2) * height as f32).floor() as u32;
    let right = (x1.max(x2) * width as f32).ceil() as u32;
    let bottom = (y1.max(y2) * height as f32).ceil() as u32;
    ensure!(
        x1 != x2 && y1 != y2 && right > left && bottom > top,
        "Empty screenshot selection"
    );
    Ok([left, top, right, bottom])
}

pub fn freeze() -> Result<Vec<FrozenOutput>> {
    #[cfg(target_os = "linux")]
    let outputs = if crate::session::is_wayland() {
        wayland::freeze()?
    } else {
        x11::freeze()?
    };
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let outputs = native_freeze()?;
    validate_outputs(&outputs)?;
    Ok(outputs)
}

// 回调交付冻结图后，后台线程继续持有可移交的 Portal 会话，直到选区确认或放弃。
pub fn freeze_selection(
    _cancellation: &RegionCancellation,
    publish: impl FnOnce(Result<FrozenCapture>),
) {
    #[cfg(target_os = "linux")]
    if crate::session::is_wayland() {
        return wayland::freeze_selection(_cancellation, publish);
    }
    publish(freeze().map(FrozenCapture::direct));
}

fn validate_outputs(outputs: &[FrozenOutput]) -> Result<()> {
    ensure!(!outputs.is_empty(), "No display available");
    for output in outputs {
        let pixels = &output.pixels;
        validate_image_layout(
            pixels.width as usize,
            pixels.height as usize,
            pixels.rgba.len(),
        )
        .map_err(anyhow::Error::msg)?;
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn native_freeze() -> Result<Vec<FrozenOutput>> {
    xcap::Monitor::all()?
        .iter()
        .map(|monitor| {
            let image = monitor.capture_image()?;
            Ok(FrozenOutput {
                name: monitor.name()?,
                geometry: DesktopRect {
                    x: monitor.x()?,
                    y: monitor.y()?,
                    width: image.width(),
                    height: image.height(),
                },
                pixels: image.into(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn crops_drag_in_either_direction_to_exact_source_pixels() {
        let picture = RgbaImage::from_fn(7, 11, |x, y| Rgba([x as u8, y as u8, 0, 255]));
        let pixels = FrozenPixels::from(picture);
        let crop = pixels.crop(0.7, 0.7, 0.2, -0.05).unwrap();
        assert_eq!(crop.dimensions(), (4, 8));
        assert_eq!(crop.get_pixel(0, 0).0, [1, 0, 0, 255]);
        assert_eq!(crop.get_pixel(3, 7).0, [4, 7, 0, 255]);
        assert!(pixels.crop(0.5, 0.4, 0.5, 0.7).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn gnome_region_maps_unscaled_pixels_and_preserves_scaled_cropping() {
        let output = DesktopRect {
            x: -1920,
            y: -1080,
            width: 1920,
            height: 1080,
        };
        let bounds = [0.75, 0.75, 0.25, 0.25];
        let (area, crop, frame_size) = gnome_region(output, (1920, 1080), bounds).unwrap();
        assert_eq!(
            area,
            DesktopRect {
                x: -1440,
                y: -810,
                width: 960,
                height: 540,
            }
        );
        assert_eq!(crop, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(frame_size, (960, 540));
        assert_eq!(
            gnome_region(output, (2880, 1620), bounds).unwrap(),
            (output, bounds, (2880, 1620)),
        );
    }
}
