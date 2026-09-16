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
//! | **同上再做 4 轮反投影补偿（本模块）** | **43.02 dB** |
//!
//! 前三行都在"能看出糊"的量级上，只有最后一行到了肉眼分不出来的程度——所以这里做的是
//! 最后一行：先把图渲染成缓冲区尺寸，再迭代地问"这张缓冲区图被合成器缩完等于原图吗"，
//! 把差值加回去。合成器那一步的核也是实测出来的：缓冲区里一个孤立白点在屏上只留下
//! **一个** 177/255 的点（= 0.8333²），正是"输出像素中心映射回输入坐标"的标准双线性，
//! 与 [`resample_bilinear`] 完全一致，所以反投影用的前向模型是准的：拿本模块真正输出的
//! 那串字节离线预测 43.09 dB，贴到屏上实测 43.02 dB，对得上。
//!
//! # 边界
//!
//! - 只在缓冲区缩放与真实缩放**不相等**时才做（X11、整数缩放的桌面本来就是 1:1）。
//! - 补偿是**按当前那块屏**算的。把贴图拖到缩放不同的另一块屏上，补偿量就偏了，
//!   看起来会有一点过锐——Wayland 不告诉客户端窗口在哪，没法跟着重算，而这比"一直糊"
//!   划算。缩放（滚轮）之后同理：那时 WebKit 会重新采样这张图，不再是 1:1，
//!   前端因此只在 1:1 时才认这份补偿（`src/react/pin/rendering.ts`）。
//! - 不便宜，所以**跑在后台线程上，而且从建条目那一刻
//!   就开跑**，和建窗、WebKit 起步并行；赶上了第一帧就是清楚的，没赶上才是"原图先上屏、
//!   算完换图"（`super::commands::spawn_sharpen`）。两种情况下开窗延迟都不受影响。
//! - 生产算法以 `u8` 保留目标、Q7 `u16` 保留缓冲图，只用一份 `i16` 残差；Lanczos 逐行缓存、
//!   双线性逐行前向/回投影，不再同时分配多份全图 `f32 RGBA`。因此 4K 分数
//!   缩放可以完整跑 4 轮，而不会像旧实现那样把峰值推到接近 1 GiB。
//! - 复制与保存**永远用原图**，补偿只进贴图窗口的显示。

use anyhow::{Context, Result};

/// 反投影迭代次数。实测 1/2/3/4/6 轮分别是 38.95/41.08/42.27/42.94/43.55 dB，
/// 8 位缓冲区的上限在 43.7 dB 附近，第 4 轮之后每轮只剩零点几分贝，
/// 而每轮都是两次全图重采样，所以停在 4。
const BACK_PROJECTION_ROUNDS: usize = 4;

/// 反投影内部的定点小数精度。`u16` 保留 7 位小数，而残差的
/// 最大幅度刚好落在 `i16` 内；不会像每轮回写 `u8` 那样丢失子像素精度。
const FIXED_SCALE: f32 = 128.0;
const FIXED_SCALE_I32: i32 = 128;
const FIXED_MAX: u16 = 255 * FIXED_SCALE_I32 as u16;

/// 单份 RGBA 平面的硬预算。用字节而不是一个没有语义的像素阈值表达：
/// 64 MiB 可完整容纳 5120x2880（56.25 MiB），即 3840x2160 显示器在
/// device=1.5 / buffer=2 下的全尺寸 Pin。超过这个防御性边界时由
/// [`compensated_png_after_wait`] 返回明确错误，后台路径会记录原因，不再在几何阶段静默跳过。
const MAX_RGBA_PLANE_BYTES: usize = 64 * 1024 * 1024;

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
#[cfg(test)]
pub(super) fn compensated_png(png: &[u8], geometry: DisplayGeometry) -> Result<Vec<u8>> {
    let _guard = compensation_guard();
    compensated_png_inner(png, geometry)
}

/// 后台 Pin 专用入口：等到全局工作位后先查取消，已关闭的排队窗口不再耗费
/// 1s 级 CPU。当前正在算的一张最多跑完本轮，不在像素内循环里加原子分支。
pub(super) fn compensated_png_after_wait(
    png: &[u8],
    geometry: DisplayGeometry,
    cancelled: impl FnOnce() -> bool,
) -> Result<Option<Vec<u8>>> {
    let _guard = compensation_guard();
    if cancelled() {
        return Ok(None);
    }
    compensated_png_inner(png, geometry).map(Some)
}

