use anyhow::{ensure, Result};
use clippy_core::clipboard::validate_image_layout;
use image::RgbaImage;
use std::sync::Arc;

#[cfg(target_os = "linux")]
pub mod gnome;
#[cfg(target_os = "linux")]
pub mod pipewire_frame;
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
mod x11;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesktopRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

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

impl FrozenPixels {
    /// 覆盖层坐标以当前窗口实际宽高归一化，选区始终映射回冻结帧的源像素。
    pub fn crop(&self, left: f32, top: f32, right: f32, bottom: f32) -> Result<RgbaImage> {
        let [left, top, right, bottom] =
            pixel_rect(self.width, self.height, [left, top, right, bottom])?;
        let mut pixels = Vec::with_capacity((right - left) as usize * (bottom - top) as usize * 4);
        let stride = self.width as usize * 4;
        for y in top..bottom {
            let row = y as usize * stride;
            pixels.extend_from_slice(&self.rgba[row + left as usize * 4..row + right as usize * 4]);
        }
        Ok(RgbaImage::from_raw(right - left, bottom - top, pixels)
            .expect("已经按 RGBA 选区逐行拷贝"))
    }
}

fn pixel_rect(width: u32, height: u32, bounds: [f32; 4]) -> Result<[u32; 4]> {
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
    ensure!(!outputs.is_empty(), "No display available");
    for output in &outputs {
        let pixels = &output.pixels;
        validate_image_layout(
            pixels.width as usize,
            pixels.height as usize,
            pixels.rgba.len(),
        )
        .map_err(anyhow::Error::msg)?;
    }
    Ok(outputs)
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
}
