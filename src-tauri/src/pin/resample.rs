//! 贴图内容按**缓冲区分辨率**出图，并预先补偿合成器那一步缩小。
//!
//! # 为什么需要这一步
//!
//! GTK3 不支持 `wp_fractional_scale_v1`，所以在 1.5 倍缩放的桌面上 Mutter 只会给窗口
//! **整数缓冲区缩放 2**：窗口按 2 倍画，合成器再把整张画面按 0.75 缩到屏上。于是"按原
//! 物理尺寸贴一张截图"这件事，链路一定是
//!
//! ```text
//! 图片 1200x900 → WebKit 放大 4/3 → 缓冲区 1600x1200 → 合成器缩小 3/4 → 屏上 1200x900
//! ```
//!
//! 中间那趟放大是**逃不掉的**（缓冲区尺寸由合成器定，屏上尺寸由"原尺寸"定，比值恒为
//! 4/3），所以问题不是"要不要重采样"，而是"缓冲区里放什么，才能让合成器缩完之后最接近
//! 原图"。本机 HDMI-1（3840x2160 物理 / 2560x1440 逻辑）上把 WebKitGTK 的成像用
//! PipeWire 原生取流拍下来，与源图逐像素比 PSNR：
//!
//! | 缓冲区里放什么 | 实测 PSNR |
//! |---|---|
//! | 源图，WebKit 默认平滑放大 | 30.28 dB |
//! | 源图，`image-rendering: pixelated` | 33.95 dB |
//! | 预先 Lanczos3 放到 1600x1200，1:1 搬进去 | 34.73 dB |
//! | **同上再做 4~6 轮反投影补偿** | **43.45 dB** |
//!
//! 前三行都在"能看出糊"的量级上，只有最后一行到了肉眼分不出来的程度——所以这里做的是
//! 最后一行：先把图渲染成缓冲区尺寸，再迭代地问"这张缓冲区图被合成器缩完等于原图吗"，
//! 把差值加回去。合成器那一步的核也是实测出来的：缓冲区里一个孤立白点在屏上只留下
//! **一个** 177/255 的点（= 0.8333²），正是"输出像素中心映射回输入坐标"的标准双线性，
//! 与 [`resample_bilinear`] 完全一致，所以反投影用的前向模型是准的（离线预测 43.49 dB
//! 与实测 43.45 dB 对得上）。
//!
//! # 边界
//!
//! - 只在缓冲区缩放与真实缩放**不相等**时才做（X11、整数缩放的桌面本来就是 1:1）。
//! - 补偿是**按当前那块屏**算的。把贴图拖到缩放不同的另一块屏上，补偿量就偏了，
//!   看起来会有一点过锐——Wayland 不告诉客户端窗口在哪，没法跟着重算，而这比"一直糊"
//!   划算。缩放（滚轮）之后同理：那时 WebKit 会重新采样这张图，不再是 1:1，
//!   前端因此只在 1:1 时才认这份补偿（`src/react/pin/rendering.ts`）。
//! - 不便宜（见 [`MAX_COMPENSATED_PIXELS`]），所以**跑在后台线程上**：原图先上屏，
//!   算完了再换（`super::commands::spawn_sharpen`），开窗延迟不受影响。
//! - 复制与保存**永远用原图**，补偿只进贴图窗口的显示。

use anyhow::{Context, Result};

/// 反投影迭代次数。实测 1/2/3/4/6 轮分别是 38.95/41.08/42.27/42.94/43.55 dB，
/// 8 位缓冲区的上限在 43.7 dB 附近，第 4 轮之后每轮只剩零点几分贝，
/// 而每轮都是两次全图重采样，所以停在 4。
const BACK_PROJECTION_ROUNDS: usize = 4;

/// 缓冲区超过这么多像素就不补偿了，原图照旧显示。
///
/// 代价与像素数成正比：本机 release 实测，缓冲区 1600x1200 要 380 ms、3413x1920 要 1.9 s，
/// 出来的 PNG 分别是 3.8 MB 与 13 MB。这条上限放得下"整块 2560x1440 屏"那种最坏情况
/// （6.55 MP），再大的贴图（多屏拼接、超宽屏）就不值得为清晰度花那份时间和内存了。
const MAX_COMPENSATED_PIXELS: u64 = 7_000_000;

/// 一条成像链路上的两个尺寸。都是像素，都已经取整。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DisplayGeometry {
    /// WebKit 的缓冲区尺寸 = 内容 CSS 尺寸 × 整数缓冲区缩放。
    pub buffer: (u32, u32),
    /// 屏上实际占的物理像素 = 内容 CSS 尺寸 × 真实缩放。
    pub panel: (u32, u32),
}

