//! 二维长截图的不可变帧画布。
//!
//! 每次提交保留一张 `Arc<RgbaImage>` 及其有符号位置。导出时按提交倒序绘制、再让
//! 较早帧覆盖重叠区，因此已经提交的像素不会被固定页头或动态内容静默改写。

use super::{
    checked_pixel_count, scaled_height, validate_dimensions, validate_preview_png_bytes,
    CaptureError, MAX_FRAMES, MAX_RAW_BYTES, PREVIEW_MAX_HEIGHT, PREVIEW_MAX_WIDTH,
    RGBA_BYTES_PER_PIXEL,
};
use image::{imageops::FilterType, Rgba, RgbaImage};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}

impl Rect {
    fn from_frame(x: i64, y: i64, image: &RgbaImage) -> Result<Self, CaptureError> {
        let right = x
            .checked_add(i64::from(image.width()))
            .ok_or(CaptureError::LongshotResourceLimit)?;
        let bottom = y
            .checked_add(i64::from(image.height()))
            .ok_or(CaptureError::LongshotResourceLimit)?;
        Ok(Self {
            left: x,
            top: y,
            right,
            bottom,
        })
    }

    fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    fn intersection(self, other: Self) -> Option<Self> {
        let intersection = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        (intersection.left < intersection.right && intersection.top < intersection.bottom)
            .then_some(intersection)
    }

    fn width(self) -> Result<u32, CaptureError> {
        u32::try_from(self.right - self.left).map_err(|_| CaptureError::LongshotResourceLimit)
    }

    fn height(self) -> Result<u32, CaptureError> {
        u32::try_from(self.bottom - self.top).map_err(|_| CaptureError::LongshotResourceLimit)
    }
}

#[derive(Clone)]
struct PlacedFrame {
    image: Arc<RgbaImage>,
    rect: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CanvasAppend {
    Committed,
    Revisited,
}

/// 有界二维画布。帧一经提交便不修改，回访帧不会进入提交列表。
pub(super) struct LongshotCanvas {
    frames: Vec<PlacedFrame>,
    bounds: Rect,
    frame_width: u32,
    frame_height: u32,
    stored_raw_bytes: u64,
}

impl LongshotCanvas {
    pub(super) fn start(first: Arc<RgbaImage>) -> Result<Self, CaptureError> {
        validate_dimensions(first.width(), first.height())?;
        let rect = Rect::from_frame(0, 0, &first)?;
        let stored_raw_bytes = frame_raw_bytes(&first)?;
        Ok(Self {
            frame_width: first.width(),
            frame_height: first.height(),
            frames: vec![PlacedFrame { image: first, rect }],
            bounds: rect,
            stored_raw_bytes,
        })
    }

    pub(super) fn append(
        &mut self,
        image: Arc<RgbaImage>,
        x: i64,
        y: i64,
    ) -> Result<CanvasAppend, CaptureError> {
        if image.dimensions() != (self.frame_width, self.frame_height) {
            return Err(CaptureError::LongshotEstimateSizeMismatch);
        }
        let rect = Rect::from_frame(x, y, &image)?;
        if self.is_covered(rect) {
            return Ok(CanvasAppend::Revisited);
        }

        let next_count = self
            .frames
            .len()
            .checked_add(1)
            .ok_or(CaptureError::LongshotResourceLimit)?;
        if next_count > MAX_FRAMES {
            return Err(CaptureError::LongshotFrameLimit);
        }
        let next_bounds = self.bounds.union(rect);
        let next_width = next_bounds.width()?;
        let next_height = next_bounds.height()?;
        validate_dimensions(next_width, next_height)?;
        let next_stored_bytes = self
            .stored_raw_bytes
            .checked_add(frame_raw_bytes(&image)?)
            .ok_or(CaptureError::LongshotResourceLimit)?;
        let next_canvas_bytes = checked_pixel_count(next_width, next_height)?
            .checked_mul(RGBA_BYTES_PER_PIXEL)
            .ok_or(CaptureError::LongshotResourceLimit)?;
        if next_stored_bytes
            .checked_add(next_canvas_bytes)
            .filter(|bytes| *bytes <= MAX_RAW_BYTES)
            .is_none()
        {
            return Err(CaptureError::LongshotResourceLimit);
        }
        self.frames
            .try_reserve(1)
            .map_err(|_| CaptureError::LongshotAllocationFailed)?;
        self.frames.push(PlacedFrame { image, rect });
        self.bounds = next_bounds;
        self.stored_raw_bytes = next_stored_bytes;
        Ok(CanvasAppend::Committed)
    }

