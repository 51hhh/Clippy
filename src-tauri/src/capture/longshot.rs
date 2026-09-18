//! 长截图的确定性二维像素核心。
//!
//! `overlap` 子模块从相邻帧估算四向位移，`canvas` 保存不可变帧及其有符号位置，
//! `session` 子模块把两者组合为单所有者事务核心；`window_host` 负责控制窗 IPC、
//! 桌面副作用和输出事务。旧的 [`VerticalStitcher`] 保留为低层纵向回归夹具。

use super::CaptureError;
use crate::pin::PinOrigin;
#[cfg(test)]
use image::{ImageBuffer, Rgba, RgbaImage};

mod canvas;
mod controller;
mod frame_adapter;
mod lifecycle;
mod manager;
mod overlap;
mod recapture;
mod session;
pub(crate) mod window_host;

// lifecycle 与控制窗 registry 通过 AppState 保持唯一实例；重导出固定 capture 领域边界。
pub(super) use frame_adapter::LongshotFrameAdapter;
pub(crate) use lifecycle::LongshotLifecycle;
pub(super) use manager::{LongshotManager, LongshotSessionToken, LongshotStart};
pub(super) use recapture::capture_monitor_frame;
pub(super) use session::{LongshotAppendOutcome, LongshotSession, LongshotSnapshot};
pub(crate) use window_host::{
    handle_controller_destroyed, LongshotActivation, LongshotControllerHandle,
    LongshotControllerLaunch, LongshotControllerRegistry, LongshotIpcError, LongshotOutputAction,
    LongshotOutputResult, LongshotSnapshotDto,
};

/// 长截图完成后的可信像素及其桌面全局逻辑来源矩形。
///
/// 来源矩形只由冻结首帧和实际物理裁剪区反算，不能由控制窗或前端提交。
#[derive(Debug, Clone, PartialEq)]
pub(in crate::capture) struct LongshotArtifact {
    pub(in crate::capture) png: Vec<u8>,
    pub(in crate::capture) origin: PinOrigin,
}

/// 资源边界只在此处定义；后续会话层必须复用而不是另设一组限制。
const MAX_FRAMES: usize = 64;
const MAX_WIDTH: u32 = 16_384;
const MAX_HEIGHT: u32 = 65_536;
const MAX_PIXELS: u64 = 64 * 1024 * 1024;
const MAX_RAW_BYTES: u64 = 256 * 1024 * 1024;
const RGBA_BYTES_PER_PIXEL: u64 = 4;
const PREVIEW_MAX_WIDTH: u32 = 320;
const PREVIEW_MAX_HEIGHT: u32 = 360;
const PREVIEW_MAX_PNG_BYTES: usize = 1024 * 1024;

/// 只追加每帧中不与上一帧重叠的像素行。
///
/// 缓冲区始终为行优先、无 padding 的 RGBA8。字段不向模块外暴露，确保所有状态只能由
/// [`Self::append`] 的预检后写入。
#[derive(Default)]
#[cfg(test)]
pub(super) struct VerticalStitcher {
    width: Option<u32>,
    height: u32,
    frame_count: usize,
    rgba: Vec<u8>,
}

