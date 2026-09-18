//! 相邻长截图帧的四方向位移估计。
//!
//! 这里的常量是 Clippy 自有的工程参数。它刻意不复制任何外部应用的私有评分实现。

use super::{validate_dimensions, CaptureError};
use image::{imageops, RgbaImage};

const MAX_MATCH_WIDTH: usize = 512;
const MAX_ESTIMATE_HEIGHT: u32 = 16_384;
const MIN_WIDTH: u32 = 8;
const MIN_HEIGHT: u32 = 24;
const COARSE_COLUMNS: usize = 12;
const COARSE_ROWS: usize = 24;
const FINE_COLUMNS: usize = 96;
const FINE_ROWS: usize = 48;
const TEXTURE_COLUMNS: usize = 96;
const TEXTURE_ROWS: usize = 64;
const MAX_FINE_CANDIDATES: usize = 64;
const AXIS_HYSTERESIS_ABSOLUTE: f32 = 0.002;
const AXIS_HYSTERESIS_RATIO: f32 = 1.15;
const DIRECTION_AMBIGUITY_MARGIN: f32 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::capture) enum ScrollAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::capture) enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

impl ScrollDirection {
    pub(super) fn axis(self) -> ScrollAxis {
        match self {
            Self::Up | Self::Down => ScrollAxis::Vertical,
            Self::Left | Self::Right => ScrollAxis::Horizontal,
        }
    }
}

#[derive(Clone, Copy)]
struct OverlapConfig {
    texture_threshold: f64,
    luma_threshold: f64,
    gradient_threshold: f64,
    ambiguity_margin: f64,
    retained_row_ratio: f64,
}

impl Default for OverlapConfig {
    fn default() -> Self {
        Self {
            texture_threshold: 4.0 / 255.0,
            luma_threshold: 18.0 / 255.0,
            gradient_threshold: 24.0 / 255.0,
            ambiguity_margin: 0.08,
            retained_row_ratio: 0.8,
        }
    }
}

/// 可直接交给 [`super::VerticalStitcher::append`] 的内部估计结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::capture) struct OverlapEstimate {
    pub(in crate::capture) overlap_rows: u32,
    pub(in crate::capture) displacement_rows: u32,
    pub(in crate::capture) score: f32,
    pub(in crate::capture) confidence: f32,
    pub(in crate::capture) sampled_width: u32,
    pub(in crate::capture) sampled_rows: u32,
    pub(in crate::capture) delta_x: i32,
    pub(in crate::capture) delta_y: i32,
    pub(in crate::capture) direction: ScrollDirection,
}

#[derive(Debug, Clone, Copy)]
struct DirectionCandidate {
    estimate: OverlapEstimate,
}

