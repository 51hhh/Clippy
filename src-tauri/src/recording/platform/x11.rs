//! Linux X11 根窗口区域帧源。
//!
//! 连接在整个会话内复用；每帧只请求选区物理矩形，不循环抓整块显示器再裁切。调用方必须使用
//! 冻结截图会话核验后的 RandR 物理区域，不能把前端提交的逻辑坐标直接传进来。

use crate::recording::frame::{CapturedFrame, FrameError, MAX_FRAME_BYTES};
use std::time::Instant;
use thiserror::Error;
use x11rb::connection::Connection;
use x11rb::image::{Image, PixelLayout};
use x11rb::protocol::xfixes::{ConnectionExt as _, GetCursorImageReply};
use x11rb::protocol::xproto::Visualtype;
use x11rb::rust_connection::RustConnection;

const MAX_CURSOR_PIXELS: usize = 512 * 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct X11PhysicalRegion {
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(super) enum X11FrameSourceError {
    #[error("X11 录屏区域无效或超出根窗口")]
    InvalidRegion,
    #[error("X11 录屏连接失败: {0}")]
    Connect(String),
    #[error("X11 录屏取帧失败: {0}")]
    Capture(String),
    #[error("X11 录屏 visual 不支持 TrueColor/DirectColor")]
    UnsupportedVisual,
    #[error("X11 录屏帧序号耗尽")]
    SequenceExhausted,
    #[error("X11 录屏光标数据无效")]
    InvalidCursor,
    #[error(transparent)]
    Frame(#[from] FrameError),
}

pub(super) struct X11RegionFrameSource {
    connection: RustConnection,
    root: u32,
    region: X11PhysicalRegion,
    clock_origin: Instant,
    last_timestamp_ns: Option<u64>,
    next_sequence: u64,
}

impl X11RegionFrameSource {
    pub fn connect(region: X11PhysicalRegion) -> Result<Self, X11FrameSourceError> {
        let (connection, screen_number) = RustConnection::connect(None)
            .map_err(|error| X11FrameSourceError::Connect(error.to_string()))?;
        let screen = connection
            .setup()
            .roots
            .get(screen_number)
            .ok_or_else(|| X11FrameSourceError::Connect("X11 screen 索引无效".to_string()))?;
        validate_region(screen.width_in_pixels, screen.height_in_pixels, region)?;
        connection
            .xfixes_query_version(5, 0)
            .map_err(|error| X11FrameSourceError::Connect(error.to_string()))?
            .reply()
            .map_err(|error| X11FrameSourceError::Connect(error.to_string()))?;
        Ok(Self {
            root: screen.root,
            connection,
            region,
            clock_origin: Instant::now(),
            last_timestamp_ns: None,
            next_sequence: 0,
        })
    }

    pub fn capture_next(&mut self) -> Result<CapturedFrame, X11FrameSourceError> {
        let sampled_at = self
            .clock_origin
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        let captured_at_ns = self
            .last_timestamp_ns
            .map_or(sampled_at, |last| sampled_at.max(last.saturating_add(1)));
        let (image, visual_id) = Image::get(
            &self.connection,
            self.root,
            self.region.x,
            self.region.y,
            self.region.width,
            self.region.height,
        )
        .map_err(|error| X11FrameSourceError::Capture(error.to_string()))?;
        let visual = find_visual(self.connection.setup(), visual_id)
            .ok_or(X11FrameSourceError::UnsupportedVisual)?;
        let layout = PixelLayout::from_visual_type(visual)
            .map_err(|_| X11FrameSourceError::UnsupportedVisual)?;
        let mut rgba = decode_rgba(&image, layout)?;
        let cursor = self
            .connection
            .xfixes_get_cursor_image()
            .map_err(|error| X11FrameSourceError::Capture(error.to_string()))?
            .reply()
            .map_err(|error| X11FrameSourceError::Capture(error.to_string()))?;
        composite_cursor(&mut rgba, self.region, &cursor)?;
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(X11FrameSourceError::SequenceExhausted)?;
        self.last_timestamp_ns = Some(captured_at_ns);
        let frame = CapturedFrame {
            sequence,
            captured_at_ns,
            width: u32::from(self.region.width),
            height: u32::from(self.region.height),
            stride: u32::from(self.region.width) * 4,
            rgba,
        };
        frame.validate()?;
        Ok(frame)
    }
}

fn composite_cursor(
    rgba: &mut [u8],
    region: X11PhysicalRegion,
    cursor: &GetCursorImageReply,
) -> Result<(), X11FrameSourceError> {
    let cursor_pixels = usize::from(cursor.width)
        .checked_mul(usize::from(cursor.height))
        .filter(|pixels| *pixels <= MAX_CURSOR_PIXELS)
        .ok_or(X11FrameSourceError::InvalidCursor)?;
    if cursor.width == 0
        || cursor.height == 0
        || cursor.cursor_image.len() != cursor_pixels
        || rgba.len() != usize::from(region.width) * usize::from(region.height) * 4
    {
        return Err(X11FrameSourceError::InvalidCursor);
    }

    let cursor_left = i32::from(cursor.x) - i32::from(cursor.xhot);
    let cursor_top = i32::from(cursor.y) - i32::from(cursor.yhot);
    let region_left = i32::from(region.x);
    let region_top = i32::from(region.y);
    let left = cursor_left.max(region_left);
    let top = cursor_top.max(region_top);
    let right = (cursor_left + i32::from(cursor.width)).min(region_left + i32::from(region.width));
    let bottom = (cursor_top + i32::from(cursor.height)).min(region_top + i32::from(region.height));
    if left >= right || top >= bottom {
        return Ok(());
    }

    for root_y in top..bottom {
        for root_x in left..right {
            let cursor_x = usize::try_from(root_x - cursor_left)
                .map_err(|_| X11FrameSourceError::InvalidCursor)?;
            let cursor_y = usize::try_from(root_y - cursor_top)
                .map_err(|_| X11FrameSourceError::InvalidCursor)?;
            let cursor_offset = cursor_y * usize::from(cursor.width) + cursor_x;
            let pixel = cursor.cursor_image[cursor_offset];
            let alpha = (pixel >> 24) as u8;
            if alpha == 0 {
                continue;
            }

            let frame_x = usize::try_from(root_x - region_left)
                .map_err(|_| X11FrameSourceError::InvalidCursor)?;
            let frame_y = usize::try_from(root_y - region_top)
                .map_err(|_| X11FrameSourceError::InvalidCursor)?;
            let frame_offset = (frame_y * usize::from(region.width) + frame_x) * 4;
            let inverse_alpha = u16::from(255 - alpha);
            for (channel, shift) in [(0, 16), (1, 8), (2, 0)] {
                let source = ((pixel >> shift) & 0xFF) as u16;
                let destination = u16::from(rgba[frame_offset + channel]);
                rgba[frame_offset + channel] =
                    (source + (destination * inverse_alpha + 127) / 255).min(255) as u8;
            }
            rgba[frame_offset + 3] = 255;
        }
    }
    Ok(())
}

fn validate_region(
    screen_width: u16,
    screen_height: u16,
    region: X11PhysicalRegion,
) -> Result<(), X11FrameSourceError> {
    let x = u16::try_from(region.x).map_err(|_| X11FrameSourceError::InvalidRegion)?;
    let y = u16::try_from(region.y).map_err(|_| X11FrameSourceError::InvalidRegion)?;
    let right = x
        .checked_add(region.width)
        .ok_or(X11FrameSourceError::InvalidRegion)?;
    let bottom = y
        .checked_add(region.height)
        .ok_or(X11FrameSourceError::InvalidRegion)?;
    let frame_bytes = usize::from(region.width)
        .checked_mul(usize::from(region.height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(X11FrameSourceError::InvalidRegion)?;
    if region.width == 0
        || region.height == 0
        || right > screen_width
        || bottom > screen_height
        || frame_bytes > MAX_FRAME_BYTES
    {
        return Err(X11FrameSourceError::InvalidRegion);
    }
    Ok(())
}

fn find_visual(setup: &x11rb::protocol::xproto::Setup, visual_id: u32) -> Option<Visualtype> {
    setup
        .roots
        .iter()
        .flat_map(|screen| &screen.allowed_depths)
        .flat_map(|depth| &depth.visuals)
        .find(|visual| visual.visual_id == visual_id)
        .copied()
}

fn decode_rgba(image: &Image<'_>, layout: PixelLayout) -> Result<Box<[u8]>, X11FrameSourceError> {
    let byte_length = usize::from(image.width())
        .checked_mul(usize::from(image.height()))
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|bytes| *bytes <= MAX_FRAME_BYTES)
        .ok_or(X11FrameSourceError::InvalidRegion)?;
    let mut rgba = vec![0_u8; byte_length];
    for y in 0..image.height() {
        for x in 0..image.width() {
            let (red, green, blue) = layout.decode(image.get_pixel(x, y));
            let offset = (usize::from(y) * usize::from(image.width()) + usize::from(x)) * 4;
            rgba[offset] = (red >> 8) as u8;
            rgba[offset + 1] = (green >> 8) as u8;
            rgba[offset + 2] = (blue >> 8) as u8;
            rgba[offset + 3] = 255;
        }
    }
    Ok(rgba.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;
    use x11rb::image::{BitsPerPixel, ColorComponent, ImageOrder, ScanlinePad};

    #[test]
    fn region_validation_rejects_empty_negative_overflow_and_memory_budget() {
        let valid = X11PhysicalRegion {
            x: 10,
            y: 20,
            width: 100,
            height: 50,
        };
        assert_eq!(validate_region(1920, 1080, valid), Ok(()));
        for invalid in [
            X11PhysicalRegion { width: 0, ..valid },
            X11PhysicalRegion { x: -1, ..valid },
            X11PhysicalRegion { x: 1900, ..valid },
            X11PhysicalRegion {
                width: u16::MAX,
                height: u16::MAX,
                ..valid
            },
        ] {
            assert_eq!(
                validate_region(1920, 1080, invalid),
                Err(X11FrameSourceError::InvalidRegion)
            );
        }
    }

    #[test]
    fn visual_masks_and_server_byte_order_decode_to_tight_rgba() {
        let image = Image::new(
            2,
            1,
            ScanlinePad::Pad32,
            24,
            BitsPerPixel::B32,
            ImageOrder::LsbFirst,
            Cow::Borrowed(&[0x33, 0x22, 0x11, 0, 0xCC, 0xBB, 0xAA, 0]),
        )
        .unwrap();
        let layout = PixelLayout::new(
            ColorComponent::from_mask(0x00FF_0000).unwrap(),
            ColorComponent::from_mask(0x0000_FF00).unwrap(),
            ColorComponent::from_mask(0x0000_00FF).unwrap(),
        );
        assert_eq!(
            decode_rgba(&image, layout).unwrap().as_ref(),
            &[0x11, 0x22, 0x33, 255, 0xAA, 0xBB, 0xCC, 255]
        );
    }

    fn cursor(
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        xhot: u16,
        yhot: u16,
        cursor_image: Vec<u32>,
    ) -> GetCursorImageReply {
        GetCursorImageReply {
            x,
            y,
            width,
            height,
            xhot,
            yhot,
            cursor_image,
            ..GetCursorImageReply::default()
        }
    }

    #[test]
    fn premultiplied_argb_cursor_blends_and_clips_at_region_edge() {
        let region = X11PhysicalRegion {
            x: 10,
            y: 20,
            width: 2,
            height: 2,
        };
        let mut rgba = [100_u8, 120, 140, 255].repeat(4);
        // 热点位于 (10, 20)，2x2 光标的左上角为 (9, 19)，只有右下像素落入选区。
        let cursor = cursor(10, 20, 2, 2, 1, 1, vec![0, 0, 0, 0x8064_3219]);
        composite_cursor(&mut rgba, region, &cursor).unwrap();
        assert_eq!(&rgba[..4], &[150, 110, 95, 255]);
        assert_eq!(&rgba[4..], &[100_u8, 120, 140, 255].repeat(3));
    }

    #[test]
    fn cursor_outside_region_is_ignored_without_touching_frame() {
        let region = X11PhysicalRegion {
            x: 100,
            y: 100,
            width: 2,
            height: 2,
        };
        let mut rgba = vec![12_u8; 16];
        let original = rgba.clone();
        let cursor = cursor(0, 0, 1, 1, 0, 0, vec![0xFFFF_FFFF]);
        composite_cursor(&mut rgba, region, &cursor).unwrap();
        assert_eq!(rgba, original);
    }

    #[test]
    fn cursor_rejects_length_mismatch_and_pixel_budget_overflow() {
        let region = X11PhysicalRegion {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        let mut rgba = vec![0_u8; 4];
        assert_eq!(
            composite_cursor(&mut rgba, region, &cursor(0, 0, 1, 1, 0, 0, vec![])),
            Err(X11FrameSourceError::InvalidCursor)
        );
        assert_eq!(
            composite_cursor(&mut rgba, region, &cursor(0, 0, 513, 513, 0, 0, Vec::new())),
            Err(X11FrameSourceError::InvalidCursor)
        );
    }

    #[test]
    #[ignore = "需要真实 X11/XWayland 根窗口，不保存或输出捕获像素"]
    fn native_region_source_reuses_connection_and_advances_identity() {
        let mut source = X11RegionFrameSource::connect(X11PhysicalRegion {
            x: 0,
            y: 0,
            width: 32,
            height: 32,
        })
        .expect("连接 X11 根窗口失败");
        let first = source.capture_next().expect("首帧捕获失败");
        let second = source.capture_next().expect("次帧捕获失败");
        assert_eq!((first.sequence, second.sequence), (0, 1));
        assert!(second.captured_at_ns > first.captured_at_ns);
        assert_eq!((first.width, first.height, first.stride), (32, 32, 128));
        assert_eq!(first.rgba.len(), 32 * 32 * 4);
        assert_eq!(second.rgba.len(), 32 * 32 * 4);
    }
}
