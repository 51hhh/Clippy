//! 可编辑贴图 renderer v2。
//!
//! 最终 PNG 只依赖 canonical 原图、工程文档、锁定的纯 Rust 光栅器和仓库内字体。
//! WebView Canvas 仍可画交互预览，但不能再决定 Copy/Save 的持久化像素。

use base64::{engine::general_purpose::STANDARD, Engine};
use image::{Rgba, RgbaImage};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::{Arc, OnceLock};

pub(crate) const RENDERER_VERSION: u32 = 2;

const FONT_FAMILY: &str = "Noto Sans CJK SC";
const FONT_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/fonts/NotoSansCJKsc-Medium.otf"
));
// v2 同时持有 source、效果合成图和 tiny-skia 输出；32 Mi 像素可覆盖
// 4K/5K/6K 显示器和 8K UHD，又不会让 64 Mi 像素的容器上限变成数 GiB 峰值内存。
const MAX_RENDERER_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_EFFECT_WORK_PIXELS: u64 = 512 * 1024 * 1024;
const MAX_BLUR_CACHE_PIXELS: u64 = 32 * 1024 * 1024;
const HIGHLIGHT_ALPHA: f64 = 0.32;
const MARKER_WIDTH_FACTOR: f64 = 2.6;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Point {
    x: f64,
    y: f64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EffectParameters {
    blur_radius: f64,
    mosaic_cell: f64,
    spotlight_dim: f64,
    magnifier_zoom: f64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Adjustments {
    grayscale: bool,
    brightness: f64,
    contrast: f64,
    saturation: f64,
    corner_radius: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
enum Annotation {
    #[serde(rename = "pen")]
    Pen {
        id: String,
        color: String,
        size: f64,
        points: Vec<Point>,
    },
    #[serde(rename = "marker")]
    Marker {
        id: String,
        color: String,
        size: f64,
        points: Vec<Point>,
    },
    #[serde(rename = "rect")]
    Rectangle {
        id: String,
        color: String,
        size: f64,
        rect: Rect,
    },
    #[serde(rename = "ellipse")]
    Ellipse {
        id: String,
        color: String,
        size: f64,
        rect: Rect,
    },
    #[serde(rename = "highlight")]
    Highlight {
        id: String,
        color: String,
        #[serde(rename = "size")]
        _size: f64,
        rect: Rect,
    },
    #[serde(rename = "line")]
    Line {
        id: String,
        color: String,
        size: f64,
        from: Point,
        to: Point,
    },
    #[serde(rename = "arrow")]
    Arrow {
        id: String,
        color: String,
        size: f64,
        from: Point,
        to: Point,
    },
    #[serde(rename = "measure")]
    Measure {
        id: String,
        color: String,
        size: f64,
        from: Point,
        to: Point,
    },
    #[serde(rename = "text")]
    Text {
        id: String,
        color: String,
        size: f64,
        at: Point,
        text: String,
        #[serde(rename = "fontFamily")]
        font_family: String,
    },
    #[serde(rename = "blur")]
    Blur {
        id: String,
        rect: Rect,
        effect: EffectParameters,
    },
    #[serde(rename = "mosaic")]
    Mosaic {
        id: String,
        rect: Rect,
        effect: EffectParameters,
    },
    #[serde(rename = "spotlight")]
    Spotlight {
        id: String,
        rect: Rect,
        effect: EffectParameters,
    },
    #[serde(rename = "magnifier")]
    Magnifier {
        id: String,
        rect: Rect,
        effect: EffectParameters,
    },
}

impl Annotation {
    fn is_effect(&self) -> bool {
        matches!(
            self,
            Self::Blur { .. }
                | Self::Mosaic { .. }
                | Self::Spotlight { .. }
                | Self::Magnifier { .. }
        )
    }

    fn id(&self) -> &str {
        match self {
            Self::Pen { id, .. }
            | Self::Marker { id, .. }
            | Self::Rectangle { id, .. }
            | Self::Ellipse { id, .. }
            | Self::Highlight { id, .. }
            | Self::Line { id, .. }
            | Self::Arrow { id, .. }
            | Self::Measure { id, .. }
            | Self::Text { id, .. }
            | Self::Blur { id, .. }
            | Self::Mosaic { id, .. }
            | Self::Spotlight { id, .. }
            | Self::Magnifier { id, .. } => id,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PixelRect {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl PixelRect {
    fn from_rect(rect: Rect, width: u32, height: u32) -> Self {
        let x0 = rect.x.floor().clamp(0.0, f64::from(width)) as u32;
        let y0 = rect.y.floor().clamp(0.0, f64::from(height)) as u32;
        let x1 = (rect.x + rect.width)
            .ceil()
            .clamp(f64::from(x0), f64::from(width)) as u32;
        let y1 = (rect.y + rect.height)
            .ceil()
            .clamp(f64::from(y0), f64::from(height)) as u32;
        Self { x0, y0, x1, y1 }
    }

    fn width(self) -> u32 {
        self.x1.saturating_sub(self.x0)
    }

    fn height(self) -> u32 {
        self.y1.saturating_sub(self.y0)
    }

    fn area(self) -> u64 {
        u64::from(self.width()).saturating_mul(u64::from(self.height()))
    }
}

/// 以 renderer v2 的固定语义合成一张完整 PNG。
pub(super) fn render(
    source_png: &[u8],
    source_width: u32,
    source_height: u32,
    annotations: &Value,
    adjustments: &Value,
) -> Result<Vec<u8>, String> {
    super::project::validate_canvas_document(
        annotations,
        adjustments,
        source_width,
        source_height,
    )?;
    let source =
        super::project::decode_png(source_png, super::project::MAX_SOURCE_PNG_BYTES, "工程原图")?;
    if source.dimensions() != (source_width, source_height) {
        return Err("工程 sourceWidth/sourceHeight 与原图不匹配".to_string());
    }
    render_image(
        source,
        annotations,
        adjustments,
        PixelRect {
            x0: 0,
            y0: 0,
            x1: source_width,
            y1: source_height,
        },
    )
}

/// 截图覆盖层与 Pin 共用同一 v2 语义，但只输出选区视口。
///
/// 先渲染完整冻结帧再取视口，是为了让跨过选区边界的路径和模糊邻域保持原有语义。
pub(crate) fn render_capture(
    source: RgbaImage,
    crop: (u32, u32, u32, u32),
    annotations: &Value,
    adjustments: &Value,
) -> Result<Vec<u8>, String> {
    let (source_width, source_height) = source.dimensions();
    super::project::validate_canvas_document(
        annotations,
        adjustments,
        source_width,
        source_height,
    )?;
    let (x, y, width, height) = crop;
    let x1 = x.checked_add(width).ok_or("截图渲染视口越界")?;
    let y1 = y.checked_add(height).ok_or("截图渲染视口越界")?;
    if width == 0 || height == 0 || x1 > source_width || y1 > source_height {
        return Err("截图渲染视口越界".to_string());
    }
    render_image(
        source,
        annotations,
        adjustments,
        PixelRect {
            x0: x,
            y0: y,
            x1,
            y1,
        },
    )
}

fn render_image(
    source: RgbaImage,
    annotations: &Value,
    adjustments: &Value,
    output_rect: PixelRect,
) -> Result<Vec<u8>, String> {
    let (source_width, source_height) = source.dimensions();
    let annotations: Vec<Annotation> = serde_json::from_value(annotations.clone())
        .map_err(|error| format!("renderer v2 annotations 解析失败: {error}"))?;
    let adjustments: Adjustments = serde_json::from_value(adjustments.clone())
        .map_err(|error| format!("renderer v2 adjustments 解析失败: {error}"))?;
    validate_renderer_values(&annotations)?;
    validate_effect_budget(&annotations, source_width, source_height)?;

    let adjusted = apply_adjustments(source, adjustments);
    let composited = apply_effects(&adjusted, &annotations)?;
    render_vectors(
        &composited,
        &annotations,
        adjustments.corner_radius,
        output_rect,
    )
}

fn validate_renderer_values(annotations: &[Annotation]) -> Result<(), String> {
    let mut ids = HashSet::with_capacity(annotations.len());
    for annotation in annotations {
        if !ids.insert(annotation.id()) {
            return Err("renderer v2 annotation id 重复".to_string());
        }
        match annotation {
            Annotation::Pen { color, .. }
            | Annotation::Marker { color, .. }
            | Annotation::Rectangle { color, .. }
            | Annotation::Ellipse { color, .. }
            | Annotation::Highlight { color, .. }
            | Annotation::Line { color, .. }
            | Annotation::Arrow { color, .. }
            | Annotation::Measure { color, .. }
            | Annotation::Text { color, .. } => validate_hex_color(color)?,
            _ => {}
        }
        if let Annotation::Text { font_family, .. } = annotation {
            if font_family != "system-ui" {
                return Err("renderer v2 fontFamily 不受支持".to_string());
            }
        }
    }
    Ok(())
}

fn validate_hex_color(color: &str) -> Result<(), String> {
    let Some(hex) = color.strip_prefix('#') else {
        return Err("renderer v2 只接受十六进制颜色".to_string());
    };
    if !matches!(hex.len(), 3 | 4 | 6 | 8) || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("renderer v2 十六进制颜色无效".to_string());
    }
    Ok(())
}

fn validate_effect_budget(
    annotations: &[Annotation],
    width: u32,
    height: u32,
) -> Result<(), String> {
    let full_area = u64::from(width).saturating_mul(u64::from(height));
    if full_area > MAX_RENDERER_PIXELS {
        return Err("renderer v2 图像像素超过安全预算".to_string());
    }
    let mut work = 0u64;
    let mut blur_radii = HashSet::new();
    for annotation in annotations {
        match annotation {
            Annotation::Blur { effect, .. } => {
                blur_radii.insert(effect.blur_radius.round().clamp(1.0, 100.0) as u32);
            }
            Annotation::Spotlight { .. } => work = work.saturating_add(full_area),
            Annotation::Mosaic { rect, .. } | Annotation::Magnifier { rect, .. } => {
                work = work.saturating_add(PixelRect::from_rect(*rect, width, height).area());
            }
            _ => {}
        }
    }
    let blur_pixels = full_area.saturating_mul(blur_radii.len() as u64);
    if blur_pixels > MAX_BLUR_CACHE_PIXELS {
        return Err("renderer v2 模糊缓存超过安全预算".to_string());
    }
    work = work.saturating_add(blur_pixels.saturating_mul(6));
    if work > MAX_EFFECT_WORK_PIXELS {
        return Err("renderer v2 效果处理超过安全预算".to_string());
    }
    Ok(())
}

fn apply_adjustments(mut source: RgbaImage, adjustments: Adjustments) -> RgbaImage {
    let brightness = 100 + normalized_percent(adjustments.brightness);
    let contrast = 100 + normalized_percent(adjustments.contrast);
    let saturation = 100 + normalized_percent(adjustments.saturation);
    for pixel in source.pixels_mut() {
        let mut rgb = [
            i32::from(pixel[0]),
            i32::from(pixel[1]),
            i32::from(pixel[2]),
        ];
        if adjustments.grayscale {
            let gray = div_round(2_126 * rgb[0] + 7_152 * rgb[1] + 722 * rgb[2], 10_000);
            rgb = [gray; 3];
        }
        for channel in &mut rgb {
            *channel = div_round(*channel * brightness, 100);
            *channel = div_round((*channel * 2 - 255) * contrast, 200) + 128;
        }
        let gray = div_round(2_126 * rgb[0] + 7_152 * rgb[1] + 722 * rgb[2], 10_000);
        for channel in &mut rgb {
            *channel = gray + div_round((*channel - gray) * saturation, 100);
        }
        pixel[0] = clamp_u8(rgb[0]);
        pixel[1] = clamp_u8(rgb[1]);
        pixel[2] = clamp_u8(rgb[2]);
    }
    source
}

fn normalized_percent(value: f64) -> i32 {
    value.round().clamp(-100.0, 100.0) as i32
}

fn div_round(value: i32, divisor: i32) -> i32 {
    if value >= 0 {
        (value + divisor / 2) / divisor
    } else {
        (value - divisor / 2) / divisor
    }
}

fn clamp_u8(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

fn apply_effects(source: &RgbaImage, annotations: &[Annotation]) -> Result<RgbaImage, String> {
    let (width, height) = source.dimensions();
    let mut output = source.clone();
    let mut blurred = HashMap::<u32, RgbaImage>::new();
    for annotation in annotations
        .iter()
        .filter(|annotation| annotation.is_effect())
    {
        match annotation {
            Annotation::Blur { rect, effect, .. } => {
                let radius = effect.blur_radius.round().clamp(1.0, 100.0) as u32;
                let pixels = blurred
                    .entry(radius)
                    .or_insert_with(|| three_pass_box_blur(source, radius));
                copy_rect(
                    pixels,
                    &mut output,
                    PixelRect::from_rect(*rect, width, height),
                );
            }
            Annotation::Mosaic { rect, effect, .. } => {
                apply_mosaic(
                    source,
                    &mut output,
                    PixelRect::from_rect(*rect, width, height),
                    effect.mosaic_cell.round().clamp(6.0, 256.0) as u32,
                );
            }
            Annotation::Spotlight { rect, effect, .. } => apply_spotlight(
                &mut output,
                PixelRect::from_rect(*rect, width, height),
                (effect.spotlight_dim.clamp(0.0, 1.0) * 255.0).round() as u8,
            ),
            Annotation::Magnifier { rect, effect, .. } => apply_magnifier(
                source,
                &mut output,
                PixelRect::from_rect(*rect, width, height),
                effect.magnifier_zoom,
            ),
            _ => unreachable!("只遍历效果标注"),
        }
    }
    Ok(output)
}

fn copy_rect(source: &RgbaImage, target: &mut RgbaImage, rect: PixelRect) {
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            target.put_pixel(x, y, *source.get_pixel(x, y));
        }
    }
}

fn apply_spotlight(target: &mut RgbaImage, rect: PixelRect, alpha: u8) {
    for (x, y, pixel) in target.enumerate_pixels_mut() {
        if x < rect.x0 || x >= rect.x1 || y < rect.y0 || y >= rect.y1 {
            *pixel = blend_over(*pixel, Rgba([0, 0, 0, alpha]));
        }
    }
}

fn apply_mosaic(source: &RgbaImage, target: &mut RgbaImage, rect: PixelRect, cell: u32) {
    if rect.width() == 0 || rect.height() == 0 {
        return;
    }
    let mut y = rect.y0;
    while y < rect.y1 {
        let end_y = y.saturating_add(cell).min(rect.y1);
        let mut x = rect.x0;
        while x < rect.x1 {
            let end_x = x.saturating_add(cell).min(rect.x1);
            let mut sums = [0u64; 4];
            let count = u64::from(end_x - x) * u64::from(end_y - y);
            for sample_y in y..end_y {
                for sample_x in x..end_x {
                    let pixel = source.get_pixel(sample_x, sample_y);
                    for channel in 0..4 {
                        sums[channel] += u64::from(pixel[channel]);
                    }
                }
            }
            let average = Rgba(sums.map(|sum| ((sum + count / 2) / count) as u8));
            for fill_y in y..end_y {
                for fill_x in x..end_x {
                    target.put_pixel(fill_x, fill_y, average);
                }
            }
            x = end_x;
        }
        y = end_y;
    }
}

fn apply_magnifier(source: &RgbaImage, target: &mut RgbaImage, rect: PixelRect, zoom: f64) {
    let width = rect.width();
    let height = rect.height();
    if width == 0 || height == 0 {
        return;
    }
    let zoom_q16 = (zoom.clamp(1.0, 16.0) * 65_536.0).round() as i64;
    let center_x_q16 = i64::from(rect.x0 + rect.x1) * 32_768;
    let center_y_q16 = i64::from(rect.y0 + rect.y1) * 32_768;
    let width_i = i64::from(width);
    let height_i = i64::from(height);
    let ellipse_limit = width_i * width_i * height_i * height_i;
    for y in rect.y0..rect.y1 {
        let dy = i64::from(2 * (y - rect.y0) + 1) - height_i;
        for x in rect.x0..rect.x1 {
            let dx = i64::from(2 * (x - rect.x0) + 1) - width_i;
            if dx * dx * height_i * height_i + dy * dy * width_i * width_i > ellipse_limit {
                continue;
            }
            let pixel_x_q16 = i64::from(x) * 65_536 + 32_768;
            let pixel_y_q16 = i64::from(y) * 65_536 + 32_768;
            let source_x_q16 = center_x_q16 + (pixel_x_q16 - center_x_q16) * 65_536 / zoom_q16;
            let source_y_q16 = center_y_q16 + (pixel_y_q16 - center_y_q16) * 65_536 / zoom_q16;
            let sampled = bilinear_sample(source, source_x_q16, source_y_q16);
            let existing = *target.get_pixel(x, y);
            target.put_pixel(x, y, blend_over(existing, sampled));
        }
    }
}

/// 坐标使用像素边界空间的 Q16；采样前减半像素得到像素中心网格。
fn bilinear_sample(image: &RgbaImage, edge_x_q16: i64, edge_y_q16: i64) -> Rgba<u8> {
    let max_x_q16 = i64::from(image.width().saturating_sub(1)) * 65_536;
    let max_y_q16 = i64::from(image.height().saturating_sub(1)) * 65_536;
    let grid_x = (edge_x_q16 - 32_768).clamp(0, max_x_q16);
    let grid_y = (edge_y_q16 - 32_768).clamp(0, max_y_q16);
    let x0 = (grid_x >> 16) as u32;
    let y0 = (grid_y >> 16) as u32;
    let x1 = x0.saturating_add(1).min(image.width() - 1);
    let y1 = y0.saturating_add(1).min(image.height() - 1);
    let fx = (grid_x & 0xffff) as u64;
    let fy = (grid_y & 0xffff) as u64;
    let inv_x = 65_536 - fx;
    let inv_y = 65_536 - fy;
    let weights = [inv_x * inv_y, fx * inv_y, inv_x * fy, fx * fy];
    let pixels = [
        image.get_pixel(x0, y0),
        image.get_pixel(x1, y0),
        image.get_pixel(x0, y1),
        image.get_pixel(x1, y1),
    ];
    let mut output = [0u8; 4];
    for channel in 0..4 {
        let sum = pixels
            .iter()
            .zip(weights)
            .map(|(pixel, weight)| u64::from(pixel[channel]) * weight)
            .sum::<u64>();
        output[channel] = ((sum + (1u64 << 31)) >> 32) as u8;
    }
    Rgba(output)
}

fn blend_over(destination: Rgba<u8>, source: Rgba<u8>) -> Rgba<u8> {
    let source_alpha = u32::from(source[3]);
    let destination_alpha = u32::from(destination[3]);
    let inverse = 255 - source_alpha;
    let output_alpha = source_alpha + div_round_u32(destination_alpha * inverse, 255);
    if output_alpha == 0 {
        return Rgba([0, 0, 0, 0]);
    }
    let mut output = [0u8; 4];
    for channel in 0..3 {
        let premultiplied = u32::from(source[channel]) * source_alpha
            + div_round_u32(
                u32::from(destination[channel]) * destination_alpha * inverse,
                255,
            );
        output[channel] = div_round_u32(premultiplied, output_alpha).min(255) as u8;
    }
    output[3] = output_alpha.min(255) as u8;
    Rgba(output)
}

fn div_round_u32(value: u32, divisor: u32) -> u32 {
    (value + divisor / 2) / divisor
}

fn three_pass_box_blur(source: &RgbaImage, radius: u32) -> RgbaImage {
    let mut current = source.clone();
    for _ in 0..3 {
        current = box_blur_horizontal(&current, radius);
        current = box_blur_vertical(&current, radius);
    }
    current
}

fn box_blur_horizontal(source: &RgbaImage, radius: u32) -> RgbaImage {
    let (width, height) = source.dimensions();
    let mut output = RgbaImage::new(width, height);
    let radius = radius.min(width.saturating_sub(1));
    let window = u64::from(radius) * 2 + 1;
    for y in 0..height {
        let mut sums = [0u64; 4];
        for offset in -(i64::from(radius))..=i64::from(radius) {
            let x = offset.clamp(0, i64::from(width - 1)) as u32;
            let pixel = source.get_pixel(x, y);
            for channel in 0..4 {
                sums[channel] += u64::from(pixel[channel]);
            }
        }
        for x in 0..width {
            output.put_pixel(
                x,
                y,
                Rgba(sums.map(|sum| ((sum + window / 2) / window) as u8)),
            );
            let remove = i64::from(x) - i64::from(radius);
            let add = i64::from(x) + i64::from(radius) + 1;
            let remove_x = remove.clamp(0, i64::from(width - 1)) as u32;
            let add_x = add.clamp(0, i64::from(width - 1)) as u32;
            let old = source.get_pixel(remove_x, y);
            let new = source.get_pixel(add_x, y);
            for channel in 0..4 {
                sums[channel] = sums[channel] + u64::from(new[channel]) - u64::from(old[channel]);
            }
        }
    }
    output
}

fn box_blur_vertical(source: &RgbaImage, radius: u32) -> RgbaImage {
    let (width, height) = source.dimensions();
    let mut output = RgbaImage::new(width, height);
    let radius = radius.min(height.saturating_sub(1));
    let window = u64::from(radius) * 2 + 1;
    for x in 0..width {
        let mut sums = [0u64; 4];
        for offset in -(i64::from(radius))..=i64::from(radius) {
            let y = offset.clamp(0, i64::from(height - 1)) as u32;
            let pixel = source.get_pixel(x, y);
            for channel in 0..4 {
                sums[channel] += u64::from(pixel[channel]);
            }
        }
        for y in 0..height {
            output.put_pixel(
                x,
                y,
                Rgba(sums.map(|sum| ((sum + window / 2) / window) as u8)),
            );
            let remove = i64::from(y) - i64::from(radius);
            let add = i64::from(y) + i64::from(radius) + 1;
            let remove_y = remove.clamp(0, i64::from(height - 1)) as u32;
            let add_y = add.clamp(0, i64::from(height - 1)) as u32;
            let old = source.get_pixel(x, remove_y);
            let new = source.get_pixel(x, add_y);
            for channel in 0..4 {
                sums[channel] = sums[channel] + u64::from(new[channel]) - u64::from(old[channel]);
            }
        }
    }
    output
}

fn render_vectors(
    composited: &RgbaImage,
    annotations: &[Annotation],
    corner_radius: f64,
    output_rect: PixelRect,
) -> Result<Vec<u8>, String> {
    let (source_width, source_height) = composited.dimensions();
    let output_width = output_rect.width();
    let output_height = output_rect.height();
    let base_png = crate::screenshot::encode_png(composited.as_raw(), source_width, source_height)
        .map_err(|error| format!("renderer v2 中间 PNG 编码失败: {error}"))?;
    let mut svg = String::with_capacity(base_png.len().saturating_mul(4) / 3 + 8_192);
    write!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{output_width}" height="{output_height}" viewBox="{} {} {output_width} {output_height}" shape-rendering="geometricPrecision" text-rendering="geometricPrecision"><defs>"#,
        output_rect.x0,
        output_rect.y0
    )
    .expect("写 String 不会失败");
    let radius = corner_radius
        .round()
        .clamp(0.0, f64::from(output_width.min(output_height)) / 2.0);
    if radius > 0.0 {
        write!(
            svg,
            r#"<clipPath id="rounded"><rect x="{}" y="{}" width="{output_width}" height="{output_height}" rx="{}" ry="{}"/></clipPath>"#,
            output_rect.x0,
            output_rect.y0,
            number(radius),
            number(radius)
        )
        .expect("写 String 不会失败");
    }
    svg.push_str("</defs>");
    if radius > 0.0 {
        svg.push_str(r#"<g clip-path="url(#rounded)">"#);
    } else {
        svg.push_str("<g>");
    }
    write!(
        svg,
        r#"<image width="{source_width}" height="{source_height}" preserveAspectRatio="none" image-rendering="optimizeSpeed" href="data:image/png;base64,{}"/>"#,
        STANDARD.encode(base_png)
    )
    .expect("写 String 不会失败");

    // 放大镜白边属于效果层，必须在所有用户矢量标注之前绘制。
    for annotation in annotations {
        if let Annotation::Magnifier { rect, .. } = annotation {
            let cx = rect.x + rect.width / 2.0;
            let cy = rect.y + rect.height / 2.0;
            write!(
                svg,
                r##"<ellipse cx="{}" cy="{}" rx="{}" ry="{}" fill="none" stroke="#ffffff" stroke-opacity="0.92" stroke-width="2"/>"##,
                number(cx),
                number(cy),
                number(rect.width / 2.0),
                number(rect.height / 2.0)
            )
            .expect("写 String 不会失败");
        }
    }
    for annotation in annotations
        .iter()
        .filter(|annotation| !annotation.is_effect())
    {
        append_vector(&mut svg, annotation)?;
    }
    svg.push_str("</g></svg>");

    let mut options = resvg::usvg::Options {
        font_family: FONT_FAMILY.to_string(),
        ..resvg::usvg::Options::default()
    };
    options.fontdb = renderer_font_database();
    let tree = resvg::usvg::Tree::from_data(svg.as_bytes(), &options)
        .map_err(|error| format!("renderer v2 SVG 解析失败: {error}"))?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(output_width, output_height)
        .ok_or_else(|| "renderer v2 无法分配输出像素".to_string())?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .map_err(|error| format!("renderer v2 PNG 编码失败: {error}"))
}

fn renderer_font_database() -> Arc<resvg::usvg::fontdb::Database> {
    static DATABASE: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    DATABASE
        .get_or_init(|| {
            let mut database = resvg::usvg::fontdb::Database::new();
            database.load_font_data(FONT_BYTES.to_vec());
            Arc::new(database)
        })
        .clone()
}

fn append_vector(svg: &mut String, annotation: &Annotation) -> Result<(), String> {
    match annotation {
        Annotation::Pen {
            color,
            size,
            points,
            ..
        } => append_polyline(svg, color, *size, points, 1.0, "round"),
        Annotation::Marker {
            color,
            size,
            points,
            ..
        } => append_polyline(
            svg,
            color,
            (*size * MARKER_WIDTH_FACTOR).max(2.0),
            points,
            HIGHLIGHT_ALPHA,
            "butt",
        ),
        Annotation::Rectangle {
            color, size, rect, ..
        } => {
            write!(
                svg,
                r#"<rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="{}" stroke-width="{}"/>"#,
                number(rect.x),
                number(rect.y),
                number(rect.width),
                number(rect.height),
                color,
                number(size.max(1.0))
            )
            .expect("写 String 不会失败");
        }
        Annotation::Ellipse {
            color, size, rect, ..
        } => {
            write!(
                svg,
                r#"<ellipse cx="{}" cy="{}" rx="{}" ry="{}" fill="none" stroke="{}" stroke-width="{}"/>"#,
                number(rect.x + rect.width / 2.0),
                number(rect.y + rect.height / 2.0),
                number(rect.width / 2.0),
                number(rect.height / 2.0),
                color,
                number(size.max(1.0))
            )
            .expect("写 String 不会失败");
        }
        Annotation::Highlight { color, rect, .. } => {
            write!(
                svg,
                r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}" fill-opacity="{}"/>"#,
                number(rect.x),
                number(rect.y),
                number(rect.width),
                number(rect.height),
                color,
                number(HIGHLIGHT_ALPHA)
            )
            .expect("写 String 不会失败");
        }
        Annotation::Line {
            color,
            size,
            from,
            to,
            ..
        } => append_segment(svg, color, *size, *from, *to),
        Annotation::Arrow {
            color,
            size,
            from,
            to,
            ..
        } => {
            append_segment(svg, color, *size, *from, *to);
            let angle = (to.y - from.y).atan2(to.x - from.x);
            let length = (size * 4.0).max(10.0);
            let left = Point {
                x: to.x - length * (angle - std::f64::consts::PI / 7.0).cos(),
                y: to.y - length * (angle - std::f64::consts::PI / 7.0).sin(),
            };
            let right = Point {
                x: to.x - length * (angle + std::f64::consts::PI / 7.0).cos(),
                y: to.y - length * (angle + std::f64::consts::PI / 7.0).sin(),
            };
            append_segment(svg, color, *size, *to, left);
            append_segment(svg, color, *size, *to, right);
        }
        Annotation::Measure {
            color,
            size,
            from,
            to,
            ..
        } => append_measure(svg, color, *size, *from, *to),
        Annotation::Text {
            color,
            size,
            at,
            text,
            ..
        } => append_text(svg, color, *size, *at, text, "start"),
        _ => unreachable!("效果标注不会进入矢量分支"),
    }
    Ok(())
}

fn append_polyline(
    svg: &mut String,
    color: &str,
    width: f64,
    points: &[Point],
    opacity: f64,
    line_cap: &str,
) {
    if points.len() < 2 {
        return;
    }
    let mut path = String::new();
    for (index, point) in points.iter().enumerate() {
        write!(
            path,
            "{}{} {}",
            if index == 0 { "M" } else { " L" },
            number(point.x),
            number(point.y)
        )
        .expect("写 String 不会失败");
    }
    write!(
        svg,
        r#"<path d="{path}" fill="none" stroke="{color}" stroke-width="{}" stroke-opacity="{}" stroke-linecap="{line_cap}" stroke-linejoin="round"/>"#,
        number(width.max(1.0)),
        number(opacity)
    )
    .expect("写 String 不会失败");
}

fn append_segment(svg: &mut String, color: &str, width: f64, from: Point, to: Point) {
    write!(
        svg,
        r#"<path d="M{} {} L{} {}" fill="none" stroke="{}" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round"/>"#,
        number(from.x),
        number(from.y),
        number(to.x),
        number(to.y),
        color,
        number(width.max(1.0))
    )
    .expect("写 String 不会失败");
}

fn append_measure(svg: &mut String, color: &str, size: f64, from: Point, to: Point) {
    append_segment(svg, color, size, from, to);
    let angle = (to.y - from.y).atan2(to.x - from.x);
    let tick = (size * 2.5).max(6.0);
    let normal = Point {
        x: -angle.sin() * tick,
        y: angle.cos() * tick,
    };
    for end in [from, to] {
        append_segment(
            svg,
            color,
            size,
            Point {
                x: end.x - normal.x,
                y: end.y - normal.y,
            },
            Point {
                x: end.x + normal.x,
                y: end.y + normal.y,
            },
        );
    }
    let pixels = ((to.x - from.x).hypot(to.y - from.y)).round() as i64;
    let at = Point {
        x: (from.x + to.x) / 2.0,
        y: (from.y + to.y) / 2.0 - tick,
    };
    append_text(
        svg,
        color,
        size * 0.8,
        at,
        &format!("{pixels} px"),
        "middle",
    );
}

fn append_text(svg: &mut String, color: &str, size: f64, at: Point, text: &str, anchor: &str) {
    let font_size = (size * 4.0).max(14.0);
    let stroke_width = size.max(3.0);
    // v2 明确定义为单行：控制字符不会交给 XML parser 改变布局。
    let normalized = text
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    write!(
        svg,
        r##"<text x="{}" y="{}" font-family="{}" font-size="{}" font-weight="500" text-anchor="{}" dominant-baseline="text-before-edge" fill="{}" stroke="#000000" stroke-opacity="0.55" stroke-width="{}" stroke-linejoin="round" paint-order="stroke fill">{}</text>"##,
        number(at.x),
        number(at.y),
        FONT_FAMILY,
        number(font_size),
        anchor,
        color,
        number(stroke_width),
        escape_xml(&normalized)
    )
    .expect("写 String 不会失败");
}

fn escape_xml(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn number(value: f64) -> String {
    if value == -0.0 {
        "0".to_string()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::time::Instant;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&[
                    (x * 17 + y * 3) as u8,
                    (x * 5 + y * 19) as u8,
                    (x * 11 + y * 7) as u8,
                    255,
                ]);
            }
        }
        crate::screenshot::encode_png(&rgba, width, height).unwrap()
    }

    fn effect() -> Value {
        serde_json::json!({
            "blurRadius": 2,
            "mosaicCell": 3,
            "spotlightDim": 0.5,
            "magnifierZoom": 2
        })
    }

    fn adjustments() -> Value {
        serde_json::json!({
            "grayscale": false,
            "brightness": 0,
            "contrast": 0,
            "saturation": 0,
            "cornerRadius": 0
        })
    }

    fn decode(png: &[u8]) -> RgbaImage {
        image::load_from_memory_with_format(png, image::ImageFormat::Png)
            .unwrap()
            .into_rgba8()
    }

    #[test]
    fn fixed_point_adjustments_are_stable_and_keep_alpha() {
        let source = RgbaImage::from_raw(1, 1, vec![20, 100, 220, 77]).unwrap();
        let output = apply_adjustments(
            source,
            Adjustments {
                grayscale: true,
                brightness: 20.0,
                contrast: -10.0,
                saturation: 35.0,
                corner_radius: 0.0,
            },
        );
        assert_eq!(output.as_raw(), &[112, 112, 112, 77]);
    }

    #[test]
    fn effects_sample_the_adjusted_source_and_preserve_document_order() {
        let source_png = png(12, 10);
        let annotations = serde_json::json!([
            {"id":"m","type":"mosaic","rect":{"x":1,"y":1,"width":5,"height":5},"effect":effect()},
            {"id":"z","type":"magnifier","rect":{"x":6,"y":2,"width":5,"height":6},"effect":effect()},
            {"id":"s","type":"spotlight","rect":{"x":2,"y":2,"width":8,"height":6},"effect":effect()}
        ]);
        let first = render(&source_png, 12, 10, &annotations, &adjustments()).unwrap();
        let second = render(&source_png, 12, 10, &annotations, &adjustments()).unwrap();
        assert_eq!(decode(&first), decode(&second));
        assert_ne!(decode(&first), decode(&source_png));
    }

    #[test]
    fn capture_viewport_matches_the_same_area_of_a_full_render() {
        let source_png = png(24, 18);
        let source = decode(&source_png);
        let annotations = serde_json::json!([
            {"id":"b","type":"blur","rect":{"x":2,"y":2,"width":15,"height":10},"effect":effect()},
            {"id":"p","type":"pen","color":"#ff3b30","size":2,"points":[{"x":1,"y":1},{"x":22,"y":16}]}
        ]);
        let full = decode(&render(&source_png, 24, 18, &annotations, &adjustments()).unwrap());
        let cropped =
            decode(&render_capture(source, (5, 4, 12, 9), &annotations, &adjustments()).unwrap());
        assert_eq!(
            cropped,
            image::imageops::crop_imm(&full, 5, 4, 12, 9).to_image()
        );
    }

    #[test]
    fn capture_viewport_rejects_empty_or_out_of_bounds_rectangles() {
        let source = decode(&png(8, 6));
        assert!(render_capture(
            source.clone(),
            (0, 0, 0, 2),
            &serde_json::json!([]),
            &adjustments()
        )
        .is_err());
        assert!(
            render_capture(source, (7, 5, 2, 2), &serde_json::json!([]), &adjustments()).is_err()
        );
    }

    #[test]
    fn every_vector_tool_and_fixed_chinese_font_render_without_system_fonts() {
        let source_png = png(240, 160);
        let annotations = serde_json::json!([
            {"id":"p","type":"pen","color":"#ff3b30","size":4,"points":[{"x":5,"y":5},{"x":50,"y":20}]},
            {"id":"k","type":"marker","color":"#ffcc00","size":4,"points":[{"x":5,"y":30},{"x":70,"y":30}]},
            {"id":"r","type":"rect","color":"#34c759","size":3,"rect":{"x":10,"y":40,"width":40,"height":30}},
            {"id":"e","type":"ellipse","color":"#0a84ff","size":3,"rect":{"x":60,"y":40,"width":40,"height":30}},
            {"id":"h","type":"highlight","color":"#ffcc00","size":3,"rect":{"x":110,"y":40,"width":40,"height":30}},
            {"id":"l","type":"line","color":"#ffffff","size":3,"from":{"x":10,"y":80},"to":{"x":70,"y":100}},
            {"id":"a","type":"arrow","color":"#ff3b30","size":3,"from":{"x":80,"y":80},"to":{"x":140,"y":100}},
            {"id":"q","type":"measure","color":"#34c759","size":3,"from":{"x":10,"y":130},"to":{"x":100,"y":130}},
            {"id":"t","type":"text","color":"#ffffff","size":4,"at":{"x":145,"y":80},"text":"Clippy 中文 <&>","fontFamily":"system-ui"}
        ]);
        let output = render(&source_png, 240, 160, &annotations, &adjustments()).unwrap();
        assert_eq!(decode(&output).dimensions(), (240, 160));
        assert_ne!(decode(&output), decode(&source_png));
    }

    #[test]
    fn combined_fixture_has_a_stable_rgba_digest() {
        let source_png = png(64, 48);
        let annotations = serde_json::json!([
            {"id":"b","type":"blur","rect":{"x":0,"y":0,"width":18,"height":16},"effect":effect()},
            {"id":"m","type":"mosaic","rect":{"x":18,"y":0,"width":18,"height":16},"effect":effect()},
            {"id":"z","type":"magnifier","rect":{"x":36,"y":0,"width":20,"height":18},"effect":effect()},
            {"id":"s","type":"spotlight","rect":{"x":3,"y":3,"width":58,"height":42},"effect":effect()},
            {"id":"p","type":"pen","color":"#ff3b30","size":2,"points":[{"x":2,"y":22},{"x":30,"y":27}]},
            {"id":"k","type":"marker","color":"#ffcc00","size":2,"points":[{"x":32,"y":22},{"x":61,"y":25}]},
            {"id":"r","type":"rect","color":"#34c759","size":2,"rect":{"x":3,"y":29,"width":12,"height":9}},
            {"id":"e","type":"ellipse","color":"#0a84ff","size":2,"rect":{"x":17,"y":29,"width":12,"height":9}},
            {"id":"h","type":"highlight","color":"#ffcc00","size":2,"rect":{"x":31,"y":29,"width":12,"height":9}},
            {"id":"l","type":"line","color":"#ffffff","size":2,"from":{"x":45,"y":30},"to":{"x":61,"y":38}},
            {"id":"a","type":"arrow","color":"#ff3b30","size":2,"from":{"x":2,"y":43},"to":{"x":18,"y":41}},
            {"id":"q","type":"measure","color":"#34c759","size":2,"from":{"x":22,"y":43},"to":{"x":42,"y":43}},
            {"id":"t","type":"text","color":"#ffffff","size":2,"at":{"x":44,"y":39},"text":"中A<&>","fontFamily":"system-ui"}
        ]);
        let adjusted = serde_json::json!({
            "grayscale": false,
            "brightness": 8,
            "contrast": 12,
            "saturation": -15,
            "cornerRadius": 5
        });
        let output = render(&source_png, 64, 48, &annotations, &adjusted).unwrap();
        let image = decode(&output);
        let digest = format!("{:x}", Sha256::digest(image.as_raw()));
        assert_eq!(
            digest,
            "0868d38bf2e18a1f62d01cfa55d37954b1a66d3f2b99b3affb83dbe5d1b64478"
        );
    }

    #[test]
    fn renderer_rejects_oversized_images_and_blur_caches_without_allocating_them() {
        let empty = Vec::new();
        assert!(validate_effect_budget(&empty, 8_192, 4_096).is_ok());
        assert_eq!(
            validate_effect_budget(&empty, 8_192, 4_097).unwrap_err(),
            "renderer v2 图像像素超过安全预算"
        );

        let annotations: Vec<Annotation> = serde_json::from_value(serde_json::json!([
            {"id":"a","type":"blur","rect":{"x":0,"y":0,"width":1,"height":1},"effect":{"blurRadius":1,"mosaicCell":6,"spotlightDim":0.5,"magnifierZoom":2}},
            {"id":"b","type":"blur","rect":{"x":0,"y":0,"width":1,"height":1},"effect":{"blurRadius":2,"mosaicCell":6,"spotlightDim":0.5,"magnifierZoom":2}},
            {"id":"c","type":"blur","rect":{"x":0,"y":0,"width":1,"height":1},"effect":{"blurRadius":3,"mosaicCell":6,"spotlightDim":0.5,"magnifierZoom":2}}
        ]))
        .unwrap();
        assert_eq!(
            validate_effect_budget(&annotations, 4_096, 4_096).unwrap_err(),
            "renderer v2 模糊缓存超过安全预算"
        );
    }

    #[test]
    #[ignore = "4K renderer v2 手动性能探针"]
    fn renderer_v2_4k_performance_probe() {
        let source_png = png(3_840, 2_160);
        let annotations = serde_json::json!([
            {"id":"b","type":"blur","rect":{"x":40,"y":40,"width":700,"height":420},"effect":{"blurRadius":8,"mosaicCell":16,"spotlightDim":0.5,"magnifierZoom":2}},
            {"id":"m","type":"mosaic","rect":{"x":800,"y":80,"width":800,"height":500},"effect":{"blurRadius":8,"mosaicCell":16,"spotlightDim":0.5,"magnifierZoom":2}},
            {"id":"z","type":"magnifier","rect":{"x":1700,"y":120,"width":640,"height":480},"effect":{"blurRadius":8,"mosaicCell":16,"spotlightDim":0.5,"magnifierZoom":2}},
            {"id":"s","type":"spotlight","rect":{"x":200,"y":160,"width":3400,"height":1800},"effect":{"blurRadius":8,"mosaicCell":16,"spotlightDim":0.3,"magnifierZoom":2}},
            {"id":"p","type":"pen","color":"#ff3b30","size":8,"points":[{"x":100,"y":800},{"x":1900,"y":1100},{"x":3600,"y":900}]},
            {"id":"r","type":"rect","color":"#34c759","size":8,"rect":{"x":400,"y":1200,"width":900,"height":600}},
            {"id":"a","type":"arrow","color":"#0a84ff","size":8,"from":{"x":1500,"y":1800},"to":{"x":3000,"y":1200}},
            {"id":"t","type":"text","color":"#ffffff","size":10,"at":{"x":2600,"y":1700},"text":"Clippy 4K 跨平台","fontFamily":"system-ui"}
        ]);
        let started = Instant::now();
        let output = render(&source_png, 3_840, 2_160, &annotations, &adjustments()).unwrap();
        let elapsed = started.elapsed();
        assert_eq!(decode(&output).dimensions(), (3_840, 2_160));
        eprintln!("renderer v2 4K 合成耗时: {elapsed:?}");
    }

    #[test]
    fn renderer_rejects_css_and_markup_colors() {
        let source_png = png(4, 4);
        let annotations = serde_json::json!([
            {"id":"x","type":"rect","color":"url(javascript:1)","size":2,"rect":{"x":0,"y":0,"width":2,"height":2}}
        ]);
        assert!(render(&source_png, 4, 4, &annotations, &adjustments()).is_err());
    }
}