/// 需要补偿吗？需要的话给出两个目标尺寸。
///
/// `buffer_scale` 是 GDK 报的整数缩放，`device_scale` 是问过合成器的真实缩放
/// （见 `crate::screenshot::desktop_scale_at`）。两者相等就没有合成器缩小那一步，
/// 直接返回 `None`：那时把图原样交给 WebKit 就是 1:1。
pub(super) fn display_geometry(
    content_width: f64,
    content_height: f64,
    device_scale: f64,
    buffer_scale: f64,
) -> Option<DisplayGeometry> {
    if !(content_width.is_finite() && content_height.is_finite()) {
        return None;
    }
    if !(device_scale.is_finite() && buffer_scale.is_finite()) {
        return None;
    }
    if device_scale <= 0.0 || buffer_scale <= 0.0 {
        return None;
    }
    if (buffer_scale - device_scale).abs() < 1e-3 {
        return None;
    }
    let buffer = (
        pixels(content_width * buffer_scale)?,
        pixels(content_height * buffer_scale)?,
    );
    if u64::from(buffer.0) * u64::from(buffer.1) > MAX_COMPENSATED_PIXELS {
        return None;
    }
    let panel = (
        pixels(content_width * device_scale)?,
        pixels(content_height * device_scale)?,
    );
    Some(DisplayGeometry { buffer, panel })
}

fn pixels(value: f64) -> Option<u32> {
    let rounded = value.round();
    if rounded < 1.0 || rounded > u32::MAX as f64 {
        return None;
    }
    Some(rounded as u32)
}

/// 把一张 PNG 换成"缓冲区尺寸 + 已补偿"的 PNG。
///
/// 失败一律返回 `Err`，调用方退回原图——清晰度差一点也好过贴不出来。
pub(super) fn compensated_png(png: &[u8], geometry: DisplayGeometry) -> Result<Vec<u8>> {
    let source = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .context("PNG 解码失败")?
        .to_rgba8();
    let (buffer_width, buffer_height) = geometry.buffer;
    let (panel_width, panel_height) = geometry.panel;

    // 屏上"应该"长什么样：源图按物理尺寸缩放一次（尺寸相同就是它自己）。
    let panel = if source.width() == panel_width && source.height() == panel_height {
        source.clone()
    } else {
        image::imageops::resize(
            &source,
            panel_width,
            panel_height,
            image::imageops::FilterType::Lanczos3,
        )
    };
    // 反投影的初值：Lanczos3 预放大。实测比双线性初值同轮次高约 1.4 dB。
    let initial = image::imageops::resize(
        &panel,
        buffer_width,
        buffer_height,
        image::imageops::FilterType::Lanczos3,
    );

    let buffer = back_project(
        &to_f32(panel.as_raw()),
        (panel_width, panel_height),
        to_f32(initial.as_raw()),
        (buffer_width, buffer_height),
        BACK_PROJECTION_ROUNDS,
    );
    crate::screenshot::encode_png(&to_u8(&buffer), buffer_width, buffer_height)
}

/// 迭代反投影：让"缓冲区图被合成器缩小之后"尽量等于 `panel`。
fn back_project(
    panel: &[f32],
    panel_size: (u32, u32),
    mut buffer: Vec<f32>,
    buffer_size: (u32, u32),
    rounds: usize,
) -> Vec<f32> {
    for _ in 0..rounds {
        let shown = resample_bilinear(&buffer, buffer_size, panel_size);
        let mut residual = shown;
        for (value, target) in residual.iter_mut().zip(panel) {
            *value = target - *value;
        }
        let correction = resample_bilinear(&residual, panel_size, buffer_size);
        for (value, delta) in buffer.iter_mut().zip(&correction) {
            *value = (*value + delta).clamp(0.0, 255.0);
        }
    }
    buffer
}