/// 复用经过验证的一维评分核心，分别检查上、下、左、右四个候选方向。
///
/// `preferred_axis` 只在两个候选质量接近时提供滞回，不会覆盖明显更好的另一主轴。
pub(super) fn estimate_translation(
    previous: &RgbaImage,
    incoming: &RgbaImage,
    preferred_axis: Option<ScrollAxis>,
) -> Result<OverlapEstimate, CaptureError> {
    validate_input(previous, incoming)?;
    let rotated_previous = imageops::rotate90(previous);
    let rotated_incoming = imageops::rotate90(incoming);
    let mut candidates = Vec::with_capacity(4);
    let mut first_error = None;

    for result in [
        estimate_direction(previous, incoming, ScrollDirection::Down),
        estimate_direction(incoming, previous, ScrollDirection::Up),
        estimate_direction(&rotated_previous, &rotated_incoming, ScrollDirection::Right),
        estimate_direction(&rotated_incoming, &rotated_previous, ScrollDirection::Left),
    ] {
        match result {
            Ok(estimate) => candidates.push(DirectionCandidate { estimate }),
            Err(error) => {
                first_error.get_or_insert(error);
            }
        };
    }
    if candidates.is_empty() {
        return Err(first_error.unwrap_or(CaptureError::LongshotEstimateLowSimilarity));
    }
    candidates.sort_unstable_by(|left, right| {
        left.estimate
            .score
            .total_cmp(&right.estimate.score)
            .then_with(|| {
                right
                    .estimate
                    .confidence
                    .total_cmp(&left.estimate.confidence)
            })
            .then_with(|| {
                direction_rank(left.estimate.direction)
                    .cmp(&direction_rank(right.estimate.direction))
            })
    });

    let raw_best = candidates[0];
    let selected = preferred_axis
        .and_then(|axis| {
            candidates
                .iter()
                .copied()
                .filter(|candidate| candidate.estimate.direction.axis() == axis)
                .min_by(|left, right| left.estimate.score.total_cmp(&right.estimate.score))
        })
        .filter(|preferred| {
            preferred.estimate.score
                <= raw_best.estimate.score * AXIS_HYSTERESIS_RATIO + AXIS_HYSTERESIS_ABSOLUTE
        })
        .unwrap_or(raw_best);

    if preferred_axis.is_none() {
        if let Some(runner_up) = candidates
            .iter()
            .copied()
            .find(|candidate| candidate.estimate.direction != selected.estimate.direction)
        {
            let denominator = runner_up.estimate.score.max(1e-6);
            let margin = (runner_up.estimate.score - selected.estimate.score) / denominator;
            if (0.0..DIRECTION_AMBIGUITY_MARGIN).contains(&margin) {
                return Err(CaptureError::LongshotEstimateAmbiguous);
            }
        }
    }
    Ok(selected.estimate)
}

fn estimate_direction(
    previous: &RgbaImage,
    incoming: &RgbaImage,
    direction: ScrollDirection,
) -> Result<OverlapEstimate, CaptureError> {
    let mut estimate = estimate_vertical_overlap(previous, incoming)?;
    let displacement = i32::try_from(estimate.displacement_rows)
        .map_err(|_| CaptureError::LongshotResourceLimit)?;
    let (delta_x, delta_y) = match direction {
        ScrollDirection::Up => (0, -displacement),
        ScrollDirection::Down => (0, displacement),
        ScrollDirection::Left => (-displacement, 0),
        ScrollDirection::Right => (displacement, 0),
    };
    estimate.delta_x = delta_x;
    estimate.delta_y = delta_y;
    estimate.direction = direction;
    Ok(estimate)
}

fn direction_rank(direction: ScrollDirection) -> u8 {
    match direction {
        ScrollDirection::Down => 0,
        ScrollDirection::Up => 1,
        ScrollDirection::Right => 2,
        ScrollDirection::Left => 3,
    }
}

#[derive(Clone)]
struct GrayFrame {
    width: usize,
    height: usize,
    luma: Vec<u8>,
    gradient: Vec<u16>,
}

#[derive(Clone, Copy)]
struct CoarseCandidate {
    displacement: usize,
    score: f64,
}

#[derive(Clone, Copy)]
struct FineCandidate {
    displacement: usize,
    luma_mae: f64,
    gradient_mae: f64,
    score: f64,
    sampled_rows: usize,
}

#[derive(Clone, Copy)]
struct RowScore {
    row: usize,
    luma_sum: u64,
    gradient_sum: u64,
    sample_count: usize,
    score: f64,
}

