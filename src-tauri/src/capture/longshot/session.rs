//! 单所有者的长截图像素聚合会话。
//!
//! 这里不承担捕获、会话管理或 IPC 生命周期；它只把已交给它的相邻 RGBA 帧按固定
//! 事务顺序估计并拼接。失败时最后一帧和拼接缓冲都保持可重试状态。

use super::overlap::OverlapEstimate;
use super::{checked_pixel_count, overlap, CaptureError, VerticalStitcher, RGBA_BYTES_PER_PIXEL};
use image::RgbaImage;

const MAX_SESSION_FRAME_PIXELS: u64 = 8 * 1024 * 1024;
const MAX_SESSION_FRAME_RAW_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ESTIMATOR_WORK_BYTES: u64 = 48 * 1024 * 1024;
const MAX_SCORING_WORK_BYTES: u64 = 1024 * 1024;
const MAX_APPEND_LOGICAL_WORK_BYTES: u64 = super::MAX_RAW_BYTES
    + MAX_SESSION_FRAME_RAW_BYTES
    + MAX_SESSION_FRAME_RAW_BYTES
    + MAX_ESTIMATOR_WORK_BYTES
    + MAX_SCORING_WORK_BYTES;

/// 当前已提交长截图的只读几何状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::capture) struct LongshotSnapshot {
    pub(in crate::capture) frame_count: usize,
    pub(in crate::capture) width: u32,
    pub(in crate::capture) frame_height: u32,
    pub(in crate::capture) total_height: u32,
}

/// 一次成功追加的估计结果及提交后状态。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::capture) struct LongshotAppendOutcome {
    pub(in crate::capture) snapshot: LongshotSnapshot,
    pub(in crate::capture) estimate: OverlapEstimate,
}

/// 已至少含一帧的、非并发长截图聚合值。
pub(in crate::capture) struct LongshotSession {
    stitcher: VerticalStitcher,
    last_frame: RgbaImage,
}

impl LongshotSession {
    /// 从第一张已裁好的帧创建会话。
    pub(in crate::capture) fn start(first_frame: RgbaImage) -> Result<Self, CaptureError> {
        overlap::validate_single_frame(&first_frame)?;
        validate_session_frame_budget(&first_frame)?;

        let mut stitcher = VerticalStitcher::new();
        stitcher.append(&first_frame, 0)?;
        Ok(Self {
            stitcher,
            last_frame: first_frame,
        })
    }

    /// 原子地估计并提交一张向下滚动后的相邻帧。
    pub(in crate::capture) fn append(
        &mut self,
        incoming: RgbaImage,
    ) -> Result<LongshotAppendOutcome, CaptureError> {
        // 这两步在成对估计之前执行，既避免在会话预算外分配灰度图，也不改变估计器
        // 自身对尺寸不一致/质量门禁的优先级。
        overlap::validate_frame_nonempty_and_global(&incoming)?;
        validate_session_frame_budget(&incoming)?;

        let estimate = overlap::estimate_vertical_overlap(&self.last_frame, &incoming)?;
        self.stitcher.append(&incoming, estimate.overlap_rows)?;
        self.last_frame = incoming;
        Ok(LongshotAppendOutcome {
            snapshot: self.snapshot(),
            estimate,
        })
    }

    /// 返回当前已提交状态，不编码或复制像素。
    pub(in crate::capture) fn snapshot(&self) -> LongshotSnapshot {
        LongshotSnapshot {
            frame_count: self.stitcher.frame_count,
            width: self.last_frame.width(),
            frame_height: self.last_frame.height(),
            total_height: self.stitcher.height,
        }
    }

    /// 以 PNG 物化当前结果；调用不会消费或终结会话。
    pub(in crate::capture) fn finish_png(&self) -> Result<Vec<u8>, CaptureError> {
        self.stitcher.finish_png()
    }

    /// 以固定资源上限物化已提交缓冲的尾部预览，不消费会话。
    pub(in crate::capture) fn preview_tail_png(&self) -> Result<Vec<u8>, CaptureError> {
        self.stitcher.preview_tail_png()
    }

    /// 消费会话并丢弃其内存；上层 manager 负责外部清理与幂等语义。
    pub(in crate::capture) fn cancel(self) {}
}

fn validate_session_frame_budget(image: &RgbaImage) -> Result<(), CaptureError> {
    validate_session_frame_budget_for_dimensions(image.width(), image.height())
}