/// 双线性重采样，RGBA、可分离（先横后纵）。
///
/// 映射规则是"输出像素中心映射回输入坐标"：`u = (i + 0.5) * in / out - 0.5`。
/// 这正是 Mutter 缩小整张画面时用的那一个（脉冲实测见模块头），反投影的前向模型
/// 必须和它逐字一致，否则补偿量会偏。
fn resample_bilinear(source: &[f32], from: (u32, u32), to: (u32, u32)) -> Vec<f32> {
    let (in_width, in_height) = (from.0 as usize, from.1 as usize);
    let (out_width, out_height) = (to.0 as usize, to.1 as usize);
    let mut horizontal = vec![0.0f32; out_width * in_height * 4];
    let taps_x = taps(in_width, out_width);
    for row in 0..in_height {
        let source_row = &source[row * in_width * 4..(row + 1) * in_width * 4];
        let target_row = &mut horizontal[row * out_width * 4..(row + 1) * out_width * 4];
        for (column, tap) in taps_x.iter().enumerate() {
            for channel in 0..4 {
                target_row[column * 4 + channel] = source_row[tap.low * 4 + channel]
                    * (1.0 - tap.weight)
                    + source_row[tap.high * 4 + channel] * tap.weight;
            }
        }
    }
    let mut out = vec![0.0f32; out_width * out_height * 4];
    let taps_y = taps(in_height, out_height);
    for (row, tap) in taps_y.iter().enumerate() {
        let (low, high) = (tap.low * out_width * 4, tap.high * out_width * 4);
        let target_row = &mut out[row * out_width * 4..(row + 1) * out_width * 4];
        for index in 0..out_width * 4 {
            target_row[index] = horizontal[low + index] * (1.0 - tap.weight)
                + horizontal[high + index] * tap.weight;
        }
    }
    out
}

struct Tap {
    low: usize,
    high: usize,
    weight: f32,
}

fn taps(input: usize, output: usize) -> Vec<Tap> {
    (0..output)
        .map(|index| {
            let position = (index as f64 + 0.5) * input as f64 / output as f64 - 0.5;
            let low = position.floor();
            let weight = (position - low) as f32;
            let low = low.max(0.0) as usize;
            Tap {
                low: low.min(input - 1),
                high: (low + 1).min(input - 1),
                weight: if position < 0.0 { 0.0 } else { weight },
            }
        })
        .collect()
}

fn to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes.iter().map(|value| f32::from(*value)).collect()
}

