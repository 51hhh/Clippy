//! 整屏平台帧到固定录屏选区的无缩放裁剪合同。

use crate::capture::RecordingCaptureSpec;
use crate::recording::frame::MAX_FRAME_BYTES;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(in crate::recording) enum RegionFrameError {
    #[error("录屏区域无效或超出冻结显示器")]
    InvalidRegion,
    #[error("录屏显示器像素几何已变化")]
    MonitorGeometryChanged,
    #[error("平台录屏帧字节数与显示器几何不一致")]
    InvalidFrameLength,
    #[error("平台整屏录制超过 64 MiB 帧预算")]
    SourceFrameTooLarge,
}

pub(super) fn validate_selection(selection: RecordingCaptureSpec) -> Result<(), RegionFrameError> {
    let right = selection
        .crop_left
        .checked_add(selection.crop_width)
        .ok_or(RegionFrameError::InvalidRegion)?;
    let bottom = selection
        .crop_top
        .checked_add(selection.crop_height)
        .ok_or(RegionFrameError::InvalidRegion)?;
    if selection.monitor_pixel_width == 0
        || selection.monitor_pixel_height == 0
        || selection.crop_width == 0
        || selection.crop_height == 0
        || right > selection.monitor_pixel_width
        || bottom > selection.monitor_pixel_height
    {
        return Err(RegionFrameError::InvalidRegion);
    }
    checked_rgba_len(
        selection.monitor_pixel_width,
        selection.monitor_pixel_height,
    )
    .filter(|bytes| *bytes <= MAX_FRAME_BYTES)
    .ok_or(RegionFrameError::SourceFrameTooLarge)?;
    checked_rgba_len(selection.crop_width, selection.crop_height)
        .filter(|bytes| *bytes <= MAX_FRAME_BYTES)
        .ok_or(RegionFrameError::InvalidRegion)?;
    Ok(())
}

pub(super) fn crop_tight_rgba(
    selection: RecordingCaptureSpec,
    actual_width: u32,
    actual_height: u32,
    rgba: &[u8],
) -> Result<Box<[u8]>, RegionFrameError> {
    validate_selection(selection)?;
    if actual_width != selection.monitor_pixel_width
        || actual_height != selection.monitor_pixel_height
    {
        return Err(RegionFrameError::MonitorGeometryChanged);
    }
    let source_len = checked_rgba_len(actual_width, actual_height)
        .ok_or(RegionFrameError::InvalidFrameLength)?;
    if rgba.len() != source_len {
        return Err(RegionFrameError::InvalidFrameLength);
    }
    let output_len = checked_rgba_len(selection.crop_width, selection.crop_height)
        .ok_or(RegionFrameError::InvalidRegion)?;
    let source_stride = usize::try_from(actual_width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or(RegionFrameError::InvalidFrameLength)?;
    let output_stride = usize::try_from(selection.crop_width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or(RegionFrameError::InvalidRegion)?;
    let left = usize::try_from(selection.crop_left)
        .ok()
        .and_then(|value| value.checked_mul(4))
        .ok_or(RegionFrameError::InvalidRegion)?;
    let top = usize::try_from(selection.crop_top).map_err(|_| RegionFrameError::InvalidRegion)?;
    let mut output = vec![0_u8; output_len];
    for row in
        0..usize::try_from(selection.crop_height).map_err(|_| RegionFrameError::InvalidRegion)?
    {
        let source_start = top
            .checked_add(row)
            .and_then(|y| y.checked_mul(source_stride))
            .and_then(|offset| offset.checked_add(left))
            .ok_or(RegionFrameError::InvalidRegion)?;
        let source_end = source_start
            .checked_add(output_stride)
            .ok_or(RegionFrameError::InvalidRegion)?;
        let output_start = row
            .checked_mul(output_stride)
            .ok_or(RegionFrameError::InvalidRegion)?;
        output[output_start..output_start + output_stride]
            .copy_from_slice(&rgba[source_start..source_end]);
    }
    Ok(output.into_boxed_slice())
}

fn checked_rgba_len(width: u32, height: u32) -> Option<usize> {
    usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?
        .checked_mul(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection() -> RecordingCaptureSpec {
        RecordingCaptureSpec {
            monitor_id: 7,
            monitor_pixel_width: 4,
            monitor_pixel_height: 3,
            crop_left: 1,
            crop_top: 1,
            crop_width: 2,
            crop_height: 2,
        }
    }

    #[test]
    fn crops_rows_without_scaling_or_hidden_capacity() {
        let rgba = (0_u8..48).collect::<Vec<_>>();
        let cropped = crop_tight_rgba(selection(), 4, 3, &rgba).unwrap();
        assert_eq!(
            cropped.as_ref(),
            &[20, 21, 22, 23, 24, 25, 26, 27, 36, 37, 38, 39, 40, 41, 42, 43]
        );
        assert_eq!(cropped.len(), 16);
    }

    #[test]
    fn rejects_changed_geometry_bad_lengths_and_out_of_bounds_regions() {
        let rgba = vec![0_u8; 48];
        assert_eq!(
            crop_tight_rgba(selection(), 5, 3, &rgba),
            Err(RegionFrameError::MonitorGeometryChanged)
        );
        assert_eq!(
            crop_tight_rgba(selection(), 4, 3, &rgba[..47]),
            Err(RegionFrameError::InvalidFrameLength)
        );
        let mut outside = selection();
        outside.crop_left = 3;
        assert_eq!(
            validate_selection(outside),
            Err(RegionFrameError::InvalidRegion)
        );
    }

    #[test]
    fn rejects_platform_sources_that_need_more_than_one_frame_budget() {
        let oversized = RecordingCaptureSpec {
            monitor_pixel_width: 7680,
            monitor_pixel_height: 4320,
            crop_width: 100,
            crop_height: 100,
            ..selection()
        };
        assert_eq!(
            validate_selection(oversized),
            Err(RegionFrameError::SourceFrameTooLarge)
        );
    }
}