fn compensation_guard() -> std::sync::MutexGuard<'static, ()> {
    // 同时开多张 Pin 时只让一张占用补偿工作集。等待发生在后台线程，
    // 窗口仍立刻用原图显示；串行化把进程峰值锁在单张图的预算内。
    static COMPENSATION_GATE: std::sync::OnceLock<std::sync::Mutex<()>> =
        std::sync::OnceLock::new();
    COMPENSATION_GATE
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn compensated_png_inner(png: &[u8], geometry: DisplayGeometry) -> Result<Vec<u8>> {
    checked_rgba_len(geometry.panel).context("贴图屏显目标超出 64 MiB RGBA 预算")?;
    checked_rgba_len(geometry.buffer).context("贴图 WebKit 缓冲区超出 64 MiB RGBA 预算")?;
    let source_size = crate::screenshot::png_dimensions(png).context("贴图 PNG 头无效")?;
    checked_rgba_len(source_size).context("贴图原图超出 64 MiB RGBA 预算")?;
    let source = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .context("PNG 解码失败")?
        .to_rgba8();

    // 屏上"应该"长什么样：源图按物理尺寸重采样一次（尺寸相同就是它自己）。
    let panel = if source_size == geometry.panel {
        std::borrow::Cow::Borrowed(source.as_raw().as_slice())
    } else {
        std::borrow::Cow::Owned(resample_lanczos3_u8(
            source.as_raw(),
            source_size,
            geometry.panel,
        ))
    };
    // 反投影的初值：Lanczos3 预放大。双线性初值便宜但收敛到的上限低 3 dB 左右
    // （同一张截图上 44.33 dB 对 47.66 dB），差的那部分再多迭代也补不回来。
    let initial = resample_lanczos3_fixed(&panel, geometry.panel, geometry.buffer);
    let buffer = back_project_fixed(
        &panel,
        geometry.panel,
        initial,
        geometry.buffer,
        BACK_PROJECTION_ROUNDS,
    );
    let buffer = fixed_to_u8(&buffer);
    crate::screenshot::encode_png(&buffer, geometry.buffer.0, geometry.buffer.1)
}

