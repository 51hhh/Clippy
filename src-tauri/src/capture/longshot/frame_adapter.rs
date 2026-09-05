//! 连续重捕获帧的固定选区适配器。
//!
//! 截图后端已经归一化图像方向；这里仅验证同一显示器的几何签名并裁出冻结矩形，
//! 不执行旋转、缩放或 PNG 编码。

use super::session::validate_session_frame_budget_for_dimensions;
use super::CaptureError;
use crate::capture::frame_crop::{selection_pixel_rect, PixelRect};
use crate::capture::CaptureSelection;
use crate::screenshot::CapturedMonitorFrame;
use image::RgbaImage;

const SCALE_TOLERANCE: f32 = 1e-4;

/// 首帧成功后冻结的连续帧几何事实。
#[derive(Debug, Clone, Copy)]
struct FrameSignature {
    monitor_id: u32,
    x: i32,
    y: i32,
    logical_width: u32,
    logical_height: u32,
    pixel_width: u32,
    pixel_height: u32,
    scale_x: f32,
    scale_y: f32,
}

impl FrameSignature {
    fn from_frame(frame: &CapturedMonitorFrame) -> Self {
        Self {
            monitor_id: frame.monitor_id,
            x: frame.x,
            y: frame.y,
            logical_width: frame.logical_width,
            logical_height: frame.logical_height,
            pixel_width: frame.pixel_width,
            pixel_height: frame.pixel_height,
            scale_x: frame.scale_x,
            scale_y: frame.scale_y,
        }
    }

    fn matches(self, frame: &CapturedMonitorFrame) -> bool {
        self.monitor_id == frame.monitor_id
            && self.x == frame.x
            && self.y == frame.y
            && self.logical_width == frame.logical_width
            && self.logical_height == frame.logical_height
            && self.pixel_width == frame.pixel_width
            && self.pixel_height == frame.pixel_height
            && scales_close(self.scale_x, frame.scale_x)
            && scales_close(self.scale_y, frame.scale_y)
    }
}

/// 在同一显示器连续帧里复用的、不可重新换算的物理选区。
#[derive(Debug, Clone, Copy)]
pub(in crate::capture) struct LongshotFrameAdapter {
    signature: FrameSignature,
    crop: PixelRect,
}

impl LongshotFrameAdapter {
    /// 校验首帧并冻结其物理裁剪矩形。
    pub(in crate::capture) fn from_first(
        frame: &CapturedMonitorFrame,
        selection: &CaptureSelection,
    ) -> Result<(Self, RgbaImage), CaptureError> {
        if selection.monitor_id != frame.monitor_id {
            return Err(CaptureError::SelectionMonitorMismatch);
        }
        validate_frame_metadata(frame)?;
        validate_exact_rgba(frame)?;
        let crop = selection_pixel_rect(frame, selection)?;
        let image = crop_frame(frame, crop)?;
        Ok((
            Self {
                signature: FrameSignature::from_frame(frame),
                crop,
            },
            image,
        ))
    }

    /// 只接受和首帧签名相同的帧，并复用冻结的物理矩形。
    pub(in crate::capture) fn crop_next(
        &self,
        frame: &CapturedMonitorFrame,
    ) -> Result<RgbaImage, CaptureError> {
        if frame.monitor_id != self.signature.monitor_id {
            return Err(CaptureError::SelectionMonitorMismatch);
        }
        validate_frame_metadata(frame)?;
        if !self.signature.matches(frame) {
            return Err(CaptureError::LongshotFrameGeometryChanged);
        }
        validate_exact_rgba(frame)?;
        crop_frame(frame, self.crop)
    }
}

fn scales_close(left: f32, right: f32) -> bool {
    left.is_finite() && right.is_finite() && (left - right).abs() <= SCALE_TOLERANCE
}

fn validate_frame_metadata(frame: &CapturedMonitorFrame) -> Result<(), CaptureError> {
    if frame.logical_width == 0
        || frame.logical_height == 0
        || frame.pixel_width == 0
        || frame.pixel_height == 0
        || !frame.scale_x.is_finite()
        || !frame.scale_y.is_finite()
        || frame.scale_x <= 0.0
        || frame.scale_y <= 0.0
    {
        return Err(CaptureError::LongshotFrameInvalid);
    }
    Ok(())
}

