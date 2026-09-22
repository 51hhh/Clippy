//! 连续重捕获帧的固定选区适配器。
//!
//! 截图后端已经归一化图像方向；这里仅验证同一显示器的几何签名并裁出冻结矩形，
//! 不执行旋转、缩放或 PNG 编码。

use super::session::validate_session_frame_budget_for_dimensions;
use super::CaptureError;
use crate::capture::frame_crop::{selection_pixel_rect, PixelRect};
use crate::capture::CaptureSelection;
use crate::pin::PinOrigin;
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

    /// 自动滚动只使用首帧确认过的裁剪区中心，避免前端提交任意桌面坐标。
    pub(in crate::capture) fn scroll_target(&self) -> Result<(i32, i32), CaptureError> {
        self.scroll_target_for_coordinate_space(cfg!(target_os = "windows"))
    }

    /// Portal 授权必须和首帧冻结的显示器保持同一份逻辑、物理几何事实。
    #[cfg(all(target_os = "linux", feature = "longshot-wayland-auto"))]
    pub(super) fn wayland_monitor_identity(
        &self,
    ) -> Result<super::auto_scroll_wayland::WaylandMonitorIdentity, CaptureError> {
        let monitor = crate::screenshot::wayland_recording_monitor(self.signature.monitor_id)
            .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?;
        if monitor.id != self.signature.monitor_id
            || monitor.logical_x != self.signature.x
            || monitor.logical_y != self.signature.y
            || monitor.logical_width != self.signature.logical_width
            || monitor.logical_height != self.signature.logical_height
            || monitor.pixel_width != self.signature.pixel_width
            || monitor.pixel_height != self.signature.pixel_height
        {
            return Err(CaptureError::LongshotFrameGeometryChanged);
        }
        Ok(super::auto_scroll_wayland::WaylandMonitorIdentity {
            logical_x: monitor.logical_x,
            logical_y: monitor.logical_y,
            logical_width: monitor.logical_width,
            logical_height: monitor.logical_height,
            monitor_count: monitor.monitor_count,
        })
    }

    /// Windows 的原生指针/命中 API 使用物理虚拟桌面坐标；其它已实现后端使用逻辑桌面坐标。
    /// `signature.x/y` 已被截图层归一为逻辑坐标，而 `crop` 始终是物理像素，所以两条换算不能混用。
    fn scroll_target_for_coordinate_space(
        &self,
        windows_physical: bool,
    ) -> Result<(i32, i32), CaptureError> {
        let center_x = (f64::from(self.crop.left) + f64::from(self.crop.right)) / 2.0;
        let center_y = (f64::from(self.crop.top) + f64::from(self.crop.bottom)) / 2.0;
        let scale_x = f64::from(self.signature.scale_x);
        let scale_y = f64::from(self.signature.scale_y);
        let (x, y) = if windows_physical {
            (
                f64::from(self.signature.x) * scale_x + center_x,
                f64::from(self.signature.y) * scale_y + center_y,
            )
        } else {
            (
                f64::from(self.signature.x) + center_x / scale_x,
                f64::from(self.signature.y) + center_y / scale_y,
            )
        };
        if !x.is_finite()
            || !y.is_finite()
            || x < f64::from(i32::MIN)
            || x > f64::from(i32::MAX)
            || y < f64::from(i32::MIN)
            || y > f64::from(i32::MAX)
        {
            return Err(CaptureError::LongshotFrameInvalid);
        }
        Ok((x.round() as i32, y.round() as i32))
    }

    /// 根据冻结首帧与实际物理裁剪区反算最终产物的桌面全局逻辑来源矩形。
    ///
    /// 最终 PNG 的宽度不能偏离固定裁剪区，且高度至少要包含首帧裁剪区；这样不会把
    /// 前端原始选区的舍入或 clamp 前坐标泄漏到后续贴图定位。
    #[cfg(test)]
    pub(in crate::capture) fn output_origin(
        &self,
        final_width: u32,
        final_height: u32,
    ) -> Result<PinOrigin, CaptureError> {
        let crop_width = self
            .crop
            .right
            .checked_sub(self.crop.left)
            .ok_or(CaptureError::LongshotFrameInvalid)?;
        if final_width != crop_width {
            return Err(CaptureError::LongshotWidthMismatch);
        }
        self.output_origin_with_offset(final_width, final_height, 0, 0)
    }

    /// 根据二维 union 的相对首帧偏移反算最终产物位置。
    pub(in crate::capture) fn output_origin_with_offset(
        &self,
        final_width: u32,
        final_height: u32,
        offset_x: i64,
        offset_y: i64,
    ) -> Result<PinOrigin, CaptureError> {
        let crop_width = self
            .crop
            .right
            .checked_sub(self.crop.left)
            .ok_or(CaptureError::LongshotFrameInvalid)?;
        let crop_height = self
            .crop
            .bottom
            .checked_sub(self.crop.top)
            .ok_or(CaptureError::LongshotFrameInvalid)?;
        if offset_x > 0
            || offset_y > 0
            || final_width < crop_width
            || final_height < crop_height
            || i64::from(crop_width)
                .checked_sub(offset_x)
                .filter(|right| *right <= i64::from(final_width))
                .is_none()
            || i64::from(crop_height)
                .checked_sub(offset_y)
                .filter(|bottom| *bottom <= i64::from(final_height))
                .is_none()
        {
            return Err(CaptureError::LongshotFrameInvalid);
        }

        let scale_x = f64::from(self.signature.scale_x);
        let scale_y = f64::from(self.signature.scale_y);
        let origin = PinOrigin {
            x: f64::from(self.signature.x)
                + (f64::from(self.crop.left) + offset_x as f64) / scale_x,
            y: f64::from(self.signature.y) + (f64::from(self.crop.top) + offset_y as f64) / scale_y,
            width: f64::from(final_width) / scale_x,
            height: f64::from(final_height) / scale_y,
        };
        origin.sanitized().ok_or(CaptureError::LongshotFrameInvalid)
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
    fn output_origin_uses_frozen_global_frame_crop_and_each_axis_scale() {
        let frame = encoded_frame(-1920, -120, 80, 50, 100, 75, 1.25, 1.5, 40);
        let (adapter, cropped) =
            LongshotFrameAdapter::from_first(&frame, &selection(1.1, 2.2, 4.4, 3.3))
                .expect("带分数缩放的首帧应可冻结");
        assert_eq!(cropped.dimensions(), (6, 6));

        let origin = adapter
            .output_origin(6, 15)
            .expect("拼接后的物理尺寸应可反算来源矩形");
        assert!((origin.x - -1919.2).abs() < f64::EPSILON);
        assert!((origin.y - -118.0).abs() < f64::EPSILON);
        assert!((origin.width - 4.8).abs() < f64::EPSILON);
        assert!((origin.height - 10.0).abs() < f64::EPSILON);
        assert_eq!(
            adapter
                .scroll_target_for_coordinate_space(false)
                .expect("逻辑滚动点应使用同一冻结几何"),
            (-1917, -116)
        );
        assert_eq!(
            adapter
                .scroll_target_for_coordinate_space(true)
                .expect("Windows 滚动点应恢复到物理虚拟桌面"),
            (-2396, -174)
        );
        let platform_expected = if cfg!(target_os = "windows") {
            (-2396, -174)
        } else {
            (-1917, -116)
        };
        assert_eq!(adapter.scroll_target().unwrap(), platform_expected);
    }

    #[test]
    fn output_origin_uses_clamped_crop_and_rejects_invalid_final_dimensions() {
        let frame = encoded_frame(-1920, -120, 80, 50, 100, 75, 1.25, 1.5, 41);
        let (adapter, cropped) =
            LongshotFrameAdapter::from_first(&frame, &selection(-2.0, -1.0, 5.0, 5.0))
                .expect("越界选区应按实际帧边界冻结");
        assert_eq!(cropped.dimensions(), (4, 6));

        let origin = adapter
            .output_origin(4, 9)
            .expect("clamp 后矩形应可反算来源");
        assert_eq!(origin.x, -1920.0);
        assert_eq!(origin.y, -120.0);
        assert!((origin.width - 3.2).abs() < f64::EPSILON);
        assert_eq!(origin.height, 6.0);

        assert_eq!(
            adapter.output_origin(5, 9).unwrap_err().code(),
            "longshot_width_mismatch"
        );
        assert_eq!(
            adapter.output_origin(4, 5).unwrap_err().code(),
            "longshot_frame_invalid"
        );
        assert_eq!(
            adapter.output_origin(0, 0).unwrap_err().code(),
            "longshot_width_mismatch"
        );
        assert!(
            adapter.output_origin(4, 9).is_ok(),
            "失败不能污染冻结适配器"
        );
    }

    #[test]
    fn output_origin_applies_signed_union_offset_on_both_axes() {
        let source = encoded_frame(-100, -50, 100, 50, 200, 100, 2.0, 2.0, 17);
        let selection = CaptureSelection {
            session_id: "capture-origin-2d".to_string(),
            monitor_id: 7,
            x: 30.0,
            y: 20.0,
            width: 40.0,
            height: 30.0,
        };
        let (adapter, _) = LongshotFrameAdapter::from_first(&source, &selection).unwrap();
        let origin = adapter
            .output_origin_with_offset(120, 100, -40, -20)
            .unwrap();
        assert_eq!(origin.x, -90.0);
        assert_eq!(origin.y, -40.0);
        assert_eq!(origin.width, 60.0);
        assert_eq!(origin.height, 50.0);
        assert!(matches!(
            adapter.output_origin_with_offset(120, 100, 1, 0),
            Err(CaptureError::LongshotFrameInvalid)
        ));
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
