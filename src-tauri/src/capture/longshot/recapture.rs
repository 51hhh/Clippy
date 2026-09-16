//! 长截图的一次性显示器帧重捕获边界。
//!
//! provider 只负责调用统一截图后端并把目标帧移交给上层；选区、像素校验和裁剪由
//! [`super::frame_adapter::LongshotFrameAdapter`] 负责。

use super::CaptureError;
use crate::screenshot::CapturedMonitorFrame;

/// 捕获一次全部显示器，并移出指定显示器的原始帧。
///
/// 入口保持同步，后续 controller 再决定是否把它放进 blocking worker。
#[allow(dead_code)]
pub(in crate::capture) fn capture_monitor_frame(
    monitor_id: u32,
) -> Result<CapturedMonitorFrame, CaptureError> {
    capture_monitor_frame_with(monitor_id, crate::screenshot::capture_monitor_frames)
}

/// 以一次性闭包注入截图后端，供生产入口和确定性测试共用选择及错误映射逻辑。
fn capture_monitor_frame_with<F>(
    monitor_id: u32,
    capture: F,
) -> Result<CapturedMonitorFrame, CaptureError>
where
    F: FnOnce() -> anyhow::Result<Vec<CapturedMonitorFrame>>,
{
    let frames = capture().map_err(|error| CaptureError::Screenshot(format!("{error:#}")))?;
    if frames.is_empty() {
        return Err(CaptureError::NoMonitorFrames);
    }
    frames
        .into_iter()
        .find(|frame| frame.monitor_id == monitor_id)
        .ok_or(CaptureError::LongshotRecaptureMonitorMissing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::longshot::frame_adapter::LongshotFrameAdapter;
    use crate::capture::CaptureSelection;
    use std::sync::Arc;

    fn frame(monitor_id: u32, pixel_width: u32, pixel_height: u32) -> CapturedMonitorFrame {
        let mut rgba = Vec::with_capacity((pixel_width * pixel_height * 4) as usize);
        for pixel in 0..pixel_width * pixel_height {
            rgba.extend_from_slice(&[monitor_id as u8, pixel as u8, (pixel >> 8) as u8, 255]);
        }
        CapturedMonitorFrame {
            monitor_id,
            x: 10,
            y: 20,
            logical_width: pixel_width,
            logical_height: pixel_height,
            pixel_width,
            pixel_height,
            scale_x: 1.0,
            scale_y: 1.0,
            rgba: Arc::from(rgba),
        }
    }

    fn selection(monitor_id: u32) -> CaptureSelection {
        CaptureSelection {
            session_id: "recapture-test".to_string(),
            monitor_id,
            x: 1.0,
            y: 1.0,
            width: 2.0,
            height: 2.0,
        }
    }

    #[test]
    fn selects_first_matching_frame_without_copying_pixels_and_calls_capture_once() {
        let first = frame(1, 2, 2);
        let target = frame(7, 3, 2);
        let last = frame(9, 2, 2);
        let target_pixels = target.rgba.clone();
        let calls = std::cell::Cell::new(0);
        let result = capture_monitor_frame_with(7, || {
            calls.set(calls.get() + 1);
            Ok(vec![first, target, last])
        })
        .expect("中部目标帧应被选中");

        assert_eq!(calls.get(), 1);
        assert_eq!(result.monitor_id, 7);
        assert_eq!(result.pixel_width, 3);
        assert!(Arc::ptr_eq(&result.rgba, &target_pixels));
    }

    #[test]
    fn selects_target_at_each_position_and_preserves_geometry() {
        for target_position in 0..3 {
            let target = frame(7, 3, 2);
            let expected = (
                target.x,
                target.y,
                target.logical_width,
                target.logical_height,
                target.pixel_width,
                target.pixel_height,
                target.scale_x,
                target.scale_y,
            );
            let mut frames = vec![frame(1, 2, 2), frame(2, 2, 2), frame(3, 2, 2)];
            frames[target_position] = target;
            let result = capture_monitor_frame_with(7, || Ok(frames)).expect("目标帧应存在");
            assert_eq!(
                (
                    result.x,
                    result.y,
                    result.logical_width,
                    result.logical_height,
                    result.pixel_width,
                    result.pixel_height,
                    result.scale_x,
                    result.scale_y,
                ),
                expected
            );
        }
    }

    #[test]
    fn duplicate_monitor_ids_choose_the_first_match() {
        let first = frame(7, 2, 2);
        let mut second = frame(7, 2, 2);
        second.x = 999;
        let expected_pixels = first.rgba.clone();
        let result = capture_monitor_frame_with(7, || Ok(vec![first, second]))
            .expect("重复 monitor id 应选首个");
        assert_eq!(result.x, 10);
        assert!(Arc::ptr_eq(&result.rgba, &expected_pixels));
    }

    #[test]
    fn maps_capture_error_with_context_and_calls_capture_once() {
        let calls = std::cell::Cell::new(0);
        let error = capture_monitor_frame_with(7, || {
            calls.set(calls.get() + 1);
            Err(anyhow::anyhow!("sentinel backend failure"))
        })
        .expect_err("底层错误必须向上传递");
        assert_eq!(calls.get(), 1);
        assert_eq!(error.code(), "screenshot");
        assert!(error.to_string().contains("sentinel backend failure"));
    }

    #[test]
    fn keeps_empty_and_missing_monitor_errors_distinct() {
        let empty = capture_monitor_frame_with(7, || Ok(Vec::new())).expect_err("空列表应失败");
        assert_eq!(empty.code(), "no_monitor_frames");

        let missing = capture_monitor_frame_with(7, || Ok(vec![frame(1, 2, 2)]))
            .expect_err("缺少目标显示器应失败");
        assert_eq!(missing.code(), "longshot_recapture_monitor_missing");
    }

    #[test]
    fn moves_malformed_frame_into_real_adapter_without_provider_validation() {
        let mut malformed = frame(7, 4, 4);
        malformed.rgba = Arc::from(vec![1, 2, 3]);
        let selected = capture_monitor_frame_with(7, || Ok(vec![malformed]))
            .expect("provider 不应校验目标帧像素");
        let error = LongshotFrameAdapter::from_first(&selected, &selection(7))
            .expect_err("真实 adapter 应拒绝畸形 RGBA");
        assert_eq!(error.code(), "longshot_frame_invalid");
    }
}
