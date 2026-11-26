use super::DesktopRect;
use anyhow::{ensure, Context, Result};
use clippy_core::clipboard::validate_image_layout;
use image::RgbaImage;

#[derive(Clone, Copy)]
pub struct OutputGeometry {
    pub desktop: DesktopRect,
    pub pixels: (u32, u32),
}

impl OutputGeometry {
    pub fn desktop_point(self, [x, y]: [f32; 2]) -> [f64; 2] {
        let rect = self.desktop;
        [
            f64::from(rect.x) + f64::from(x) * f64::from(rect.width),
            f64::from(rect.y) + f64::from(y) * f64::from(rect.height),
        ]
    }

    fn scales(self) -> [f64; 2] {
        [
            f64::from(self.pixels.0) / f64::from(self.desktop.width),
            f64::from(self.pixels.1) / f64::from(self.desktop.height),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesktopSelection {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl DesktopSelection {
    pub fn between([x0, y0]: [f64; 2], [x1, y1]: [f64; 2]) -> Self {
        Self {
            left: x0.min(x1),
            top: y0.min(y1),
            right: x0.max(x1),
            bottom: y0.max(y1),
        }
    }

    pub fn intersect(self, rect: DesktopRect) -> Option<Self> {
        let x = f64::from(rect.x);
        let y = f64::from(rect.y);
        let clipped = Self {
            left: self.left.max(x),
            top: self.top.max(y),
            right: self.right.min(x + f64::from(rect.width)),
            bottom: self.bottom.min(y + f64::from(rect.height)),
        };
        (clipped.right > clipped.left && clipped.bottom > clipped.top).then_some(clipped)
    }

    pub fn relative_bounds(self, rect: DesktopRect) -> [f32; 4] {
        let x = f64::from(rect.x);
        let y = f64::from(rect.y);
        let w = f64::from(rect.width);
        let h = f64::from(rect.height);
        [
            ((self.left - x) / w) as f32,
            ((self.top - y) / h) as f32,
            ((self.right - x) / w) as f32,
            ((self.bottom - y) / h) as f32,
        ]
    }

    pub fn composition(
        self,
        outputs: impl IntoIterator<Item = OutputGeometry>,
        pixel_limit: u64,
    ) -> Result<Composition> {
        let scale = outputs
            .into_iter()
            .filter(|output| self.intersect(output.desktop).is_some())
            .flat_map(OutputGeometry::scales)
            .reduce(f64::max)
            .context("Selection does not intersect a display")?;
        let [left, top, right, bottom] = self.sampled_bounds(scale);
        let width = u32::try_from(right - left)?;
        let height = u32::try_from(bottom - top)?;
        let pixels = u64::from(width) * u64::from(height);
        ensure!(pixels <= pixel_limit, "Screenshot selection is too large");
        let bytes = usize::try_from(pixels.checked_mul(4).context("Screenshot is too large")?)?;
        validate_image_layout(width as usize, height as usize, bytes)
            .map_err(anyhow::Error::msg)?;
        Ok(Composition {
            selection: self,
            scale,
            left,
            top,
            width,
            height,
        })
    }

    fn sampled_bounds(self, scale: f64) -> [i64; 4] {
        // 按目标像素中心做半开区间归属，共用边界的两块屏不会多出一列也不会留缝。
        [self.left, self.top, self.right, self.bottom]
            .map(|edge| (edge * scale - 0.5).ceil() as i64)
    }
}

pub struct Frame<'a> {
    pub output: OutputGeometry,
    pub rgba: &'a [u8],
}

#[derive(Clone, Copy, Debug)]
pub struct Composition {
    selection: DesktopSelection,
    scale: f64,
    left: i64,
    top: i64,
    width: u32,
    height: u32,
}

impl Composition {
    pub fn intersects(self, output: DesktopRect) -> bool {
        self.selection.intersect(output).is_some()
    }

    pub fn size(self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn desktop_point(self, [x, y]: [f32; 2]) -> [f64; 2] {
        // 识别点以合成帧的像素为原点，与 paint 使用同一像素中心取样。
        [
            (self.left as f64 + f64::from(x) + 0.5) / self.scale,
            (self.top as f64 + f64::from(y) + 0.5) / self.scale,
        ]
    }

    pub fn render<'a>(self, frames: impl DoubleEndedIterator<Item = Frame<'a>>) -> RgbaImage {
        let mut image = RgbaImage::new(self.width, self.height);
        let canvas: &mut [u8] = image.as_mut();
        // 镜像或相交输出按冻结顺序取前者。其余未覆盖的坐标保留透明像素。
        for frame in frames.rev() {
            let Some(area) = self.selection.intersect(frame.output.desktop) else {
                continue;
            };
            self.paint(canvas, frame, area);
        }
        image
    }

    fn paint(self, canvas: &mut [u8], frame: Frame<'_>, area: DesktopSelection) {
        let [left, top, right, bottom] = area.sampled_bounds(self.scale);
        if left == right || top == bottom {
            return;
        }
        let rect = frame.output.desktop;
        let (width, height) = frame.output.pixels;
        let [sx, sy] = frame.output.scales().map(|scale| scale / self.scale);
        let origin_x = f64::from(rect.x) * self.scale;
        let origin_y = f64::from(rect.y) * self.scale;
        let column = |x| sample(x, origin_x, sx, width);
        let columns = (sx != 1.0).then(|| (left..right).map(column).collect::<Vec<_>>());
        let row_bytes = (right - left) as usize * 4;
        for y in top..bottom {
            let source_y = sample(y, origin_y, sy, height);
            let source_start = source_y * width as usize * 4;
            let target_start =
                ((y - self.top) as usize * self.width as usize + (left - self.left) as usize) * 4;
            let row = &mut canvas[target_start..target_start + row_bytes];
            if let Some(columns) = &columns {
                for (pixel, &source_x) in row.as_chunks_mut::<4>().0.iter_mut().zip(columns) {
                    let offset = source_start + source_x * 4;
                    pixel.copy_from_slice(&frame.rgba[offset..offset + 4]);
                }
            } else {
                let offset = source_start + column(left) * 4;
                row.copy_from_slice(&frame.rgba[offset..offset + row_bytes]);
            }
        }
    }
}

fn sample(at: i64, origin: f64, ratio: f64, length: u32) -> usize {
    let pixel = ((at as f64 + 0.5 - origin) * ratio).floor() as usize;
    pixel.min(length as usize - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn output(x: i32, y: i32, width: u32, height: u32, pixels: (u32, u32)) -> OutputGeometry {
        OutputGeometry {
            desktop: DesktopRect {
                x,
                y,
                width,
                height,
            },
            pixels,
        }
    }

    fn fixture(output: OutputGeometry, color: u8) -> RgbaImage {
        RgbaImage::from_fn(output.pixels.0, output.pixels.1, |x, y| {
            Rgba([color, x as u8, y as u8, 255])
        })
    }

    #[test]
    fn negative_origin_reverse_drag_and_gaps_preserve_source_pixels() {
        let left = output(-3, -1, 3, 4, (3, 4));
        let right = output(0, 1, 2, 3, (2, 3));
        let forward = DesktopSelection::between([-2.0, 0.0], [2.0, 4.0]);
        let reverse = DesktopSelection::between([2.0, 4.0], [-2.0, 0.0]);
        assert_eq!(forward, reverse);
        let layers = [fixture(left, 42), fixture(right, 83)];
        let composition = reverse.composition([left, right], 24).unwrap();
        let image = composition.render([left, right].into_iter().zip(&layers).map(
            |(output, source)| Frame {
                output,
                rgba: source.as_raw(),
            },
        ));
        assert_eq!(image.dimensions(), (4, 4));
        for (x, y, pixel) in image.enumerate_pixels() {
            let expected = if x < 2 && y < 3 {
                *layers[0].get_pixel(x + 1, y + 1)
            } else if x >= 2 && y >= 1 {
                *layers[1].get_pixel(x - 2, y - 1)
            } else {
                Rgba([0, 0, 0, 0])
            };
            assert_eq!(*pixel, expected, "像素 ({x}, {y})");
        }
    }

    #[test]
    fn highest_density_preserves_a_rotated_output_and_repeats_low_density_samples() {
        let left = output(-2, 0, 2, 2, (2, 2));
        let portrait = output(0, -1, 1, 3, (2, 6));
        let layout = [left, portrait];
        let layers = [fixture(left, 1), fixture(portrait, 2)];
        let area = DesktopSelection::between([-1.5, 0.0], [1.0, 2.0]);
        let composition = area.composition(layout, 100).unwrap();
        let image =
            composition.render(
                layout
                    .into_iter()
                    .zip(&layers)
                    .map(|(output, source)| Frame {
                        output,
                        rgba: source.as_raw(),
                    }),
            );
        assert_eq!(image.dimensions(), (5, 4));
        assert_eq!(composition.desktop_point([0.0, 0.0]), [-1.25, 0.25]);
        assert_eq!(composition.desktop_point([4.0, 3.0]), [0.75, 1.75]);
        for (x, y, pixel) in image.enumerate_pixels() {
            let expected = if x < 3 {
                layers[0].get_pixel(u32::from(x > 0), y / 2)
            } else {
                layers[1].get_pixel(x - 3, y + 2)
            };
            assert_eq!(pixel, expected, "像素 ({x}, {y})");
        }
    }

    #[test]
    fn fractional_shared_edge_is_owned_by_exactly_one_output() {
        let left = output(-1, 0, 2, 2, (2, 2));
        let right = output(1, 0, 2, 2, (3, 3));
        let layers = [fixture(left, 1), fixture(right, 2)];
        let area = DesktopSelection::between([-1.0, 0.0], [3.0, 2.0]);
        let layout = [left, right];
        let image =
            area.composition(layout, 18)
                .unwrap()
                .render(
                    layout
                        .into_iter()
                        .zip(&layers)
                        .map(|(output, source)| Frame {
                            output,
                            rgba: source.as_raw(),
                        }),
                );
        assert_eq!(image.dimensions(), (6, 3));
        assert!(image
            .enumerate_pixels()
            .all(|(x, _, pixel)| { pixel[0] == if x < 3 { 1 } else { 2 } && pixel[3] == 255 }));
    }

    #[test]
    fn rejects_a_selection_above_the_existing_pixel_budget() {
        let area = DesktopSelection::between([0.0, 0.0], [7680.0, 4320.0]);
        let high_dpi = output(0, 0, 7680, 4320, (15360, 8640));
        assert!(area.composition([high_dpi], 32 * 1024 * 1024).is_err());
    }
}