fn to_u8(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .map(|value| value.round().clamp(0.0, 255.0) as u8)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 缓冲区缩放等于真实缩放（X11、整数缩放桌面）时不该动图片。
    #[test]
    fn no_geometry_when_scales_match() {
        assert!(display_geometry(800.0, 600.0, 1.0, 1.0).is_none());
        assert!(display_geometry(800.0, 600.0, 2.0, 2.0).is_none());
        // 数字不可信时也不做
        assert!(display_geometry(800.0, 600.0, 0.0, 2.0).is_none());
        assert!(display_geometry(f64::NAN, 600.0, 1.5, 2.0).is_none());
    }

    /// 1.5 倍缩放桌面：CSS 800x600 → 缓冲区 1600x1200、屏上 1200x900。
    #[test]
    fn geometry_follows_both_scales() {
        let geometry = display_geometry(800.0, 600.0, 1.5, 2.0).expect("需要补偿");
        assert_eq!(geometry.buffer, (1600, 1200));
        assert_eq!(geometry.panel, (1200, 900));
    }

    /// 大到不值得补偿的贴图直接放弃，让原图照旧显示。
    /// 边界就在"整块 2560x1440 屏"上方：那个最坏情况必须仍然被补偿。
    #[test]
    fn oversized_buffers_are_left_alone() {
        // 2560x1440 物理的整屏贴图：CSS 1706.67 → 缓冲区 3413x1920 = 6.55 MP，要做。
        assert!(display_geometry(2560.0 / 1.5, 1440.0 / 1.5, 1.5, 2.0).is_some());
        // 再宽一截（多屏拼接）就超预算了。
        assert!(display_geometry(2560.0, 1440.0, 1.5, 2.0).is_none());
    }

    /// 重采样的映射必须和合成器一致：缓冲区里一个孤立白点，按 0.75 缩小之后
    /// 只在**一个**输出像素上留下 0.8333² ≈ 0.694 的亮度（实测 177/255）。
    #[test]
    fn bilinear_matches_the_measured_compositor_kernel() {
        let (width, height) = (40u32, 40u32);
        let mut buffer = vec![0.0f32; (width * height * 4) as usize];
        let index = ((20 * width + 20) * 4) as usize;
        for channel in 0..4 {
            buffer[index + channel] = 255.0;
        }
        let shown = resample_bilinear(&buffer, (width, height), (30, 30));
        let at = |x: usize, y: usize| shown[(y * 30 + x) * 4];
        assert!((at(15, 15) - 177.08).abs() < 0.5, "{}", at(15, 15));
        assert_eq!(at(14, 15), 0.0);
        assert_eq!(at(16, 15), 0.0);
        assert_eq!(at(15, 14), 0.0);
    }

    /// 反投影确实让"缩小之后"更接近目标：合成 4/3 放大的链路上，
    /// 补偿后的 PSNR 必须明显高于只做 Lanczos 预放大。
    #[test]
    fn back_projection_beats_plain_prescale() {
        let (panel_width, panel_height) = (120u32, 90u32);
        let (buffer_width, buffer_height) = (160u32, 120u32);
        // 造一张有高频细节的图：细网格 + 斜边，正是最容易被平滑掉的东西
        let mut panel = vec![0.0f32; (panel_width * panel_height * 4) as usize];
        for y in 0..panel_height {
            for x in 0..panel_width {
                let index = ((y * panel_width + x) * 4) as usize;
                let value = if x % 3 == 0 || y % 4 == 0 || x == y {
                    240.0
                } else {
                    16.0
                };
                for channel in 0..3 {
                    panel[index + channel] = value;
                }
                panel[index + 3] = 255.0;
            }
        }
        let panel_image =
            image::RgbaImage::from_raw(panel_width, panel_height, to_u8(&panel)).expect("构造图片");
        let initial = to_f32(
            image::imageops::resize(
                &panel_image,
                buffer_width,
                buffer_height,
                image::imageops::FilterType::Lanczos3,
            )
            .as_raw(),
        );

        let psnr = |buffer: &[f32]| {
            let shown = resample_bilinear(
                buffer,
                (buffer_width, buffer_height),
                (panel_width, panel_height),
            );
            let mse: f64 = shown
                .iter()
                .zip(&panel)
                .map(|(a, b)| f64::from(a - b) * f64::from(a - b))
                .sum::<f64>()
                / shown.len() as f64;
            10.0 * (255.0f64 * 255.0 / mse).log10()
        };

        let plain = psnr(&initial);
        let compensated = psnr(&back_project(
            &panel,
            (panel_width, panel_height),
            initial.clone(),
            (buffer_width, buffer_height),
            BACK_PROJECTION_ROUNDS,
        ));
        assert!(
            compensated > plain + 4.0,
            "补偿 {compensated:.2} dB 应当明显好过纯预放大 {plain:.2} dB"
        );
    }

    /// 补偿这一步的耗时，用来定 [`MAX_COMPENSATED_PIXELS`]。默认不跑（数字与机器相关，
    /// 而且 debug 构建比 release 慢一个量级），要看就
    /// `cargo test --release --lib compensation_cost -- --ignored --nocapture`。
    #[test]
    #[ignore = "性能探针，只在需要重新定阈值时手动跑"]
    fn compensation_cost() {
        for (panel_width, panel_height) in [(1200u32, 900u32), (2160, 1215), (2560, 1440)] {
            let mut source = image::RgbaImage::new(panel_width, panel_height);
            for (x, y, pixel) in source.enumerate_pixels_mut() {
                let value = ((x * 7 + y * 13) % 256) as u8;
                *pixel = image::Rgba([value, value.wrapping_add(80), 40, 255]);
            }
            let png =
                crate::screenshot::encode_png(source.as_raw(), panel_width, panel_height).unwrap();
            let geometry = DisplayGeometry {
                buffer: (panel_width * 4 / 3, panel_height * 4 / 3),
                panel: (panel_width, panel_height),
            };
            let started = std::time::Instant::now();
            let out = compensated_png(&png, geometry).unwrap();
            println!(
                "屏上 {panel_width}x{panel_height} → 缓冲区 {:?}：{:?}，PNG {} KiB",
                geometry.buffer,
                started.elapsed(),
                out.len() / 1024
            );
        }
    }

    /// 端到端：一张 PNG 进去，缓冲区尺寸的 PNG 出来。
    #[test]
    fn compensated_png_has_buffer_dimensions() {
        let mut source = image::RgbaImage::new(120, 90);
        for (x, y, pixel) in source.enumerate_pixels_mut() {
            let value = if (x / 2 + y / 3) % 2 == 0 { 230 } else { 25 };
            *pixel = image::Rgba([value, value, value, 255]);
        }
        let png = crate::screenshot::encode_png(source.as_raw(), 120, 90).expect("编码");
        let geometry = DisplayGeometry {
            buffer: (160, 120),
            panel: (120, 90),
        };
        let compensated = compensated_png(&png, geometry).expect("补偿");
        assert_eq!(
            crate::screenshot::png_dimensions(&compensated).expect("读头"),
            (160, 120)
        );
    }
}
