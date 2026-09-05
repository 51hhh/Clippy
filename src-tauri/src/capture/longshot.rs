//! 长截图的确定性垂直像素核心。
//!
//! [`VerticalStitcher`] 消费显式重叠行，`overlap` 子模块负责从相邻帧估算该值，
//! `session` 子模块把两者组合为单所有者事务核心；IPC 和 UI 仍由后续层负责。

use super::CaptureError;
use image::RgbaImage;

mod controller;
mod frame_adapter;
mod lifecycle;
mod manager;
mod overlap;
mod recapture;
mod session;

// lifecycle 已注入 AppState；IPC 尚未接线。保留 crate 内重导出以固定 capture 领域边界，
// 而不是为了消除 dead-code 伪造调用。
#[allow(unused_imports)]
pub(super) use frame_adapter::LongshotFrameAdapter;
pub(crate) use lifecycle::LongshotLifecycle;
#[allow(unused_imports)]
pub(super) use manager::{LongshotManager, LongshotSessionToken, LongshotStart};
#[allow(unused_imports)]
pub(super) use recapture::capture_monitor_frame;
#[allow(unused_imports)]
pub(super) use session::{LongshotAppendOutcome, LongshotSession, LongshotSnapshot};

/// 资源边界只在此处定义；后续会话层必须复用而不是另设一组限制。
const MAX_FRAMES: usize = 64;
const MAX_WIDTH: u32 = 16_384;
const MAX_HEIGHT: u32 = 65_536;
const MAX_PIXELS: u64 = 64 * 1024 * 1024;
const MAX_RAW_BYTES: u64 = 256 * 1024 * 1024;
const RGBA_BYTES_PER_PIXEL: u64 = 4;

/// 只追加每帧中不与上一帧重叠的像素行。
///
/// 缓冲区始终为行优先、无 padding 的 RGBA8。字段不向模块外暴露，确保所有状态只能由
/// [`Self::append`] 的预检后写入。
#[derive(Default)]
pub(super) struct VerticalStitcher {
    width: Option<u32>,
    height: u32,
    frame_count: usize,
    rgba: Vec<u8>,
}

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

fn checked_row_bytes(width: u32) -> Result<usize, CaptureError> {
    let bytes = u64::from(width)
        .checked_mul(RGBA_BYTES_PER_PIXEL)
        .ok_or(CaptureError::LongshotResourceLimit)?;
    usize::try_from(bytes).map_err(|_| CaptureError::LongshotResourceLimit)
}

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
}