    /// 只移除最后一次显式提交，首帧不能撤销。
    pub(super) fn undo(&mut self) -> Result<Option<(Arc<RgbaImage>, i64, i64)>, CaptureError> {
        if self.frames.len() <= 1 {
            return Ok(None);
        }
        let removed = self.frames.pop().ok_or(CaptureError::LongshotEmpty)?;
        self.stored_raw_bytes = self
            .stored_raw_bytes
            .checked_sub(frame_raw_bytes(&removed.image)?)
            .ok_or(CaptureError::LongshotResourceLimit)?;
        self.bounds = self
            .frames
            .iter()
            .map(|frame| frame.rect)
            .reduce(Rect::union)
            .ok_or(CaptureError::LongshotEmpty)?;
        let current = self.frames.last().ok_or(CaptureError::LongshotEmpty)?;
        Ok(Some((
            Arc::clone(&current.image),
            current.rect.left,
            current.rect.top,
        )))
    }

    pub(super) fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub(super) fn frame_dimensions(&self) -> (u32, u32) {
        (self.frame_width, self.frame_height)
    }

    pub(super) fn dimensions(&self) -> Result<(u32, u32), CaptureError> {
        Ok((self.bounds.width()?, self.bounds.height()?))
    }

    pub(super) fn offset(&self) -> (i64, i64) {
        (self.bounds.left, self.bounds.top)
    }

    pub(super) fn finish_png(&self) -> Result<Vec<u8>, CaptureError> {
        let image = self.materialize()?;
        crate::screenshot::encode_png(image.as_raw(), image.width(), image.height())
            .map_err(CaptureError::codec)
    }

    pub(super) fn preview_png(&self) -> Result<Vec<u8>, CaptureError> {
        let image = self.materialize()?;
        let width_ratio = f64::from(PREVIEW_MAX_WIDTH) / f64::from(image.width());
        let height_ratio = f64::from(PREVIEW_MAX_HEIGHT) / f64::from(image.height());
        let ratio = width_ratio.min(height_ratio).min(1.0);
        let target_width = (f64::from(image.width()) * ratio).round().max(1.0) as u32;
        let target_height =
            scaled_height(image.height(), target_width, image.width())?.min(PREVIEW_MAX_HEIGHT);
        let preview =
            image::imageops::resize(&image, target_width, target_height, FilterType::Triangle);
        let png =
            crate::screenshot::encode_png(preview.as_raw(), preview.width(), preview.height())
                .map_err(CaptureError::codec)?;
        validate_preview_png_bytes(png)
    }

    fn materialize(&self) -> Result<RgbaImage, CaptureError> {
        let (width, height) = self.dimensions()?;
        validate_dimensions(width, height)?;
        let mut output = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 0]));
        // 后提交帧先画，较早帧最后覆盖重叠区，保证已提交像素不可变。
        for frame in self.frames.iter().rev() {
            let target_x = u32::try_from(frame.rect.left - self.bounds.left)
                .map_err(|_| CaptureError::LongshotResourceLimit)?;
            let target_y = u32::try_from(frame.rect.top - self.bounds.top)
                .map_err(|_| CaptureError::LongshotResourceLimit)?;
            image::imageops::replace(
                &mut output,
                frame.image.as_ref(),
                i64::from(target_x),
                i64::from(target_y),
            );
        }
        Ok(output)
    }

    fn is_covered(&self, rect: Rect) -> bool {
        let mut uncovered = vec![rect];
        for frame in &self.frames {
            let mut next = Vec::new();
            for candidate in uncovered {
                subtract_rect(candidate, frame.rect, &mut next);
            }
            if next.is_empty() {
                return true;
            }
            if next.len() > MAX_FRAMES * MAX_FRAMES {
                return false;
            }
            uncovered = next;
        }
        false
    }
}