fn checked_rgba_len(size: (u32, u32)) -> Result<usize> {
    let bytes = usize::try_from(size.0)
        .ok()
        .and_then(|width| {
            usize::try_from(size.1)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .context("RGBA 尺寸溢出")?;
    anyhow::ensure!(bytes <= MAX_RGBA_PLANE_BYTES, "{bytes} 字节");
    Ok(bytes)
}

/// 迭代反投影：让"缓冲区图被合成器缩小之后"尽量等于 `panel`。
fn back_project_fixed(
    panel: &[u8],
    panel_size: (u32, u32),
    mut buffer: Vec<u16>,
    buffer_size: (u32, u32),
    rounds: usize,
) -> Vec<u16> {
    let mut residual = vec![0i16; panel.len()];
    let panel_x = bilinear_taps(buffer_size.0 as usize, panel_size.0 as usize);
    let panel_y = bilinear_taps(buffer_size.1 as usize, panel_size.1 as usize);
    let buffer_x = bilinear_taps(panel_size.0 as usize, buffer_size.0 as usize);
    let buffer_y = bilinear_taps(panel_size.1 as usize, buffer_size.1 as usize);
    for _ in 0..rounds {
        for_each_row(&mut residual, panel_size.0 as usize * 4, |row, target| {
            let y = &panel_y[row];
            for (column, x) in panel_x.iter().enumerate() {
                for channel in 0..4 {
                    let shown =
                        bilinear_sample_u16(&buffer, buffer_size.0 as usize, x, y, column, channel);
                    let index = column * 4 + channel;
                    target[index] =
                        (f32::from(panel[row * panel_size.0 as usize * 4 + index]) * FIXED_SCALE
                            - shown)
                            .round()
                            .clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                }
            }
        });
        for_each_row(&mut buffer, buffer_size.0 as usize * 4, |row, target| {
            let y = &buffer_y[row];
            for (column, x) in buffer_x.iter().enumerate() {
                for channel in 0..4 {
                    let correction = bilinear_sample_i16(
                        &residual,
                        panel_size.0 as usize,
                        x,
                        y,
                        column,
                        channel,
                    );
                    let index = column * 4 + channel;
                    target[index] = (f32::from(target[index]) + correction)
                        .round()
                        .clamp(0.0, f32::from(FIXED_MAX))
                        as u16;
                }
            }
        });
    }
    buffer
}

fn bilinear_sample_u16(
    source: &[u16],
    width: usize,
    x: &BilinearTap,
    y: &BilinearTap,
    _column: usize,
    channel: usize,
) -> f32 {
    bilinear_sample(source, width, x, y, channel, |value| f32::from(*value))
}

fn bilinear_sample_i16(
    source: &[i16],
    width: usize,
    x: &BilinearTap,
    y: &BilinearTap,
    _column: usize,
    channel: usize,
) -> f32 {
    bilinear_sample(source, width, x, y, channel, |value| f32::from(*value))
}

fn bilinear_sample<T>(
    source: &[T],
    width: usize,
    x: &BilinearTap,
    y: &BilinearTap,
    channel: usize,
    value: impl Fn(&T) -> f32,
) -> f32 {
    let at = |row: usize, column: usize| value(&source[(row * width + column) * 4 + channel]);
    let top = at(y.low, x.low) * (1.0 - x.weight) + at(y.low, x.high) * x.weight;
    let bottom = at(y.high, x.low) * (1.0 - x.weight) + at(y.high, x.high) * x.weight;
    top * (1.0 - y.weight) + bottom * y.weight
}

/// 双线性重采样，RGBA、可分离（先横后纵），两趟都按行并行。
///
/// 映射规则是"输出像素中心映射回输入坐标"：`u = (i + 0.5) * in / out - 0.5`，
/// 而且**缩小时也不把核拉宽**。这正是 Mutter 缩小整张画面时用的那一个（脉冲实测见
/// 模块头），反投影的前向模型必须和它逐字一致，否则补偿量会偏——所以这里既不能换成
/// 下面那个通用实现（它按缩放比拉宽核），也不能"顺手改成更正确的面积平均"。
///
/// 反投影每轮要走它两次，是整个补偿里最热的一段，因此专门写死成两点采样：
/// 通用实现每个输出像素要过一层 `Vec` 间接，实测慢 4~5 倍。
#[cfg(test)]
fn resample_bilinear(source: &[f32], from: (u32, u32), to: (u32, u32)) -> Vec<f32> {
    let (in_width, in_height) = (from.0 as usize, from.1 as usize);
    let (out_width, out_height) = (to.0 as usize, to.1 as usize);

    let taps_x = bilinear_taps(in_width, out_width);
    let mut horizontal = vec![0.0f32; out_width * in_height * 4];
    for_each_row(&mut horizontal, out_width * 4, |row, target| {
        let source_row = &source[row * in_width * 4..(row + 1) * in_width * 4];
        for (column, tap) in taps_x.iter().enumerate() {
            let low = &source_row[tap.low * 4..tap.low * 4 + 4];
            let high = &source_row[tap.high * 4..tap.high * 4 + 4];
            let target_pixel = &mut target[column * 4..column * 4 + 4];
            for channel in 0..4 {
                target_pixel[channel] =
                    low[channel] * (1.0 - tap.weight) + high[channel] * tap.weight;
            }
        }
    });

    let taps_y = bilinear_taps(in_height, out_height);
    let mut out = vec![0.0f32; out_width * out_height * 4];
    for_each_row(&mut out, out_width * 4, |row, target| {
        let tap = &taps_y[row];
        let low = &horizontal[tap.low * out_width * 4..(tap.low + 1) * out_width * 4];
        let high = &horizontal[tap.high * out_width * 4..(tap.high + 1) * out_width * 4];
        for ((value, above), below) in target.iter_mut().zip(low).zip(high) {
            *value = above * (1.0 - tap.weight) + below * tap.weight;
        }
    });
    out
}

struct BilinearTap {
    low: usize,
    high: usize,
    weight: f32,
}

fn bilinear_taps(input: usize, output: usize) -> Vec<BilinearTap> {
    (0..output)
        .map(|index| {
            let position = (index as f64 + 0.5) * input as f64 / output as f64 - 0.5;
            let low = position.floor();
            let weight = (position - low) as f32;
            let low = low.max(0.0) as usize;
            BilinearTap {
                low: low.min(input - 1),
                high: (low + 1).min(input - 1),
                weight: if position < 0.0 { 0.0 } else { weight },
            }
        })
        .collect()
}

/// Lanczos3 重采样，结果直接保留为 8 位 RGBA。
///
/// 旧算法先分配整张横向 `f32` 中间图，再分配整张 `f32` 输出；4K 下光这
/// 两份就超过 400 MiB。这里每个并行分块只缓存垂直核当前用到的几行横向
/// `f32` 结果，整图输出一直是 `u8`。缩小时核仍按比例拉宽，采样语义不变。
fn resample_lanczos3_u8(source: &[u8], from: (u32, u32), to: (u32, u32)) -> Vec<u8> {
    fixed_to_u8(&resample_lanczos3_fixed(source, from, to))
}

fn resample_lanczos3_fixed(source: &[u8], from: (u32, u32), to: (u32, u32)) -> Vec<u16> {
    let (in_width, in_height) = (from.0 as usize, from.1 as usize);
    let (out_width, out_height) = (to.0 as usize, to.1 as usize);
    let taps_x = lanczos3_taps(in_width, out_width);
    let taps_y = lanczos3_taps(in_height, out_height);
    let mut out = vec![0u16; out_width * out_height * 4];
    let blocks = worker_count(out_height);
    let block_rows = out_height.div_ceil(blocks);
    std::thread::scope(|scope| {
        for (block, chunk) in out.chunks_mut(block_rows * out_width * 4).enumerate() {
            let first_row = block * block_rows;
            let taps_x = &taps_x;
            let taps_y = &taps_y;
            scope.spawn(move || {
                let mut horizontal = std::collections::BTreeMap::<usize, Vec<f32>>::new();
                for (offset, target) in chunk.chunks_mut(out_width * 4).enumerate() {
                    let row = first_row + offset;
                    let vertical = &taps_y[row];
                    for &(source_row, _) in vertical {
                        horizontal.entry(source_row).or_insert_with(|| {
                            lanczos_horizontal_row(source, in_width, source_row, out_width, taps_x)
                        });
                    }
                    for value in target.iter_mut() {
                        *value = 0;
                    }
                    for column in 0..out_width {
                        for channel in 0..4 {
                            let value = vertical
                                .iter()
                                .map(|&(source_row, weight)| {
                                    horizontal[&source_row][column * 4 + channel] * weight
                                })
                                .sum::<f32>();
                            target[column * 4 + channel] = (value * FIXED_SCALE)
                                .round()
                                .clamp(0.0, f32::from(FIXED_MAX))
                                as u16;
                        }
                    }
                    if let Some(&(oldest_needed, _)) = vertical.first() {
                        horizontal.retain(|source_row, _| *source_row >= oldest_needed);
                    }
                }
            });
        }
    });
    out
}

fn fixed_to_u8(values: &[u16]) -> Vec<u8> {
    values
        .iter()
        .map(|value| {
            ((u32::from(*value) + (FIXED_SCALE_I32 as u32 / 2)) / FIXED_SCALE_I32 as u32) as u8
        })
        .collect()
}

fn lanczos_horizontal_row(
    source: &[u8],
    in_width: usize,
    row: usize,
    out_width: usize,
    taps_x: &[Tap],
) -> Vec<f32> {
    let source_row = &source[row * in_width * 4..(row + 1) * in_width * 4];
    let mut target = vec![0.0f32; out_width * 4];
    for (column, tap) in taps_x.iter().enumerate() {
        for channel in 0..4 {
            target[column * 4 + channel] = tap
                .iter()
                .map(|&(index, weight)| f32::from(source_row[index * 4 + channel]) * weight)
                .sum();
        }
    }
    target
}

/// 一个输出像素的采样点：输入下标 + 权重，权重之和为 1。
type Tap = Vec<(usize, f32)>;

fn lanczos3_taps(input: usize, output: usize) -> Vec<Tap> {
    let ratio = (input as f64 / output as f64).max(1.0);
    let support = 3.0 * ratio;
    (0..output)
        .map(|index| {
            let center = (index as f64 + 0.5) * input as f64 / output as f64 - 0.5;
            let first = ((center - support).ceil() as i64).max(0);
            let last = ((center + support).floor() as i64).min(input as i64 - 1);
            let mut tap: Tap = (first..=last)
                .filter_map(|position| {
                    let weight = lanczos3((center - position as f64) / ratio);
                    (weight != 0.0).then_some((position as usize, weight as f32))
                })
                .collect();
            // 边缘上核会被截掉一部分，归一化等于把缺的那份摊给留下来的采样点，
            // 也就是"边界像素向外延伸"。
            let total: f32 = tap.iter().map(|(_, weight)| weight).sum();
            if total.abs() > f32::EPSILON {
                for (_, weight) in tap.iter_mut() {
                    *weight /= total;
                }
            } else {
                tap = vec![(center.round().clamp(0.0, input as f64 - 1.0) as usize, 1.0)];
            }
            tap
        })
        .collect()
}

fn lanczos3(x: f64) -> f64 {
    let x = x.abs();
    if x < 1e-9 {
        1.0
    } else if x < 3.0 {
        let pi_x = std::f64::consts::PI * x;
        3.0 * pi_x.sin() * (pi_x / 3.0).sin() / (pi_x * pi_x)
    } else {
        0.0
    }
}

/// 把 `output` 按行切块，交给作用域线程各算一块。
///
/// 重采样的每一行都只读输入、只写自己那一行，天然可并行；本机 18 核上补偿因此从
/// 秒级掉到百毫秒级（见 [`compensation_cost`](tests::compensation_cost)）。用
/// `std::thread::scope` 而不是引一个线程池依赖：这段路一次贴图只走一趟，
/// 建线程的几十微秒相比整张图的重采样可以忽略。
fn for_each_row<T: Send>(
    output: &mut [T],
    row_len: usize,
    work: impl Fn(usize, &mut [T]) + Send + Sync,
) {
    let rows = output.len() / row_len;
    let blocks = worker_count(rows);
    if blocks <= 1 {
        for (row, target) in output.chunks_mut(row_len).enumerate() {
            work(row, target);
        }
        return;
    }
    let block_rows = rows.div_ceil(blocks);
    let work = &work;
    std::thread::scope(|scope| {
        for (block, chunk) in output.chunks_mut(block_rows * row_len).enumerate() {
            scope.spawn(move || {
                for (row, target) in chunk.chunks_mut(row_len).enumerate() {
                    work(block * block_rows + row, target);
                }
            });
        }
    });
}

/// 切几块。每块至少 32 行，行数少的小图不值得摊到所有核上。
fn worker_count(rows: usize) -> usize {
    static CORES: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let cores = *CORES.get_or_init(|| {
        std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(1)
    });
    cores.min(rows.div_ceil(32)).max(1)
}

#[cfg(test)]
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

    /// 3840x2160 原生屏在 logical 2560x1440、device=1.5 / buffer=2 下，
    /// 全尺寸 Pin 的 WebKit 缓冲区是 5120x2880。这条曾被 7 MP 阈值静默跳过。
    #[test]
    fn native_4k_fractional_scale_is_compensated() {
        let geometry = display_geometry(2560.0, 1440.0, 1.5, 2.0).expect("4K 必须补偿");
        assert_eq!(geometry.panel, (3840, 2160));
        assert_eq!(geometry.buffer, (5120, 2880));
        assert_eq!(
            checked_rgba_len(geometry.buffer).expect("在预算内"),
            58_982_400
        );
    }

    /// 全屏 origin 因工作区/控件边距缩小后仍要补偿，而不是因 `scale == 1`
    /// 误判。按 `window::origin_content_size` 的实际常量，2560x1440 工作区可用
    /// 2492x1368，全屏 16:9 origin 按高度缩成 2432x1368。
    #[test]
    fn workspace_shrunk_4k_origin_is_still_compensated() {
        let geometry = display_geometry(2432.0, 1368.0, 1.5, 2.0).expect("缩小后仍需补偿");
        assert_eq!(geometry.panel, (3648, 2052));
        assert_eq!(geometry.buffer, (4864, 2736));
        assert!(checked_rgba_len(geometry.buffer).is_ok());
    }

    #[test]
    fn allocations_above_the_explicit_plane_budget_are_rejected() {
        assert!(checked_rgba_len((4096, 4096)).is_ok());
        assert!(checked_rgba_len((4097, 4096)).is_err());
    }

    #[test]
    fn cancelled_queue_item_exits_before_png_decode() {
        let result = compensated_png_after_wait(
            "这不是 PNG，若继续必然报错".as_bytes(),
            DisplayGeometry {
                panel: (1, 1),
                buffer: (2, 2),
            },
            || true,
        )
        .expect("取消不是失败");
        assert!(result.is_none(), "已取消任务不应解码图片");
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
        let panel_u8 = to_u8(&panel);
        let initial = resample_lanczos3_fixed(
            &panel_u8,
            (panel_width, panel_height),
            (buffer_width, buffer_height),
        );

        let psnr = |buffer: &[u16]| {
            let buffer = buffer
                .iter()
                .map(|value| f32::from(*value) / FIXED_SCALE)
                .collect::<Vec<_>>();
            let shown = resample_bilinear(
                &buffer,
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
        let compensated = psnr(&back_project_fixed(
            &panel_u8,
            (panel_width, panel_height),
            initial.clone(),
            (buffer_width, buffer_height),
            BACK_PROJECTION_ROUNDS,
        ));
        assert!(
            compensated > plain + 4.0,
            "补偿 {compensated:.2} dB 应当明显好过纯预放大 {plain:.2} dB"
        );

        // 用改造前的全图 f32 算法在同一 fixture 上做基线；有界内存实现不能以
        // 量化为代价偷掉原有画质。
        let baseline_initial = baseline_lanczos3(
            &panel,
            (panel_width, panel_height),
            (buffer_width, buffer_height),
        );
        let baseline = baseline_back_project(
            &panel,
            (panel_width, panel_height),
            baseline_initial,
            (buffer_width, buffer_height),
            BACK_PROJECTION_ROUNDS,
        );
        let baseline_psnr = {
            let shown = resample_bilinear(
                &baseline,
                (buffer_width, buffer_height),
                (panel_width, panel_height),
            );
            let mse = shown
                .iter()
                .zip(&panel)
                .map(|(a, b)| f64::from(a - b).powi(2))
                .sum::<f64>()
                / shown.len() as f64;
            10.0 * (255.0f64 * 255.0 / mse).log10()
        };
        println!("同 fixture PSNR：旧 f32 {baseline_psnr:.3} dB，新 Q7 {compensated:.3} dB");
        assert!(
            compensated + 0.05 >= baseline_psnr,
            "新路径 {compensated:.3} dB 不得低于旧路径 {baseline_psnr:.3} dB"
        );
    }

    fn baseline_back_project(
        panel: &[f32],
        panel_size: (u32, u32),
        mut buffer: Vec<f32>,
        buffer_size: (u32, u32),
        rounds: usize,
    ) -> Vec<f32> {
        for _ in 0..rounds {
            let mut residual = resample_bilinear(&buffer, buffer_size, panel_size);
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

    fn baseline_lanczos3(source: &[f32], from: (u32, u32), to: (u32, u32)) -> Vec<f32> {
        let (in_width, in_height) = (from.0 as usize, from.1 as usize);
        let (out_width, out_height) = (to.0 as usize, to.1 as usize);
        let taps_x = lanczos3_taps(in_width, out_width);
        let mut horizontal = vec![0.0f32; out_width * in_height * 4];
        for_each_row(&mut horizontal, out_width * 4, |row, target| {
            let source_row = &source[row * in_width * 4..(row + 1) * in_width * 4];
            for (column, tap) in taps_x.iter().enumerate() {
                for channel in 0..4 {
                    target[column * 4 + channel] = tap
                        .iter()
                        .map(|&(index, weight)| source_row[index * 4 + channel] * weight)
                        .sum();
                }
            }
        });
        let taps_y = lanczos3_taps(in_height, out_height);
        let mut out = vec![0.0f32; out_width * out_height * 4];
        for_each_row(&mut out, out_width * 4, |row, target| {
            for &(source_row, weight) in &taps_y[row] {
                let source_row =
                    &horizontal[source_row * out_width * 4..(source_row + 1) * out_width * 4];
                for (value, sample) in target.iter_mut().zip(source_row) {
                    *value += sample * weight;
                }
            }
        });
        out
    }

    /// 补偿这一步的耗时，用来复验后台时间预算。默认不跑（数字与机器相关，
    /// 而且 debug 构建比 release 慢一个量级），要看就
    /// `cargo test --release --lib compensation_cost -- --ignored --nocapture`。
    #[test]
    #[ignore = "性能探针，只在需要重新定阈值时手动跑"]
    fn compensation_cost() {
        for (panel_width, panel_height) in [(1200u32, 900u32), (2560, 1440), (3840, 2160)] {
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
