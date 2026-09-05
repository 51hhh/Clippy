//! 普通截图与长截图共用的逻辑选区到物理像素矩形换算。
//!
//! 选区坐标始终相对其所在显示器；显示器在桌面中的逻辑原点不参与换算。

use super::{CaptureError, CaptureSelection};
use crate::screenshot::CapturedMonitorFrame;

/// 已 clamp 到一张物理帧内的半开像素矩形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PixelRect {
    pub(super) left: u32,
    pub(super) top: u32,
    pub(super) right: u32,
    pub(super) bottom: u32,
}

impl PixelRect {
    pub(super) fn width(self) -> u32 {
        self.right - self.left
    }

    pub(super) fn height(self) -> u32 {
        self.bottom - self.top
    }

    pub(super) fn as_crop(self) -> (u32, u32, u32, u32) {
        (self.left, self.top, self.width(), self.height())
    }
}

/// 沿用普通截图既有的 floor/ceil 与 clamp 语义，把逻辑选区映射到物理帧。
pub(super) fn selection_pixel_rect(
    frame: &CapturedMonitorFrame,
    selection: &CaptureSelection,
) -> Result<PixelRect, CaptureError> {
    for value in [selection.x, selection.y, selection.width, selection.height] {
        if !value.is_finite() {
            return Err(CaptureError::SelectionNotFinite);
        }
    }
    if selection.width < 2.0 || selection.height < 2.0 {
        return Err(CaptureError::SelectionTooSmall);
    }

    // 保持原普通截图的转换顺序和 f64 -> u32 语义，不能让长截图另起一套边界规则。
    let left = (selection.x.max(0.0) * frame.scale_x as f64).floor() as u32;
    let top = (selection.y.max(0.0) * frame.scale_y as f64).floor() as u32;
    let right = ((selection.x + selection.width).min(frame.logical_width as f64)
        * frame.scale_x as f64)
        .ceil() as u32;
    let bottom = ((selection.y + selection.height).min(frame.logical_height as f64)
        * frame.scale_y as f64)
        .ceil() as u32;
    let (left, top) = (left.min(frame.pixel_width), top.min(frame.pixel_height));
    let (right, bottom) = (right.min(frame.pixel_width), bottom.min(frame.pixel_height));
    if right <= left || bottom <= top {
        return Err(CaptureError::SelectionEmpty);
    }
    Ok(PixelRect {
        left,
        top,
        right,
        bottom,
    })
}