#[cfg(test)]
impl VerticalStitcher {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// 追加一帧，跳过其顶部 `overlap_rows` 行。
    ///
    /// 第一帧没有可重叠的前帧，因此只能传入零。所有会失败的检查和内存预留都在修改
    /// 状态之前完成，失败后拼接器仍可继续使用。
    pub(super) fn append(
        &mut self,
        frame: &RgbaImage,
        overlap_rows: u32,
    ) -> Result<(), CaptureError> {
        let width = frame.width();
        let frame_height = frame.height();
        if width == 0 || frame_height == 0 {
            return Err(CaptureError::LongshotFrameEmpty);
        }
        validate_dimensions(width, frame_height)?;

        let is_first_frame = self.frame_count == 0;
        if is_first_frame {
            if overlap_rows != 0 {
                return Err(CaptureError::LongshotFirstFrameOverlap);
            }
        } else {
            if self.width != Some(width) {
                return Err(CaptureError::LongshotWidthMismatch);
            }
            if overlap_rows >= frame_height {
                return Err(CaptureError::LongshotOverlapInvalid);
            }
        }

        let appended_height = frame_height
            .checked_sub(overlap_rows)
            .ok_or(CaptureError::LongshotOverlapInvalid)?;
        let next_frame_count = self
            .frame_count
            .checked_add(1)
            .ok_or(CaptureError::LongshotResourceLimit)?;
        if next_frame_count > MAX_FRAMES {
            return Err(CaptureError::LongshotFrameLimit);
        }
        let next_height = self
            .height
            .checked_add(appended_height)
            .ok_or(CaptureError::LongshotResourceLimit)?;
        validate_dimensions(width, next_height)?;

        let row_bytes = checked_row_bytes(width)?;
        let source_start = usize::try_from(overlap_rows)
            .ok()
            .and_then(|rows| rows.checked_mul(row_bytes))
            .ok_or(CaptureError::LongshotResourceLimit)?;
        let appended_bytes = usize::try_from(appended_height)
            .ok()
            .and_then(|rows| rows.checked_mul(row_bytes))
            .ok_or(CaptureError::LongshotResourceLimit)?;
        let expected_current_bytes = checked_raw_bytes(width, self.height)?;
        let expected_next_bytes = checked_raw_bytes(width, next_height)?;
        if self.rgba.len() != expected_current_bytes
            || expected_next_bytes
                .checked_sub(expected_current_bytes)
                .filter(|bytes| *bytes == appended_bytes)
                .is_none()
        {
            return Err(CaptureError::LongshotResourceLimit);
        }
        let source = frame
            .as_raw()
            .get(source_start..)
            .filter(|bytes| bytes.len() == appended_bytes)
            .ok_or(CaptureError::LongshotResourceLimit)?;

        self.rgba
            .try_reserve_exact(appended_bytes)
            .map_err(|_| CaptureError::LongshotAllocationFailed)?;
        self.rgba.extend_from_slice(source);
        self.width = Some(width);
        self.height = next_height;
        self.frame_count = next_frame_count;
        Ok(())
    }

    /// 把已验证的原始 RGBA 缓冲编码为项目统一格式的 PNG。
    pub(super) fn finish_png(&self) -> Result<Vec<u8>, CaptureError> {
        let width = self.width.ok_or(CaptureError::LongshotEmpty)?;
        if self.frame_count == 0 {
            return Err(CaptureError::LongshotEmpty);
        }
        validate_dimensions(width, self.height)?;
        if self.rgba.len() != checked_raw_bytes(width, self.height)? {
            return Err(CaptureError::LongshotResourceLimit);
        }
        crate::screenshot::encode_png(&self.rgba, width, self.height).map_err(CaptureError::codec)
    }

    /// 从已提交 RGBA 缓冲的借用视图生成固定上限的尾部预览。
    ///
    /// 这里只分配缩放后的目标图与其 PNG；不会复制源尾部，也不会编码完整全景。
    pub(super) fn preview_tail_png(&self) -> Result<Vec<u8>, CaptureError> {
        let width = self.width.ok_or(CaptureError::LongshotEmpty)?;
        if self.frame_count == 0 {
            return Err(CaptureError::LongshotEmpty);
        }
        validate_dimensions(width, self.height)?;
        if self.rgba.len() != checked_raw_bytes(width, self.height)? {
            return Err(CaptureError::LongshotResourceLimit);
        }

        let source =
            ImageBuffer::<Rgba<u8>, &[u8]>::from_raw(width, self.height, self.rgba.as_slice())
                .ok_or(CaptureError::LongshotResourceLimit)?;
        let target_width = width.min(PREVIEW_MAX_WIDTH);
        let full_target_height = scaled_height(self.height, target_width, width)?;
        let (source_top, source_height, target_height) = if full_target_height <= PREVIEW_MAX_HEIGHT
        {
            (0, self.height, full_target_height)
        } else {
            let source_height = div_ceil_u64(
                u64::from(PREVIEW_MAX_HEIGHT) * u64::from(width),
                u64::from(target_width),
            )?;
            let source_height = u32::try_from(source_height)
                .map_err(|_| CaptureError::LongshotResourceLimit)?
                .min(self.height);
            (
                self.height - source_height,
                source_height,
                scaled_height(source_height, target_width, width)?.min(PREVIEW_MAX_HEIGHT),
            )
        };
        let mut preview = RgbaImage::new(target_width, target_height);
        for target_y in 0..target_height {
            let source_y =
                source_top + nearest_source_coordinate(target_y, target_height, source_height)?;
            for target_x in 0..target_width {
                let source_x = nearest_source_coordinate(target_x, target_width, width)?;
                preview.put_pixel(target_x, target_y, *source.get_pixel(source_x, source_y));
            }
        }
        let png =
            crate::screenshot::encode_png(preview.as_raw(), preview.width(), preview.height())
                .map_err(CaptureError::codec)?;
        validate_preview_png_bytes(png)
    }
}

