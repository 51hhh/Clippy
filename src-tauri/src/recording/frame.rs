use thiserror::Error;

pub(super) const BYTES_PER_PIXEL: u32 = 4;
pub(super) const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FrameSpec {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub byte_length: usize,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(super) enum FrameError {
    #[error("录屏帧尺寸必须大于零")]
    EmptyDimensions,
    #[error("录屏帧行跨度溢出")]
    StrideOverflow,
    #[error("录屏帧必须是紧凑 RGBA 行布局")]
    InvalidStride,
    #[error("录屏帧字节数溢出")]
    ByteLengthOverflow,
    #[error("录屏单帧超过 64 MiB 上限")]
    FrameTooLarge,
    #[error("录屏帧字节数与尺寸不一致")]
    ByteLengthMismatch,
}

#[derive(Debug)]
pub(super) struct CapturedFrame {
    pub sequence: u64,
    pub captured_at_ns: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    /// 定长切片没有 `Vec` 的隐藏备用 capacity，队列字节预算因此等于实际所有权。
    pub rgba: Box<[u8]>,
}

impl CapturedFrame {
    pub fn validate(&self) -> Result<FrameSpec, FrameError> {
        if self.width == 0 || self.height == 0 {
            return Err(FrameError::EmptyDimensions);
        }
        let expected_stride = self
            .width
            .checked_mul(BYTES_PER_PIXEL)
            .ok_or(FrameError::StrideOverflow)?;
        if self.stride != expected_stride {
            return Err(FrameError::InvalidStride);
        }
        let expected_bytes_u64 = u64::from(self.stride)
            .checked_mul(u64::from(self.height))
            .ok_or(FrameError::ByteLengthOverflow)?;
        let expected_bytes =
            usize::try_from(expected_bytes_u64).map_err(|_| FrameError::ByteLengthOverflow)?;
        if expected_bytes > MAX_FRAME_BYTES {
            return Err(FrameError::FrameTooLarge);
        }
        if self.rgba.len() != expected_bytes {
            return Err(FrameError::ByteLengthMismatch);
        }
        Ok(FrameSpec {
            width: self.width,
            height: self.height,
            stride: self.stride,
            byte_length: expected_bytes,
        })
    }
}