/// 估计 `previous` 与向下滚动后的 `incoming` 之间的垂直位移。
pub(super) fn estimate_vertical_overlap(
    previous: &RgbaImage,
    incoming: &RgbaImage,
) -> Result<OverlapEstimate, CaptureError> {
    let config = OverlapConfig::default();
    validate_input(previous, incoming)?;

    let previous = GrayFrame::from_rgba(previous)?;
    let incoming = GrayFrame::from_rgba(incoming)?;
    let previous_texture = previous.texture()?;
    let incoming_texture = incoming.texture()?;
    let min_texture = previous_texture.min(incoming_texture);
    if min_texture < config.texture_threshold {
        return Err(CaptureError::LongshotEstimateLowTexture);
    }

    let height = previous.height;
    let min_evidence_rows = min_evidence_rows(height);
    let max_displacement = height - min_evidence_rows;
    let mut candidates = Vec::new();
    candidates
        .try_reserve_exact(max_displacement + 1)
        .map_err(|_| CaptureError::LongshotAllocationFailed)?;
    for displacement in 0..=max_displacement {
        candidates.push(CoarseCandidate {
            displacement,
            score: coarse_score(&previous, &incoming, displacement),
        });
    }
    candidates.sort_unstable_by(|left, right| {
        left.score
            .total_cmp(&right.score)
            .then_with(|| left.displacement.cmp(&right.displacement))
    });
    candidates.truncate(MAX_FINE_CANDIDATES.min(candidates.len()));

    let mut fine_candidates = Vec::new();
    fine_candidates
        .try_reserve_exact(candidates.len())
        .map_err(|_| CaptureError::LongshotAllocationFailed)?;
    for candidate in candidates {
        fine_candidates.push(fine_score(
            &previous,
            &incoming,
            candidate.displacement,
            config,
        )?);
    }
    fine_candidates.sort_unstable_by(|left, right| {
        left.score
            .total_cmp(&right.score)
            .then_with(|| left.displacement.cmp(&right.displacement))
    });
    let best = fine_candidates
        .first()
        .copied()
        .ok_or(CaptureError::LongshotEstimateLowSimilarity)?;
    if best.luma_mae > config.luma_threshold || best.gradient_mae > config.gradient_threshold {
        return Err(CaptureError::LongshotEstimateLowSimilarity);
    }

    let second = fine_candidates
        .iter()
        .copied()
        .find(|candidate| candidate.displacement != best.displacement)
        .ok_or(CaptureError::LongshotEstimateAmbiguous)?;
    let margin = (second.score - best.score) / second.score.max(1e-12);
    if margin < config.ambiguity_margin {
        return Err(CaptureError::LongshotEstimateAmbiguous);
    }

    if best.displacement == 0 {
        return Err(CaptureError::LongshotEstimateNoExtension);
    }
    let allowed_displacement = (height * 2) / 3;
    if best.displacement > allowed_displacement {
        return Err(CaptureError::LongshotEstimateDisplacementTooLarge);
    }

    let absolute_quality = 1.0
        - (best.luma_mae / config.luma_threshold)
            .max(best.gradient_mae / config.gradient_threshold);
    let margin_quality = clamp_unit(margin / config.ambiguity_margin);
    let texture_quality = clamp_unit(min_texture / (32.0 / 255.0));
    let confidence =
        clamp_unit(0.5 * absolute_quality + 0.3 * margin_quality + 0.2 * texture_quality);
    let score = clamp_unit(0.7 * best.luma_mae + 0.3 * best.gradient_mae);
    Ok(OverlapEstimate {
        overlap_rows: u32::try_from(height - best.displacement)
            .map_err(|_| CaptureError::LongshotResourceLimit)?,
        displacement_rows: u32::try_from(best.displacement)
            .map_err(|_| CaptureError::LongshotResourceLimit)?,
        score: score as f32,
        confidence: confidence as f32,
        sampled_width: u32::try_from(previous.width)
            .map_err(|_| CaptureError::LongshotResourceLimit)?,
        sampled_rows: u32::try_from(best.sampled_rows)
            .map_err(|_| CaptureError::LongshotResourceLimit)?,
        delta_x: 0,
        delta_y: i32::try_from(best.displacement)
            .map_err(|_| CaptureError::LongshotResourceLimit)?,
        direction: ScrollDirection::Down,
    })
}