fn validate_exact_rgba(frame: &CapturedMonitorFrame) -> Result<(), CaptureError> {
    let expected = checked_source_len(frame.pixel_width, frame.pixel_height)?;
    if frame.rgba.len() != expected {
        return Err(CaptureError::LongshotFrameInvalid);
    }
    Ok(())
}

fn checked_source_len(width: u32, height: u32) -> Result<usize, CaptureError> {
    let bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(CaptureError::LongshotFrameInvalid)?;
    usize::try_from(bytes).map_err(|_| CaptureError::LongshotFrameInvalid)
}

fn checked_crop_usize(value: u64) -> Result<usize, CaptureError> {
    usize::try_from(value).map_err(|_| CaptureError::CropOutOfBounds)
}

fn crop_frame(frame: &CapturedMonitorFrame, crop: PixelRect) -> Result<RgbaImage, CaptureError> {
    if crop.right > frame.pixel_width
        || crop.bottom > frame.pixel_height
        || crop.right <= crop.left
        || crop.bottom <= crop.top
    {
        return Err(CaptureError::CropOutOfBounds);
    }

    let width = crop
        .right
        .checked_sub(crop.left)
        .ok_or(CaptureError::CropOutOfBounds)?;
    let height = crop
        .bottom
        .checked_sub(crop.top)
        .ok_or(CaptureError::CropOutOfBounds)?;
    validate_session_frame_budget_for_dimensions(width, height)?;
    let source_len = checked_source_len(frame.pixel_width, frame.pixel_height)
        .map_err(|_| CaptureError::CropOutOfBounds)?;
    if frame.rgba.len() != source_len {
        return Err(CaptureError::CropOutOfBounds);
    }

    let source_row_bytes = checked_crop_usize(
        u64::from(frame.pixel_width)
            .checked_mul(4)
            .ok_or(CaptureError::CropOutOfBounds)?,
    )?;
    let row_bytes = checked_crop_usize(
        u64::from(width)
            .checked_mul(4)
            .ok_or(CaptureError::CropOutOfBounds)?,
    )?;
    let output_len = checked_crop_usize(
        u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(CaptureError::CropOutOfBounds)?,
    )?;

    let mut rgba = Vec::new();
    rgba.try_reserve_exact(output_len)
        .map_err(|_| CaptureError::LongshotAllocationFailed)?;
    for row in crop.top..crop.bottom {
        let row_start = u64::from(row)
            .checked_mul(
                u64::try_from(source_row_bytes).map_err(|_| CaptureError::CropOutOfBounds)?,
            )
            .and_then(|offset| {
                u64::from(crop.left)
                    .checked_mul(4)
                    .and_then(|left| offset.checked_add(left))
            })
            .ok_or(CaptureError::CropOutOfBounds)?;
        let start = checked_crop_usize(row_start)?;
        let end = start
            .checked_add(row_bytes)
            .ok_or(CaptureError::CropOutOfBounds)?;
        let source = frame
            .rgba
            .get(start..end)
            .ok_or(CaptureError::CropOutOfBounds)?;
        rgba.extend_from_slice(source);
    }

    RgbaImage::from_raw(width, height, rgba).ok_or(CaptureError::LongshotFrameInvalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::longshot::LongshotManager;
    use image::imageops;
    use std::sync::Arc;

    fn selection(x: f64, y: f64, width: f64, height: f64) -> CaptureSelection {
        CaptureSelection {
            session_id: "adapter-test".to_string(),
            monitor_id: 7,
            x,
            y,
            width,
            height,
        }
    }

    #[allow(clippy::too_many_arguments)] // 测试需逐项表达帧几何、缩放与种子，避免为此新增生产抽象。
    fn encoded_frame(
        x: i32,
        y: i32,
        logical_width: u32,
        logical_height: u32,
        pixel_width: u32,
        pixel_height: u32,
        scale_x: f32,
        scale_y: f32,
        seed: u32,
    ) -> CapturedMonitorFrame {
        let mut rgba = Vec::with_capacity((pixel_width * pixel_height * 4) as usize);
        for row in 0..pixel_height {
            for column in 0..pixel_width {
                let value = seed ^ column.wrapping_mul(0x9e37_79b9) ^ row.wrapping_mul(0x85eb_ca6b);
                rgba.extend_from_slice(&[
                    column as u8,
                    row as u8,
                    value as u8,
                    (value >> 24) as u8,
                ]);
            }
        }
        CapturedMonitorFrame {
            monitor_id: 7,
            x,
            y,
            logical_width,
            logical_height,
            pixel_width,
            pixel_height,
            scale_x,
            scale_y,
            rgba: Arc::from(rgba),
        }
    }

    fn assert_pixel(
        image: &RgbaImage,
        x: u32,
        y: u32,
        frame: &CapturedMonitorFrame,
        source_x: u32,
        source_y: u32,
    ) {
        let start = ((source_y * frame.pixel_width + source_x) * 4) as usize;
        assert_eq!(image.get_pixel(x, y).0, frame.rgba[start..start + 4]);
    }

    #[test]
    fn first_frame_uses_shared_floor_ceil_rules_at_multiple_scales() {
        let cases = [
            (
                encoded_frame(0, 0, 10, 10, 10, 10, 1.0, 1.0, 1),
                selection(1.2, 2.2, 3.1, 2.1),
                (1, 2, 4, 3),
            ),
            (
                encoded_frame(0, 0, 10, 10, 20, 20, 2.0, 2.0, 2),
                selection(1.25, 1.25, 2.5, 2.5),
                (2, 2, 6, 6),
            ),
            (
                encoded_frame(0, 0, 8, 8, 10, 12, 1.25, 1.5, 3),
                selection(1.1, 1.1, 2.0, 2.0),
                (1, 1, 3, 4),
            ),
        ];
        for (frame, selection, (left, top, width, height)) in cases {
            let (_, image) =
                LongshotFrameAdapter::from_first(&frame, &selection).expect("有效首帧应可裁剪");
            assert_eq!(image.dimensions(), (width, height));
            assert_pixel(&image, 0, 0, &frame, left, top);
            assert_pixel(
                &image,
                width - 1,
                height - 1,
                &frame,
                left + width - 1,
                top + height - 1,
            );
        }
    }

    #[test]
    fn first_frame_clamps_monitor_local_selection_without_desktop_origin() {
        let negative_origin = encoded_frame(-1920, -120, 10, 10, 10, 10, 1.0, 1.0, 4);
        let zero_origin = encoded_frame(0, 0, 10, 10, 10, 10, 1.0, 1.0, 4);
        let negative_selection = selection(-2.0, -1.0, 5.0, 5.0);
        let (_, from_negative) =
            LongshotFrameAdapter::from_first(&negative_origin, &negative_selection)
                .expect("负选区应被 clamp");
        let (_, from_zero) = LongshotFrameAdapter::from_first(&zero_origin, &negative_selection)
            .expect("桌面原点不应影响局部选区");
        assert_eq!(from_negative.dimensions(), (3, 4));
        assert_eq!(from_negative, from_zero);

        let (_, edge) =
            LongshotFrameAdapter::from_first(&negative_origin, &selection(8.0, 9.0, 5.0, 4.0))
                .expect("右下越界应被 clamp");
        assert_eq!(edge.dimensions(), (2, 1));
        assert_pixel(&edge, 1, 0, &negative_origin, 9, 9);
    }

    #[test]
    fn next_frame_reuses_frozen_rect_and_preserves_source() {
        let first = encoded_frame(0, 0, 10, 10, 10, 10, 1.0, 1.0, 5);
        let source_before = first.rgba.clone();
        let (adapter, cropped_first) =
            LongshotFrameAdapter::from_first(&first, &selection(2.0, 3.0, 4.0, 4.0))
                .expect("首帧应成功");
        let next = encoded_frame(0, 0, 10, 10, 10, 10, 1.0, 1.0, 6);
        let cropped_next = adapter.crop_next(&next).expect("同签名帧应成功");
        assert_eq!(cropped_first.dimensions(), (4, 4));
        assert_eq!(cropped_next.dimensions(), (4, 4));
        assert_ne!(cropped_first.into_raw(), cropped_next.clone().into_raw());
        assert_pixel(&cropped_next, 0, 0, &next, 2, 3);
        assert_eq!(first.rgba, source_before);
        assert_eq!(
            adapter.crop_next(&next).expect("重复调用应确定"),
            cropped_next
        );
    }

    #[test]
    fn rejects_monitor_and_geometry_drift_without_poisoning_adapter() {
        let first = encoded_frame(-10, 4, 10, 10, 10, 10, 1.0, 1.0, 7);
        let (adapter, _) = LongshotFrameAdapter::from_first(&first, &selection(2.0, 2.0, 4.0, 4.0))
            .expect("首帧应成功");
        let mut wrong_monitor = first.clone();
        wrong_monitor.monitor_id = 8;
        assert_eq!(
            adapter.crop_next(&wrong_monitor).unwrap_err().code(),
            "selection_monitor_mismatch"
        );

        for changed in [
            {
                let mut value = first.clone();
                value.x += 1;
                value
            },
            {
                let mut value = first.clone();
                value.y += 1;
                value
            },
            {
                let mut value = first.clone();
                value.logical_width += 1;
                value
            },
            {
                let mut value = first.clone();
                value.logical_height += 1;
                value
            },
            {
                let mut value = first.clone();
                value.pixel_width += 1;
                value
            },
            {
                let mut value = first.clone();
                value.pixel_height += 1;
                value
            },
            {
                let mut value = first.clone();
                value.scale_x += 0.01;
                value
            },
            {
                let mut value = first.clone();
                value.scale_y += 0.01;
                value
            },
        ] {
            assert_eq!(
                adapter.crop_next(&changed).unwrap_err().code(),
                "longshot_frame_geometry_changed"
            );
        }
        let mut tolerated = first.clone();
        tolerated.scale_x += SCALE_TOLERANCE / 2.0;
        assert!(adapter.crop_next(&tolerated).is_ok());
        assert!(adapter.crop_next(&first).is_ok());
    }

    #[test]
    fn rejects_invalid_metadata_buffers_and_selection_contracts() {
        let valid = encoded_frame(0, 0, 10, 10, 10, 10, 1.0, 1.0, 8);
        for invalid in [
            {
                let mut value = valid.clone();
                value.logical_width = 0;
                value
            },
            {
                let mut value = valid.clone();
                value.pixel_height = 0;
                value
            },
            {
                let mut value = valid.clone();
                value.scale_x = f32::NAN;
                value
            },
            {
                let mut value = valid.clone();
                value.scale_y = f32::INFINITY;
                value
            },
            {
                let mut value = valid.clone();
                value.scale_x = 0.0;
                value
            },
            {
                let mut value = valid.clone();
                value.scale_y = -1.0;
                value
            },
            {
                let mut value = valid.clone();
                value.rgba = Arc::from(vec![0; 399]);
                value
            },
            {
                let mut value = valid.clone();
                value.rgba = Arc::from(vec![0; 401]);
                value
            },
            CapturedMonitorFrame {
                monitor_id: 7,
                x: 0,
                y: 0,
                logical_width: 1,
                logical_height: 1,
                pixel_width: u32::MAX,
                pixel_height: u32::MAX,
                scale_x: 1.0,
                scale_y: 1.0,
                rgba: Arc::from(Vec::<u8>::new()),
            },
        ] {
            assert_eq!(
                LongshotFrameAdapter::from_first(&invalid, &selection(1.0, 1.0, 4.0, 4.0))
                    .unwrap_err()
                    .code(),
                "longshot_frame_invalid"
            );
        }
        assert_eq!(
            LongshotFrameAdapter::from_first(&valid, &selection(f64::NAN, 0.0, 4.0, 4.0))
                .unwrap_err()
                .code(),
            "selection_not_finite"
        );
        assert_eq!(
            LongshotFrameAdapter::from_first(&valid, &selection(0.0, 0.0, 1.0, 4.0))
                .unwrap_err()
                .code(),
            "selection_too_small"
        );
        assert_eq!(
            LongshotFrameAdapter::from_first(&valid, &selection(12.0, 0.0, 4.0, 4.0))
                .unwrap_err()
                .code(),
            "selection_empty"
        );

        let (adapter, _) = LongshotFrameAdapter::from_first(&valid, &selection(1.0, 1.0, 4.0, 4.0))
            .expect("有效首帧应可冻结");
        let mut truncated_next = valid.clone();
        truncated_next.rgba = Arc::from(vec![0; 399]);
        assert_eq!(
            adapter.crop_next(&truncated_next).unwrap_err().code(),
            "longshot_frame_invalid"
        );
        for invalid_next in [
            {
                let mut value = valid.clone();
                value.logical_height = 0;
                value
            },
            {
                let mut value = valid.clone();
                value.pixel_width = 0;
                value
            },
            {
                let mut value = valid.clone();
                value.scale_x = f32::NEG_INFINITY;
                value
            },
            {
                let mut value = valid.clone();
                value.scale_y = 0.0;
                value
            },
        ] {
            assert_eq!(
                adapter.crop_next(&invalid_next).unwrap_err().code(),
                "longshot_frame_invalid"
            );
        }
        assert!(adapter.crop_next(&valid).is_ok());
    }

    #[test]
    fn session_budget_is_the_single_frame_limit() {
        assert!(validate_session_frame_budget_for_dimensions(8_192, 1_024).is_ok());
        assert_eq!(
            validate_session_frame_budget_for_dimensions(8_193, 1_024)
                .unwrap_err()
                .code(),
            "longshot_resource_limit"
        );
        assert_eq!(
            validate_session_frame_budget_for_dimensions(u32::MAX, u32::MAX)
                .unwrap_err()
                .code(),
            "longshot_resource_limit"
        );
    }

    #[test]
    fn cropped_frames_drive_the_real_manager() {
        let panorama = nonperiodic(64, 96, 0x1234_5678);
        let first = monitor_with_target(&panorama, 0);
        let second = monitor_with_target(&panorama, 24);
        let (adapter, initial) =
            LongshotFrameAdapter::from_first(&first, &selection(4.0, 5.0, 64.0, 72.0))
                .expect("首帧应可适配");
        let incoming = adapter.crop_next(&second).expect("第二帧应可适配");
        let manager = LongshotManager::new();
        let started = manager.begin(initial).expect("首帧应可开始");
        let outcome = manager
            .append(&started.token, incoming)
            .expect("裁剪帧应可追加");
        assert_eq!(outcome.estimate.displacement_rows, 24);
        assert_eq!(outcome.estimate.overlap_rows, 48);
        assert_eq!(outcome.snapshot.total_height, 96);
        let png = manager.finish_png(&started.token).expect("会话应可完成");
        assert_eq!(
            image::load_from_memory(&png)
                .expect("PNG 应可解码")
                .into_rgba8(),
            panorama
        );
    }

    fn nonperiodic(width: u32, height: u32, seed: u32) -> RgbaImage {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
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
                rgba.extend_from_slice(&[
                    value as u8,
                    (value >> 8) as u8,
                    (value >> 16) as u8,
                    (value >> 24) as u8,
                ]);
            }
        }
        RgbaImage::from_raw(width, height, rgba).expect("固定测试尺寸")
    }

    fn monitor_with_target(panorama: &RgbaImage, top: u32) -> CapturedMonitorFrame {
        let target = imageops::crop_imm(panorama, 0, top, 64, 72).to_image();
        let mut bytes = vec![0; 80 * 100 * 4];
        for row in 0..72 {
            let destination = ((row + 5) * 80 + 4) as usize * 4;
            let source = (row * 64) as usize * 4;
            bytes[destination..destination + 64 * 4]
                .copy_from_slice(&target.as_raw()[source..source + 64 * 4]);
        }
        CapturedMonitorFrame {
            monitor_id: 7,
            x: 0,
            y: 0,
            logical_width: 80,
            logical_height: 100,
            pixel_width: 80,
            pixel_height: 100,
            scale_x: 1.0,
            scale_y: 1.0,
            rgba: Arc::from(bytes),
        }
    }
}