fn validate_preview_png_bytes(png: Vec<u8>) -> Result<Vec<u8>, CaptureError> {
    if png.is_empty() {
        return Err(CaptureError::Codec("长截图预览 PNG 为空".to_string()));
    }
    if png.len() > PREVIEW_MAX_PNG_BYTES {
        return Err(CaptureError::LongshotResourceLimit);
    }
    Ok(png)
}

/// 以像素中心为基准的整数最近邻映射，结果恒落在 `[0, source_extent)`。
#[cfg(test)]
fn nearest_source_coordinate(
    target: u32,
    target_extent: u32,
    source_extent: u32,
) -> Result<u32, CaptureError> {
    let doubled_target = u64::from(target)
        .checked_mul(2)
        .and_then(|value| value.checked_add(1))
        .ok_or(CaptureError::LongshotResourceLimit)?;
    let numerator = doubled_target
        .checked_mul(u64::from(source_extent))
        .ok_or(CaptureError::LongshotResourceLimit)?;
    let denominator = u64::from(target_extent)
        .checked_mul(2)
        .ok_or(CaptureError::LongshotResourceLimit)?;
    let coordinate = (numerator / denominator).min(u64::from(source_extent - 1));
    u32::try_from(coordinate).map_err(|_| CaptureError::LongshotResourceLimit)
}

fn scaled_height(height: u32, numerator: u32, denominator: u32) -> Result<u32, CaptureError> {
    let scaled = u64::from(height)
        .checked_mul(u64::from(numerator))
        .ok_or(CaptureError::LongshotResourceLimit)?
        / u64::from(denominator);
    u32::try_from(scaled.max(1)).map_err(|_| CaptureError::LongshotResourceLimit)
}

#[cfg(test)]
fn div_ceil_u64(numerator: u64, denominator: u64) -> Result<u64, CaptureError> {
    numerator
        .checked_add(denominator - 1)
        .ok_or(CaptureError::LongshotResourceLimit)
        .map(|value| value / denominator)
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), CaptureError> {
    if width > MAX_WIDTH || height > MAX_HEIGHT {
        return Err(CaptureError::LongshotResourceLimit);
    }
    let pixels = checked_pixel_count(width, height)?;
    if pixels > MAX_PIXELS {
        return Err(CaptureError::LongshotResourceLimit);
    }
    let bytes = pixels
        .checked_mul(RGBA_BYTES_PER_PIXEL)
        .ok_or(CaptureError::LongshotResourceLimit)?;
    if bytes > MAX_RAW_BYTES || usize::try_from(bytes).is_err() {
        return Err(CaptureError::LongshotResourceLimit);
    }
    Ok(())
}

fn checked_pixel_count(width: u32, height: u32) -> Result<u64, CaptureError> {
    u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(CaptureError::LongshotResourceLimit)
}

#[cfg(test)]
fn checked_row_bytes(width: u32) -> Result<usize, CaptureError> {
    let bytes = u64::from(width)
        .checked_mul(RGBA_BYTES_PER_PIXEL)
        .ok_or(CaptureError::LongshotResourceLimit)?;
    usize::try_from(bytes).map_err(|_| CaptureError::LongshotResourceLimit)
}