fn validate_input(previous: &RgbaImage, incoming: &RgbaImage) -> Result<(), CaptureError> {
    // 保持成对入口原有的全局优先级：任一帧为空时，不能先被另一帧的资源错误遮蔽。
    if previous.width() == 0
        || previous.height() == 0
        || incoming.width() == 0
        || incoming.height() == 0
    {
        return Err(CaptureError::LongshotFrameEmpty);
    }
    validate_dimensions(previous.width(), previous.height())?;
    validate_dimensions(incoming.width(), incoming.height())?;
    if previous.dimensions() != incoming.dimensions() {
        return Err(CaptureError::LongshotEstimateSizeMismatch);
    }
    validate_estimator_dimensions(previous)
}

/// 验证可作为会话首帧保存的单张估计器输入。
///
/// 成对估计必须先比较尺寸才检查最小估计尺寸，因此它使用本文件中的两段式 helper
/// 保持既有错误优先级；会话创建则直接复用此完整的单帧合同。
pub(super) fn validate_single_frame(image: &RgbaImage) -> Result<(), CaptureError> {
    validate_frame_nonempty_and_global(image)?;
    validate_estimator_dimensions(image)
}

pub(super) fn validate_frame_nonempty_and_global(image: &RgbaImage) -> Result<(), CaptureError> {
    if image.width() == 0 || image.height() == 0 {
        return Err(CaptureError::LongshotFrameEmpty);
    }
    validate_dimensions(image.width(), image.height())
}

fn validate_estimator_dimensions(image: &RgbaImage) -> Result<(), CaptureError> {
    if image.width() < MIN_WIDTH
        || image.height() < MIN_HEIGHT
        || image.height() > MAX_ESTIMATE_HEIGHT
    {
        return if image.height() > MAX_ESTIMATE_HEIGHT {
            Err(CaptureError::LongshotResourceLimit)
        } else {
            Err(CaptureError::LongshotEstimateTooSmall)
        };
    }
    Ok(())
}

impl GrayFrame {
    fn from_rgba(image: &RgbaImage) -> Result<Self, CaptureError> {
        let original_width =
            usize::try_from(image.width()).map_err(|_| CaptureError::LongshotResourceLimit)?;
        let width = original_width.min(MAX_MATCH_WIDTH);
        let height =
            usize::try_from(image.height()).map_err(|_| CaptureError::LongshotResourceLimit)?;
        let pixels = width
            .checked_mul(height)
            .ok_or(CaptureError::LongshotResourceLimit)?;
        let mut luma = Vec::new();
        luma.try_reserve_exact(pixels)
            .map_err(|_| CaptureError::LongshotAllocationFailed)?;
        for y in 0..height {
            for sampled_x in 0..width {
                let source_x = sample_position(sampled_x, width, original_width);
                let offset = y
                    .checked_mul(original_width)
                    .and_then(|index| index.checked_add(source_x))
                    .and_then(|index| index.checked_mul(4))
                    .ok_or(CaptureError::LongshotResourceLimit)?;
                let pixel = image
                    .as_raw()
                    .get(offset..offset + 3)
                    .ok_or(CaptureError::LongshotResourceLimit)?;
                let value = (u16::from(pixel[0]) * 77
                    + u16::from(pixel[1]) * 150
                    + u16::from(pixel[2]) * 29
                    + 128)
                    >> 8;
                luma.push(u8::try_from(value).map_err(|_| CaptureError::LongshotResourceLimit)?);
            }
        }

        let mut gradient = Vec::new();
        gradient
            .try_reserve_exact(pixels)
            .map_err(|_| CaptureError::LongshotAllocationFailed)?;
        for y in 0..height {
            for x in 0..width {
                let index = y * width + x;
                let right = if x + 1 < width {
                    u16::from(luma[index].abs_diff(luma[index + 1]))
                } else {
                    0
                };
                let down = if y + 1 < height {
                    u16::from(luma[index].abs_diff(luma[index + width]))
                } else {
                    0
                };
                gradient.push(right + down);
            }
        }
        Ok(Self {
            width,
            height,
            luma,
            gradient,
        })
    }

