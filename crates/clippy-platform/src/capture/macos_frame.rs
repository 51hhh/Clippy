use super::pixel_rect;
use anyhow::{ensure, Context, Result};
use image::RgbaImage;

pub(super) struct RegionLayout {
    pub points: [f64; 4],
    pub pixels: (u32, u32),
}

pub(super) fn region_layout(
    source: (u32, u32),
    logical: (f64, f64),
    bounds: [f32; 4],
) -> Result<RegionLayout> {
    ensure!(
        logical.0.is_finite() && logical.0 > 0.0 && logical.1.is_finite() && logical.1 > 0.0,
        "Invalid display size"
    );
    let [left, top, right, bottom] = pixel_rect(source.0, source.1, bounds)?;
    let pixels = (right - left, bottom - top);
    let x_scale = logical.0 / f64::from(source.0);
    let y_scale = logical.1 / f64::from(source.1);
    Ok(RegionLayout {
        points: [
            f64::from(left) * x_scale,
            f64::from(top) * y_scale,
            f64::from(pixels.0) * x_scale,
            f64::from(pixels.1) * y_scale,
        ],
        pixels,
    })
}

pub(super) fn bgra_span(width: u32, height: u32, stride: usize) -> Result<usize> {
    let row = width as usize * 4;
    ensure!(stride >= row, "Invalid ScreenCaptureKit row stride");
    stride
        .checked_mul((height - 1) as usize)
        .and_then(|bytes| bytes.checked_add(row))
        .filter(|bytes| *bytes <= isize::MAX as usize)
        .context("ScreenCaptureKit buffer size overflow")
}

pub(super) fn bgra_to_rgba(bytes: &[u8], (width, height): (u32, u32), stride: usize) -> RgbaImage {
    let row_bytes = width as usize * 4;
    let mut rgba = Vec::with_capacity(row_bytes * height as usize);
    for row in bytes.chunks(stride) {
        for bgra in row[..row_bytes].as_chunks::<4>().0 {
            rgba.extend_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]);
        }
    }
    RgbaImage::from_raw(width, height, rgba).expect("已经按选区逐行转换 BGRA")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_region_projects_retina_and_fractional_pixels_from_top_left() {
        let backwards = [0.75, 0.75, 0.25, 0.25];
        let retina = region_layout((12, 20), (6.0, 10.0), backwards).unwrap();
        assert_eq!(retina.points, [1.5, 2.5, 3.0, 5.0]);
        assert_eq!(retina.pixels, (6, 10));
        let scaled = region_layout((12, 20), (8.0, 16.0), backwards).unwrap();
        assert_eq!(scaled.points, [2.0, 4.0, 4.0, 8.0]);
        assert_eq!(scaled.pixels, (6, 10));
        assert!(region_layout((12, 20), (0.0, 16.0), backwards).is_err());
    }

    #[test]
    fn macos_bgra_readback_skips_row_padding_and_preserves_alpha() {
        let bytes = [1, 2, 3, 4, 90, 91, 92, 93, 5, 6, 7, 8];
        assert_eq!(bgra_span(1, 2, 8).unwrap(), bytes.len());
        assert_eq!(
            bgra_to_rgba(&bytes, (1, 2), 8).into_raw(),
            [3, 2, 1, 4, 7, 6, 5, 8]
        );
        assert!(bgra_span(2, 2, 4).is_err());
        assert!(bgra_span(2, 2, usize::MAX).is_err());
    }
}