fn frame_raw_bytes(image: &RgbaImage) -> Result<u64, CaptureError> {
    checked_pixel_count(image.width(), image.height())?
        .checked_mul(RGBA_BYTES_PER_PIXEL)
        .ok_or(CaptureError::LongshotResourceLimit)
}

fn subtract_rect(source: Rect, cover: Rect, output: &mut Vec<Rect>) {
    let Some(overlap) = source.intersection(cover) else {
        output.push(source);
        return;
    };
    if source.top < overlap.top {
        output.push(Rect {
            bottom: overlap.top,
            ..source
        });
    }
    if overlap.bottom < source.bottom {
        output.push(Rect {
            top: overlap.bottom,
            ..source
        });
    }
    if source.left < overlap.left {
        output.push(Rect {
            top: overlap.top,
            right: overlap.left,
            bottom: overlap.bottom,
            ..source
        });
    }
    if overlap.right < source.right {
        output.push(Rect {
            left: overlap.right,
            top: overlap.top,
            bottom: overlap.bottom,
            ..source
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, value: u8) -> Arc<RgbaImage> {
        Arc::new(RgbaImage::from_pixel(
            width,
            height,
            Rgba([value, 0, 0, 255]),
        ))
    }

    #[test]
    fn extends_in_all_directions_and_preserves_earlier_overlap() {
        let mut canvas = LongshotCanvas::start(solid(8, 8, 1)).unwrap();
        assert_eq!(
            canvas.append(solid(8, 8, 2), 0, 4).unwrap(),
            CanvasAppend::Committed
        );
        assert_eq!(
            canvas.append(solid(8, 8, 3), 4, 4).unwrap(),
            CanvasAppend::Committed
        );
        assert_eq!(
            canvas.append(solid(8, 8, 4), -4, -4).unwrap(),
            CanvasAppend::Committed
        );
        assert_eq!(canvas.dimensions().unwrap(), (16, 16));
        assert_eq!(canvas.offset(), (-4, -4));

        let png = canvas.finish_png().unwrap();
        let image = image::load_from_memory(&png).unwrap().into_rgba8();
        assert_eq!(image.get_pixel(4, 4).0[0], 1, "首帧必须覆盖后续重叠像素");
        assert_eq!(image.get_pixel(0, 0).0[0], 4);
        assert_eq!(image.get_pixel(15, 15).0[0], 3);
    }

    #[test]
    fn revisit_does_not_commit_and_undo_only_removes_latest_commit() {
        let first = solid(8, 8, 1);
        let mut canvas = LongshotCanvas::start(Arc::clone(&first)).unwrap();
        assert_eq!(
            canvas.append(solid(8, 8, 2), 0, 4).unwrap(),
            CanvasAppend::Committed
        );
        assert_eq!(
            canvas.append(solid(8, 8, 9), 0, 0).unwrap(),
            CanvasAppend::Revisited
        );
        assert_eq!(canvas.frame_count(), 2);
        let restored = canvas.undo().unwrap().unwrap();
        assert_eq!((restored.1, restored.2), (0, 0));
        assert!(Arc::ptr_eq(&restored.0, &first));
        assert_eq!(canvas.frame_count(), 1);
        assert_eq!(canvas.dimensions().unwrap(), (8, 8));
        assert!(canvas.undo().unwrap().is_none());
    }

    #[test]
    fn fills_a_hole_even_when_bounds_do_not_expand() {
        let mut canvas = LongshotCanvas::start(solid(8, 8, 1)).unwrap();
        canvas.append(solid(8, 8, 2), 8, 0).unwrap();
        canvas.append(solid(8, 8, 3), 0, 8).unwrap();
        canvas.append(solid(8, 8, 4), 8, 8).unwrap();
        assert_eq!(canvas.dimensions().unwrap(), (16, 16));
        assert_eq!(
            canvas.append(solid(8, 8, 5), 4, 4).unwrap(),
            CanvasAppend::Revisited,
            "四个已提交矩形已经联合覆盖中心"
        );
    }
}