    fn texture(&self) -> Result<f64, CaptureError> {
        let rows = self.height.min(TEXTURE_ROWS);
        let columns = self.width.min(TEXTURE_COLUMNS);
        let count = rows
            .checked_mul(columns)
            .ok_or(CaptureError::LongshotResourceLimit)?;
        let mut sum = 0_u64;
        for row in 0..rows {
            let y = sample_position(row, rows, self.height);
            for column in 0..columns {
                let x = sample_position(column, columns, self.width);
                sum += u64::from(self.gradient[y * self.width + x]);
            }
        }
        Ok((sum as f64) / (count as f64 * 510.0))
    }

    /// 匹配窗口的末行没有可比较的下一行，纵向梯度必须归零，不能借到窗口外的新内容。
    fn comparison_gradient(&self, x: usize, y: usize, has_next_row: bool) -> u16 {
        let index = y * self.width + x;
        let right = if x + 1 < self.width {
            u16::from(self.luma[index].abs_diff(self.luma[index + 1]))
        } else {
            0
        };
        let down = if has_next_row && y + 1 < self.height {
            u16::from(self.luma[index].abs_diff(self.luma[index + self.width]))
        } else {
            0
        };
        right + down
    }
}

fn coarse_score(previous: &GrayFrame, incoming: &GrayFrame, displacement: usize) -> f64 {
    let evidence_rows = previous.height - displacement;
    let rows = evidence_rows.min(COARSE_ROWS);
    let columns = previous.width.min(COARSE_COLUMNS);
    let mut luma_sum = 0_u64;
    let mut gradient_sum = 0_u64;
    for row in 0..rows {
        let source_y = sample_position(row, rows, evidence_rows);
        let has_next_row = source_y + 1 < evidence_rows;
        for column in 0..columns {
            let x = sample_position(column, columns, previous.width);
            let previous_index = (source_y + displacement) * previous.width + x;
            let incoming_index = source_y * incoming.width + x;
            luma_sum +=
                u64::from(previous.luma[previous_index].abs_diff(incoming.luma[incoming_index]));
            gradient_sum += u64::from(
                previous
                    .comparison_gradient(x, source_y + displacement, has_next_row)
                    .abs_diff(incoming.comparison_gradient(x, source_y, has_next_row)),
            );
        }
    }
    let samples = (rows * columns) as f64;
    let luma = luma_sum as f64 / (samples * 255.0);
    let gradient = gradient_sum as f64 / (samples * 510.0);
    0.7 * luma + 0.3 * gradient
}