#[cfg(test)]
fn checked_raw_bytes(width: u32, height: u32) -> Result<usize, CaptureError> {
    let bytes = checked_pixel_count(width, height)?
        .checked_mul(RGBA_BYTES_PER_PIXEL)
        .ok_or(CaptureError::LongshotResourceLimit)?;
    usize::try_from(bytes).map_err(|_| CaptureError::LongshotResourceLimit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32, pixels: &[[u8; 4]]) -> RgbaImage {
        let rgba = pixels
            .iter()
            .flat_map(|pixel| pixel.iter().copied())
            .collect::<Vec<_>>();
        RgbaImage::from_raw(width, height, rgba).expect("测试像素必须匹配图片尺寸")
    }

    fn solid(width: u32, height: u32, pixel: [u8; 4]) -> RgbaImage {
        image(width, height, &vec![pixel; (width * height) as usize])
    }

    fn decode(png: &[u8]) -> RgbaImage {
        image::load_from_memory(png)
            .expect("预览 PNG 应可解码")
            .into_rgba8()
    }

    fn coordinate_image(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            Rgba([(x & 0xff) as u8, (y & 0xff) as u8, (y >> 8) as u8, 173])
        })
    }

    #[test]
    fn stitches_explicit_overlap_without_changing_rgba_rows() {
        let first = image(
            2,
            2,
            &[
                [10, 11, 12, 13],
                [20, 21, 22, 23],
                [30, 31, 32, 33],
                [40, 41, 42, 43],
            ],
        );
        let second = image(
            2,
            3,
            &[
                [30, 31, 32, 33],
                [40, 41, 42, 43],
                [50, 51, 52, 53],
                [60, 61, 62, 63],
                [70, 71, 72, 73],
                [80, 81, 82, 83],
            ],
        );
        let mut stitcher = VerticalStitcher::new();

        stitcher.append(&first, 0).expect("首帧应可追加");
        stitcher.append(&second, 1).expect("第二帧应可追加");

        assert_eq!(stitcher.width, Some(2));
        assert_eq!(stitcher.height, 4);
        assert_eq!(
            stitcher.rgba,
            vec![
                10, 11, 12, 13, 20, 21, 22, 23, 30, 31, 32, 33, 40, 41, 42, 43, 50, 51, 52, 53, 60,
                61, 62, 63, 70, 71, 72, 73, 80, 81, 82, 83,
            ]
        );

        let png = stitcher.finish_png().expect("非空拼接应编码 PNG");
        assert_eq!(
            crate::screenshot::validate_png(&png).expect("PNG 应可验证"),
            (2, 4)
        );
        assert_eq!(
            image::load_from_memory(&png)
                .expect("PNG 应可解码")
                .into_rgba8()
                .into_raw(),
            stitcher.rgba
        );
    }

    #[test]
    fn rejects_each_invalid_frame_contract_with_a_structured_error() {
        let mut stitcher = VerticalStitcher::new();
        assert!(matches!(
            stitcher.append(&solid(1, 1, [1, 2, 3, 4]), 1),
            Err(CaptureError::LongshotFirstFrameOverlap)
        ));
        assert!(matches!(
            stitcher.append(&RgbaImage::new(0, 1), 0),
            Err(CaptureError::LongshotFrameEmpty)
        ));

        stitcher
            .append(&solid(2, 2, [1, 2, 3, 4]), 0)
            .expect("有效首帧应可追加");
        assert!(matches!(
            stitcher.append(&solid(2, 2, [1, 2, 3, 4]), 2),
            Err(CaptureError::LongshotOverlapInvalid)
        ));
        assert!(matches!(
            stitcher.append(&solid(2, 2, [1, 2, 3, 4]), 3),
            Err(CaptureError::LongshotOverlapInvalid)
        ));
        assert!(matches!(
            stitcher.append(&solid(3, 1, [1, 2, 3, 4]), 0),
            Err(CaptureError::LongshotWidthMismatch)
        ));
    }

    #[test]
    fn rejects_frame_and_resource_limits_before_copying() {
        let mut stitcher = VerticalStitcher::new();
        for _ in 0..MAX_FRAMES {
            stitcher
                .append(&solid(1, 1, [1, 2, 3, 4]), 0)
                .expect("第 64 帧应可追加");
        }
        let bytes_before_limit = stitcher.rgba.clone();
        assert!(matches!(
            stitcher.append(&solid(1, 1, [9, 9, 9, 9]), 0),
            Err(CaptureError::LongshotFrameLimit)
        ));
        assert_eq!(stitcher.rgba, bytes_before_limit);

        for (width, height) in [
            (MAX_WIDTH + 1, 1),
            (1, MAX_HEIGHT + 1),
            (MAX_WIDTH, MAX_PIXELS as u32 / MAX_WIDTH + 1),
        ] {
            assert!(matches!(
                validate_dimensions(width, height),
                Err(CaptureError::LongshotResourceLimit)
            ));
        }
        assert!(matches!(
            checked_pixel_count(u32::MAX, u32::MAX),
            Ok(pixel_count) if pixel_count > MAX_PIXELS
        ));
    }

    #[test]
    fn cannot_finish_an_empty_stitcher() {
        assert!(matches!(
            VerticalStitcher::new().finish_png(),
            Err(CaptureError::LongshotEmpty)
        ));
    }

    #[test]
    fn preview_keeps_full_small_image_and_transparent_pixels() {
        let source = image(
            2,
            2,
            &[
                [1, 2, 3, 0],
                [4, 5, 6, 64],
                [7, 8, 9, 128],
                [10, 11, 12, 255],
            ],
        );
        let mut stitcher = VerticalStitcher::new();
        stitcher.append(&source, 0).expect("小图应可追加");

        let preview = decode(&stitcher.preview_tail_png().expect("小图预览应成功"));
        assert_eq!(preview.dimensions(), (2, 2));
        assert_eq!(preview.into_raw(), source.into_raw());
    }

    #[test]
    fn preview_anchors_tall_and_extremely_narrow_images_to_exact_tail() {
        let source = coordinate_image(1, 400);
        let mut stitcher = VerticalStitcher::new();
        stitcher.append(&source, 0).expect("极窄高图应可追加");

        let preview = decode(&stitcher.preview_tail_png().expect("极窄高图预览应成功"));
        assert_eq!(preview.dimensions(), (1, PREVIEW_MAX_HEIGHT));
        assert_eq!(preview.get_pixel(0, 0), source.get_pixel(0, 40));
        assert_eq!(preview.get_pixel(0, 359), source.get_pixel(0, 399));
    }

    #[test]
    fn preview_scales_superwide_image_with_fixed_center_sampling() {
        let source = coordinate_image(640, 200);
        let mut stitcher = VerticalStitcher::new();
        stitcher.append(&source, 0).expect("超宽图应可追加");

        let preview = decode(&stitcher.preview_tail_png().expect("超宽图预览应成功"));
        assert_eq!(preview.dimensions(), (320, 100));
        assert_eq!(preview.get_pixel(0, 0), source.get_pixel(1, 1));
        assert_eq!(preview.get_pixel(319, 99), source.get_pixel(639, 199));
    }

    #[test]
    fn preview_tall_scaled_crop_uses_integer_ceil_and_tail_coordinates() {
        let source = coordinate_image(640, 1_000);
        let mut stitcher = VerticalStitcher::new();
        stitcher.append(&source, 0).expect("缩放高图应可追加");

        let preview = decode(&stitcher.preview_tail_png().expect("缩放高图预览应成功"));
        assert_eq!(preview.dimensions(), (320, 360));
        // 源尾区为 [280, 1000)，中心最近邻首末采样分别落在 281 与 999。
        assert_eq!(preview.get_pixel(0, 0), source.get_pixel(1, 281));
        assert_eq!(preview.get_pixel(319, 359), source.get_pixel(639, 999));
    }

    #[test]
    fn preview_boundary_never_upscales_or_changes_pixels() {
        let source = coordinate_image(PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT);
        let mut stitcher = VerticalStitcher::new();
        stitcher.append(&source, 0).expect("边界图应可追加");

        let png = stitcher.preview_tail_png().expect("边界图预览应成功");
        assert!(!png.is_empty() && png.len() <= PREVIEW_MAX_PNG_BYTES);
        let preview = decode(&png);
        assert_eq!(preview.dimensions(), (320, 360));
        assert_eq!(preview.into_raw(), source.into_raw());
    }

    #[test]
    fn preview_png_budget_rejects_empty_and_oversized_payloads() {
        assert!(matches!(
            validate_preview_png_bytes(Vec::new()),
            Err(CaptureError::Codec(message)) if message == "长截图预览 PNG 为空"
        ));
        assert!(matches!(
            validate_preview_png_bytes(vec![0; PREVIEW_MAX_PNG_BYTES + 1]),
            Err(CaptureError::LongshotResourceLimit)
        ));
        assert_eq!(
            validate_preview_png_bytes(vec![1; PREVIEW_MAX_PNG_BYTES])
                .expect("精确上限应被接受")
                .len(),
            PREVIEW_MAX_PNG_BYTES
        );
    }

    #[test]
    fn noisy_maximum_preview_stays_inside_binary_ipc_budget() {
        let source = RgbaImage::from_fn(PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT, |x, y| {
            let mut value = x.wrapping_mul(0x9e37_79b9) ^ y.wrapping_mul(0x85eb_ca6b);
            value ^= value >> 16;
            value = value.wrapping_mul(0x7feb_352d);
            value ^= value >> 15;
            Rgba([
                value as u8,
                (value >> 8) as u8,
                (value >> 16) as u8,
                (value >> 24) as u8,
            ])
        });
        let mut stitcher = VerticalStitcher::new();
        stitcher.append(&source, 0).expect("噪声边界图应可追加");
        let png = stitcher.preview_tail_png().expect("噪声边界预览应成功");
        assert!(!png.is_empty() && png.len() <= PREVIEW_MAX_PNG_BYTES);
        assert_eq!(crate::screenshot::validate_png(&png).unwrap(), (320, 360));
    }
}