pub(super) fn validate_session_frame_budget_for_dimensions(
    width: u32,
    height: u32,
) -> Result<(), CaptureError> {
    let pixels = checked_pixel_count(width, height)?;
    let raw_bytes = pixels
        .checked_mul(RGBA_BYTES_PER_PIXEL)
        .ok_or(CaptureError::LongshotResourceLimit)?;
    if pixels > MAX_SESSION_FRAME_PIXELS || raw_bytes > MAX_SESSION_FRAME_RAW_BYTES {
        return Err(CaptureError::LongshotResourceLimit);
    }
    Ok(())
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

    fn pair(displacement: u32) -> (RgbaImage, RgbaImage, RgbaImage) {
        let source = panorama(64, 128, 0x1234_5678);
        (
            frame(&source, 0, 72),
            frame(&source, displacement, 72),
            source,
        )
    }

    fn png_rgba(png: &[u8]) -> Vec<u8> {
        image::load_from_memory(png)
            .expect("PNG 应可解码")
            .into_rgba8()
            .into_raw()
    }

    fn assert_unchanged_after_error(
        session: &mut LongshotSession,
        incoming: RgbaImage,
        expected: fn(&CaptureError) -> bool,
    ) {
        let before_snapshot = session.snapshot();
        let before_png = session.finish_png().expect("失败前应可导出");
        let error = session.append(incoming).expect_err("夹具应被拒绝");
        assert!(expected(&error), "实际错误: {error:?}");
        assert_eq!(session.snapshot(), before_snapshot);
        assert_eq!(session.finish_png().expect("失败后仍应可导出"), before_png);
    }

    #[test]
    fn starts_with_a_valid_frame_and_repeats_exact_png() {
        let source = panorama(64, 72, 7);
        let session = LongshotSession::start(frame(&source, 0, 72)).expect("有效首帧应可创建");

        assert_eq!(
            session.snapshot(),
            LongshotSnapshot {
                frame_count: 1,
                width: 64,
                frame_height: 72,
                total_height: 72,
            }
        );
        let first = session.finish_png().expect("首帧应可导出");
        assert_eq!(session.finish_png().expect("重复导出应成功"), first);
        assert_eq!(png_rgba(&first), source.into_raw());
        session.cancel();
    }

    #[test]
    fn start_rejects_single_frame_contract_and_session_budget() {
        assert!(matches!(
            LongshotSession::start(RgbaImage::new(0, 24)),
            Err(CaptureError::LongshotFrameEmpty)
        ));
        assert!(matches!(
            LongshotSession::start(panorama(7, 24, 1)),
            Err(CaptureError::LongshotEstimateTooSmall)
        ));
        assert!(matches!(
            LongshotSession::start(panorama(8, 23, 1)),
            Err(CaptureError::LongshotEstimateTooSmall)
        ));
        assert!(matches!(
            LongshotSession::start(panorama(8, 16_385, 1)),
            Err(CaptureError::LongshotResourceLimit)
        ));

        assert!(validate_session_frame_budget_for_dimensions(8_192, 1_024).is_ok());
        assert!(matches!(
            validate_session_frame_budget_for_dimensions(8_193, 1_024),
            Err(CaptureError::LongshotResourceLimit)
        ));
        assert_eq!(
            MAX_SESSION_FRAME_PIXELS * RGBA_BYTES_PER_PIXEL,
            MAX_SESSION_FRAME_RAW_BYTES
        );
        assert_eq!(MAX_APPEND_LOGICAL_WORK_BYTES, 369 * 1024 * 1024);
    }

    #[test]
    fn appends_exact_estimate_and_preserves_rgba_alpha() {
        let (previous, incoming, source) = pair(24);
        let mut session = LongshotSession::start(previous).expect("首帧应可创建");
        let outcome = session.append(incoming).expect("精确平移应可追加");

        assert_eq!(outcome.estimate.displacement_rows, 24);
        assert_eq!(outcome.estimate.overlap_rows, 48);
        assert_eq!(
            outcome.snapshot,
            LongshotSnapshot {
                frame_count: 2,
                width: 64,
                frame_height: 72,
                total_height: 96,
            }
        );
        assert_eq!(
            png_rgba(&session.finish_png().expect("结果应可导出")),
            frame(&source, 0, 96).into_raw()
        );
    }

    #[test]
    fn append_failures_are_atomic_and_normal_frames_can_retry() {
        let (previous, valid, _) = pair(24);
        let mut session = LongshotSession::start(previous).expect("首帧应可创建");
        assert_unchanged_after_error(
            &mut session,
            RgbaImage::from_pixel(64, 72, image::Rgba([7, 7, 7, 255])),
            |error| matches!(error, CaptureError::LongshotEstimateLowTexture),
        );
        session.append(valid).expect("低纹理失败后可重试");

        let (previous, valid, _) = pair(24);
        let mut session = LongshotSession::start(previous).expect("首帧应可创建");
        assert_unchanged_after_error(
            &mut session,
            frame(&panorama(64, 128, 99), 0, 72),
            |error| matches!(error, CaptureError::LongshotEstimateLowSimilarity),
        );
        session.append(valid).expect("低相似失败后可重试");

        let (previous, valid, _) = pair(24);
        let mut session = LongshotSession::start(previous).expect("首帧应可创建");
        assert_unchanged_after_error(
            &mut session,
            frame(&panorama(64, 128, 0x1234_5678), 0, 72),
            |error| matches!(error, CaptureError::LongshotEstimateNoExtension),
        );
        session.append(valid).expect("无新增失败后可重试");

        let (previous, valid, _) = pair(24);
        let mut session = LongshotSession::start(previous).expect("首帧应可创建");
        let too_large = frame(&panorama(64, 128, 0x1234_5678), 49, 72);
        assert_unchanged_after_error(&mut session, too_large, |error| {
            matches!(error, CaptureError::LongshotEstimateDisplacementTooLarge)
        });
        session.append(valid).expect("过大位移失败后可重试");

        let (previous, valid, _) = pair(24);
        let mut session = LongshotSession::start(previous).expect("首帧应可创建");
        assert_unchanged_after_error(&mut session, panorama(63, 72, 3), |error| {
            matches!(error, CaptureError::LongshotEstimateSizeMismatch)
        });
        session.append(valid).expect("宽度不一致失败后可重试");

        let (previous, valid, _) = pair(24);
        let mut session = LongshotSession::start(previous).expect("首帧应可创建");
        assert_unchanged_after_error(&mut session, panorama(64, 71, 3), |error| {
            matches!(error, CaptureError::LongshotEstimateSizeMismatch)
        });
        session.append(valid).expect("高度不一致失败后可重试");
    }

    #[test]
    fn periodic_failure_keeps_the_session_exportable() {
        let mut periodic = RgbaImage::new(64, 72);
        for y in 0..72 {
            for x in 0..64 {
                let value = ((y % 4) * 60 + (x % 2) * 20) as u8;
                periodic.put_pixel(x, y, image::Rgba([value, 255 - value, value / 2, 255]));
            }
        }
        let mut session = LongshotSession::start(periodic).expect("周期首帧可被保存");
        let mut incoming = RgbaImage::new(64, 72);
        for y in 0..72 {
            for x in 0..64 {
                let value = ((y % 4) * 60 + (x % 2) * 20) as u8;
                incoming.put_pixel(x, y, image::Rgba([value, 255 - value, value / 2, 255]));
            }
        }
        assert_unchanged_after_error(&mut session, incoming, |error| {
            matches!(error, CaptureError::LongshotEstimateAmbiguous)
        });
        session.cancel();
    }

    #[test]
    fn allows_64_frames_and_rejects_the_65th_without_mutation() {
        let source = panorama(64, 536, 0x4444_5555);
        let mut session = LongshotSession::start(frame(&source, 0, 24)).expect("首帧应可创建");
        for index in 1..64 {
            session
                .append(frame(&source, index * 8, 24))
                .expect("第 64 帧前应可追加");
        }
        assert_eq!(session.snapshot().frame_count, 64);
        assert_eq!(session.snapshot().total_height, 528);
        let before = session.finish_png().expect("第 64 帧结果应可导出");
        assert!(matches!(
            session.append(frame(&source, 512, 24)),
            Err(CaptureError::LongshotFrameLimit)
        ));
        assert_eq!(session.snapshot().frame_count, 64);
        assert_eq!(session.finish_png().expect("失败后仍可导出"), before);
        assert_eq!(png_rgba(&before), frame(&source, 0, 528).into_raw());
    }

    #[test]
    fn identical_sequences_have_identical_outcomes_snapshots_and_png() {
        let source = panorama(64, 128, 0x99aa_bbcc);
        let mut left = LongshotSession::start(frame(&source, 0, 72)).expect("左会话应可创建");
        let mut right = LongshotSession::start(frame(&source, 0, 72)).expect("右会话应可创建");
        for top in [24, 48] {
            let left_outcome = left
                .append(frame(&source, top, 72))
                .expect("左会话应可追加");
            let right_outcome = right
                .append(frame(&source, top, 72))
                .expect("右会话应可追加");
            assert_eq!(left_outcome, right_outcome);
            assert_eq!(left.snapshot(), right.snapshot());
        }
        assert_eq!(
            left.finish_png().expect("左会话应可导出"),
            right.finish_png().expect("右会话应可导出")
        );
    }
}