fn fine_score(
    previous: &GrayFrame,
    incoming: &GrayFrame,
    displacement: usize,
    config: OverlapConfig,
) -> Result<FineCandidate, CaptureError> {
    let evidence_rows = previous.height - displacement;
    let rows = evidence_rows.min(FINE_ROWS);
    let columns = previous.width.min(FINE_COLUMNS);
    let mut row_scores = Vec::new();
    row_scores
        .try_reserve_exact(rows)
        .map_err(|_| CaptureError::LongshotAllocationFailed)?;
    for row in 0..rows {
        let source_y = sample_position(row, rows, evidence_rows);
        let has_next_row = source_y + 1 < evidence_rows;
        let mut luma_sum = 0_u64;
        let mut gradient_sum = 0_u64;
        for column in 0..columns {
            let x = sample_position(column, columns, previous.width);
            let previous_index = (source_y + displacement) * previous.width + x;
            let incoming_index = source_y * incoming.width + x;
            luma_sum +=
                u64::from(previous.luma[previous_index].abs_diff(incoming.luma[incoming_index]));
            gradient_sum += u64::from(
                previous
                    .comparison_gradient(x, source_y + displacement, has_next_row)
                    .abs_diff(incoming.comparison_gradient(x, source_y, has_next_row)),
            );
        }
        let samples = columns as f64;
        let luma = luma_sum as f64 / (samples * 255.0);
        let gradient = gradient_sum as f64 / (samples * 510.0);
        row_scores.push(RowScore {
            row,
            luma_sum,
            gradient_sum,
            sample_count: columns,
            score: 0.7 * luma + 0.3 * gradient,
        });
    }
    row_scores.sort_unstable_by(|left, right| {
        left.score
            .total_cmp(&right.score)
            .then_with(|| left.row.cmp(&right.row))
    });
    let retained_rows = ((rows as f64) * config.retained_row_ratio).ceil() as usize;
    let retained_rows = retained_rows.max(1).min(row_scores.len());
    let mut luma_sum = 0_u64;
    let mut gradient_sum = 0_u64;
    let mut sample_count = 0_usize;
    for row in &row_scores[..retained_rows] {
        luma_sum = luma_sum
            .checked_add(row.luma_sum)
            .ok_or(CaptureError::LongshotResourceLimit)?;
        gradient_sum = gradient_sum
            .checked_add(row.gradient_sum)
            .ok_or(CaptureError::LongshotResourceLimit)?;
        sample_count = sample_count
            .checked_add(row.sample_count)
            .ok_or(CaptureError::LongshotResourceLimit)?;
    }
    let luma_mae = luma_sum as f64 / (sample_count as f64 * 255.0);
    let gradient_mae = gradient_sum as f64 / (sample_count as f64 * 510.0);
    Ok(FineCandidate {
        displacement,
        luma_mae,
        gradient_mae,
        score: 0.7 * luma_mae + 0.3 * gradient_mae,
        sampled_rows: retained_rows,
    })
}

fn min_evidence_rows(height: usize) -> usize {
    (height.div_ceil(8)).max(8)
}

/// 在两端保持不变的均匀整数网格中取位置。
fn sample_position(index: usize, count: usize, length: usize) -> usize {
    if count <= 1 {
        return 0;
    }
    let numerator = index * (length - 1);
    (numerator + (count - 1) / 2) / (count - 1)
}

fn clamp_unit(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::imageops;

    fn panorama(width: u32, height: u32, seed: u32) -> RgbaImage {
        let mut bytes = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let mut value = seed
                    ^ x.wrapping_mul(0x9e37_79b9)
                    ^ y.wrapping_mul(0x85eb_ca6b)
                    ^ (x ^ y).wrapping_mul(0xc2b2_ae35);
                value ^= value >> 16;
                value = value.wrapping_mul(0x7feb_352d);
                value ^= value >> 15;
                value = value.wrapping_mul(0x846c_a68b);
                value ^= value >> 16;
                bytes.extend_from_slice(&[
                    value as u8,
                    (value >> 8) as u8,
                    (value >> 16) as u8,
                    (value >> 24) as u8,
                ]);
            }
        }
        RgbaImage::from_raw(width, height, bytes).expect("测试图像尺寸固定")
    }

    fn frame(panorama: &RgbaImage, top: u32, height: u32) -> RgbaImage {
        imageops::crop_imm(panorama, 0, top, panorama.width(), height).to_image()
    }

    fn viewport(panorama: &RgbaImage, left: u32, top: u32, width: u32, height: u32) -> RgbaImage {
        imageops::crop_imm(panorama, left, top, width, height).to_image()
    }

    fn pair(displacement: u32) -> (RgbaImage, RgbaImage, RgbaImage) {
        let panorama = panorama(64, 128, 0x1234_5678);
        (
            frame(&panorama, 0, 72),
            frame(&panorama, displacement, 72),
            panorama,
        )
    }

    #[test]
    fn estimates_exact_translation_and_keeps_original_rows() {
        let (previous, incoming, _) = pair(24);
        let estimate = estimate_vertical_overlap(&previous, &incoming).expect("精确平移应可估计");

        assert_eq!(estimate.displacement_rows, 24);
        assert_eq!(estimate.overlap_rows, 48);
        assert_eq!(estimate.sampled_width, 64);
        assert_eq!(estimate.sampled_rows, 39);
        assert!(estimate.score.abs() <= f32::EPSILON);
        assert!((0.0..=1.0).contains(&estimate.confidence));
    }

    #[test]
    fn estimates_all_four_cardinal_directions() {
        let source = panorama(160, 160, 0x77aa_3311);
        let center = viewport(&source, 40, 40, 64, 72);
        for (incoming, expected_direction, expected_delta) in [
            (
                viewport(&source, 40, 64, 64, 72),
                ScrollDirection::Down,
                (0, 24),
            ),
            (
                viewport(&source, 40, 16, 64, 72),
                ScrollDirection::Up,
                (0, -24),
            ),
            (
                viewport(&source, 64, 40, 64, 72),
                ScrollDirection::Right,
                (24, 0),
            ),
            (
                viewport(&source, 16, 40, 64, 72),
                ScrollDirection::Left,
                (-24, 0),
            ),
        ] {
            let estimate = estimate_translation(&center, &incoming, None)
                .expect("四个正交方向都应得到确定位移");
            assert_eq!(estimate.direction, expected_direction);
            assert_eq!((estimate.delta_x, estimate.delta_y), expected_delta);
            assert_eq!(estimate.displacement_rows, 24);
        }
    }

    #[test]
    fn axis_hysteresis_does_not_override_a_clear_turn() {
        let source = panorama(160, 160, 0x1133_7799);
        let previous = viewport(&source, 40, 40, 64, 72);
        let incoming = viewport(&source, 64, 40, 64, 72);
        let estimate = estimate_translation(&previous, &incoming, Some(ScrollAxis::Vertical))
            .expect("明确的水平滚动应胜过纵向滞回");
        assert_eq!(estimate.direction, ScrollDirection::Right);
        assert_eq!((estimate.delta_x, estimate.delta_y), (24, 0));
    }

    #[test]
    fn ignores_bounded_dynamic_rows_and_small_rgb_noise() {
        let (previous, mut incoming, _) = pair(24);
        for (index, pixel) in incoming.pixels_mut().enumerate() {
            for channel in &mut pixel.0[..3] {
                let delta = if index % 2 == 0 { 2 } else { -2 };
                *channel = channel.saturating_add_signed(delta);
            }
        }
        for y in 0..6 {
            for x in 0..incoming.width() {
                let pixel = incoming.get_pixel_mut(x, y);
                pixel.0[..3].copy_from_slice(&[255, 0, 255]);
            }
        }

        let estimate =
            estimate_vertical_overlap(&previous, &incoming).expect("动态内容应被截尾聚合忽略");
        assert_eq!(estimate.displacement_rows, 24);
    }

    #[test]
    fn rejects_unrelated_identical_and_low_texture_inputs() {
        let previous = frame(&panorama(64, 128, 1), 0, 72);
        let unrelated = frame(&panorama(64, 128, 2), 0, 72);
        assert!(matches!(
            estimate_vertical_overlap(&previous, &unrelated),
            Err(CaptureError::LongshotEstimateLowSimilarity)
        ));
        assert!(matches!(
            estimate_vertical_overlap(&previous, &previous),
            Err(CaptureError::LongshotEstimateNoExtension)
        ));
        let solid = RgbaImage::from_pixel(64, 72, image::Rgba([8, 8, 8, 99]));
        assert!(matches!(
            estimate_vertical_overlap(&solid, &solid),
            Err(CaptureError::LongshotEstimateLowTexture)
        ));
    }

    #[test]
    fn rejects_periodic_pattern_as_ambiguous_before_no_extension() {
        let mut periodic = RgbaImage::new(64, 72);
        for y in 0..72 {
            for x in 0..64 {
                let value = ((y % 4) * 60 + (x % 2) * 20) as u8;
                periodic.put_pixel(x, y, image::Rgba([value, 255 - value, value / 2, 255]));
            }
        }
        assert!(matches!(
            estimate_vertical_overlap(&periodic, &periodic),
            Err(CaptureError::LongshotEstimateAmbiguous)
        ));
    }

    #[test]
    fn accepts_limit_and_classifies_larger_clear_displacement() {
        let (previous, incoming, _) = pair(48);
        assert_eq!(
            estimate_vertical_overlap(&previous, &incoming)
                .expect("位移上限应可接受")
                .displacement_rows,
            48
        );
        let (previous, incoming, _) = pair(49);
        assert!(matches!(
            estimate_vertical_overlap(&previous, &incoming),
            Err(CaptureError::LongshotEstimateDisplacementTooLarge)
        ));
    }

    #[test]
    fn samples_wide_images_horizontally_without_scaling_displacement() {
        let panorama = panorama(1024, 96, 0x9876_5432);
        let previous = frame(&panorama, 0, 72);
        let incoming = frame(&panorama, 24, 72);
        let estimate = estimate_vertical_overlap(&previous, &incoming).expect("宽图应可估计");
        assert_eq!(estimate.sampled_width, 512);
        assert_eq!(estimate.displacement_rows, 24);
    }

    #[test]
    fn validates_inputs_before_scoring_and_keeps_budgets_fixed() {
        let valid = panorama(8, 24, 9);
        assert!(matches!(
            estimate_vertical_overlap(&RgbaImage::new(0, 1), &valid),
            Err(CaptureError::LongshotFrameEmpty)
        ));
        let oversized = RgbaImage::new(super::super::MAX_WIDTH + 1, 1);
        assert!(matches!(
            estimate_vertical_overlap(&oversized, &RgbaImage::new(1, 0)),
            Err(CaptureError::LongshotFrameEmpty)
        ));
        assert!(matches!(
            estimate_vertical_overlap(&valid, &panorama(9, 24, 9)),
            Err(CaptureError::LongshotEstimateSizeMismatch)
        ));
        assert!(matches!(
            estimate_vertical_overlap(&valid, &panorama(8, 25, 9)),
            Err(CaptureError::LongshotEstimateSizeMismatch)
        ));
        assert!(matches!(
            estimate_vertical_overlap(&panorama(7, 24, 9), &panorama(7, 24, 9)),
            Err(CaptureError::LongshotEstimateTooSmall)
        ));
        assert!(matches!(
            estimate_vertical_overlap(&panorama(8, 23, 9), &panorama(8, 23, 9)),
            Err(CaptureError::LongshotEstimateTooSmall)
        ));
        let too_tall = panorama(8, MAX_ESTIMATE_HEIGHT + 1, 9);
        assert!(matches!(
            estimate_vertical_overlap(&too_tall, &too_tall),
            Err(CaptureError::LongshotResourceLimit)
        ));
        const {
            assert!(MAX_ESTIMATE_HEIGHT as usize * COARSE_COLUMNS * COARSE_ROWS <= 4_718_592);
        };
        const {
            assert!(MAX_FINE_CANDIDATES * FINE_COLUMNS * FINE_ROWS <= 294_912);
        };
    }

    #[test]
    fn is_repeatable_and_drives_the_stitcher_without_changing_rgba() {
        let (previous, incoming, panorama) = pair(24);
        let first = estimate_vertical_overlap(&previous, &incoming).expect("应可估计");
        for _ in 0..8 {
            assert_eq!(
                estimate_vertical_overlap(&previous, &incoming).expect("应保持确定性"),
                first
            );
        }
        let mut stitcher = super::super::VerticalStitcher::new();
        stitcher.append(&previous, 0).expect("首帧应可追加");
        stitcher
            .append(&incoming, first.overlap_rows)
            .expect("估计值应可直接拼接");
        assert_eq!(stitcher.width, Some(64));
        assert_eq!(stitcher.height, 96);
        assert_eq!(
            stitcher.rgba,
            imageops::crop_imm(&panorama, 0, 0, 64, 96)
                .to_image()
                .into_raw()
        );
    }
}
