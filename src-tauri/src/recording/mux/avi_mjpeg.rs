//! MJPEG/AVI 诊断分段编码器。
//!
//! 每个分段都是可独立播放的 AVI 1.0 文件，用来验证录屏 journal、固定帧率时间线和恢复链。
//! 它受 4 GiB RIFF 与有损 JPEG 限制，不能在没有 A/B 数据时成为产品默认编码器。

use image::codecs::jpeg::JpegEncoder;
use image::{ImageBuffer, Rgba};
use std::io::{self, Seek, SeekFrom, Write};
use thiserror::Error;

const NANOS_PER_SECOND: u128 = 1_000_000_000;
const AVIIF_KEYFRAME: u32 = 0x10;
const AVIF_HAS_INDEX: u32 = 0x10;
const MAX_AVI_FRAMES: u64 = 18_000;

#[derive(Debug, Error)]
pub(in crate::recording) enum AviMjpegError {
    #[error("MJPEG/AVI 配置无效")]
    InvalidConfiguration,
    #[error("MJPEG 输入 RGBA 字节数与尺寸不一致")]
    InvalidFrame,
    #[error("MJPEG 输入时间戳必须严格递增并从零开始")]
    InvalidTimestamp,
    #[error("MJPEG/AVI 单分段超过 18,000 帧上限")]
    FrameLimit,
    #[error("MJPEG/AVI 1.0 分段超过 4 GiB 上限")]
    ContainerTooLarge,
    #[error("MJPEG 编码失败: {0}")]
    Encode(#[from] image::ImageError),
    #[error("MJPEG/AVI 写入失败: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, Copy)]
struct HeaderOffsets {
    microseconds_per_frame: u64,
    maximum_bytes_per_second: u64,
    total_frames: u64,
    avih_suggested_buffer: u64,
    stream_length: u64,
    stream_suggested_buffer: u64,
    bitmap_size_image: u64,
}

#[derive(Debug, Clone, Copy)]
struct IndexEntry {
    offset: u32,
    size: u32,
}

#[derive(Debug)]
struct PendingFrame {
    slot: u64,
    jpeg: Vec<u8>,
}

pub(in crate::recording) struct AviMjpegWriter<W: Write + Seek> {
    writer: W,
    width: u32,
    height: u32,
    fps_numerator: u32,
    fps_denominator: u32,
    quality: u8,
    riff_start: u64,
    riff_size_offset: u64,
    movi_size_offset: u64,
    movi_fourcc_offset: u64,
    header_offsets: HeaderOffsets,
    pending: Option<PendingFrame>,
    last_presentation_ns: Option<u64>,
    index: Vec<IndexEntry>,
    maximum_jpeg_bytes: u32,
}

pub(in crate::recording) struct AviMjpegOutput<W> {
    pub writer: W,
    pub frame_count: u64,
}

impl<W: Write + Seek> AviMjpegWriter<W> {
    pub fn new(
        mut writer: W,
        width: u32,
        height: u32,
        fps_numerator: u32,
        fps_denominator: u32,
        quality: u8,
    ) -> Result<Self, AviMjpegError> {
        if width == 0
            || height == 0
            || width > i16::MAX as u32
            || height > i16::MAX as u32
            || fps_numerator == 0
            || fps_denominator == 0
            || u64::from(fps_numerator) > 240 * u64::from(fps_denominator)
            || !(1..=100).contains(&quality)
        {
            return Err(AviMjpegError::InvalidConfiguration);
        }

        let riff_start = writer.stream_position()?;
        if riff_start != 0 {
            return Err(AviMjpegError::InvalidConfiguration);
        }
        writer.write_all(b"RIFF")?;
        let riff_size_offset = writer.stream_position()?;
        write_u32(&mut writer, 0)?;
        writer.write_all(b"AVI ")?;

        let (header, relative_offsets) =
            build_header(width, height, fps_numerator, fps_denominator);
        let header_start = writer.stream_position()?;
        writer.write_all(&header)?;
        let header_offsets = HeaderOffsets {
            microseconds_per_frame: header_start + relative_offsets.microseconds_per_frame,
            maximum_bytes_per_second: header_start + relative_offsets.maximum_bytes_per_second,
            total_frames: header_start + relative_offsets.total_frames,
            avih_suggested_buffer: header_start + relative_offsets.avih_suggested_buffer,
            stream_length: header_start + relative_offsets.stream_length,
            stream_suggested_buffer: header_start + relative_offsets.stream_suggested_buffer,
            bitmap_size_image: header_start + relative_offsets.bitmap_size_image,
        };

        writer.write_all(b"LIST")?;
        let movi_size_offset = writer.stream_position()?;
        write_u32(&mut writer, 0)?;
        let movi_fourcc_offset = writer.stream_position()?;
        writer.write_all(b"movi")?;

        Ok(Self {
            writer,
            width,
            height,
            fps_numerator,
            fps_denominator,
            quality,
            riff_start,
            riff_size_offset,
            movi_size_offset,
            movi_fourcc_offset,
            header_offsets,
            pending: None,
            last_presentation_ns: None,
            index: Vec::new(),
            maximum_jpeg_bytes: 0,
        })
    }

    pub fn push_rgba(&mut self, rgba: &[u8], presentation_at_ns: u64) -> Result<(), AviMjpegError> {
        let expected = u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .and_then(|pixels| pixels.checked_mul(4))
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or(AviMjpegError::InvalidFrame)?;
        if rgba.len() != expected
            || self
                .last_presentation_ns
                .is_some_and(|last| presentation_at_ns <= last)
            || (self.last_presentation_ns.is_none() && presentation_at_ns != 0)
        {
            return Err(if rgba.len() != expected {
                AviMjpegError::InvalidFrame
            } else {
                AviMjpegError::InvalidTimestamp
            });
        }
        let target_slot = self.slot_for(presentation_at_ns)?;
        let jpeg = self.encode_jpeg(rgba)?;

        if let Some(pending) = self.pending.as_ref() {
            if target_slot < pending.slot {
                return Err(AviMjpegError::InvalidTimestamp);
            }
            if target_slot > pending.slot {
                self.flush_pending_until(target_slot)?;
            }
        }
        self.pending = Some(PendingFrame {
            slot: target_slot,
            jpeg,
        });
        self.last_presentation_ns = Some(presentation_at_ns);
        Ok(())
    }

    pub fn finish(self, duration_ns: u64) -> Result<W, AviMjpegError> {
        Ok(self.finish_with_stats(duration_ns)?.writer)
    }

    pub fn finish_with_stats(
        mut self,
        duration_ns: u64,
    ) -> Result<AviMjpegOutput<W>, AviMjpegError> {
        let last = self
            .last_presentation_ns
            .ok_or(AviMjpegError::InvalidTimestamp)?;
        if duration_ns < last {
            return Err(AviMjpegError::InvalidTimestamp);
        }
        let minimum_frames = self
            .pending
            .as_ref()
            .map(|pending| pending.slot.saturating_add(1))
            .ok_or(AviMjpegError::InvalidTimestamp)?;
        let desired_frames = self.frames_for_duration(duration_ns)?.max(minimum_frames);
        self.flush_pending_until(desired_frames)?;

        let movi_end = self.writer.stream_position()?;
        let index_bytes = self
            .index
            .len()
            .checked_mul(16)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(AviMjpegError::ContainerTooLarge)?;
        let projected_file_end = movi_end
            .checked_add(8)
            .and_then(|position| position.checked_add(u64::from(index_bytes)))
            .ok_or(AviMjpegError::ContainerTooLarge)?;
        if projected_file_end.saturating_sub(self.riff_start) > u64::from(u32::MAX) {
            return Err(AviMjpegError::ContainerTooLarge);
        }
        self.writer.write_all(b"idx1")?;
        write_u32(&mut self.writer, index_bytes)?;
        for entry in &self.index {
            self.writer.write_all(b"00dc")?;
            write_u32(&mut self.writer, AVIIF_KEYFRAME)?;
            write_u32(&mut self.writer, entry.offset)?;
            write_u32(&mut self.writer, entry.size)?;
        }
        let file_end = self.writer.stream_position()?;
        let frame_count = u32::try_from(self.index.len()).map_err(|_| AviMjpegError::FrameLimit)?;
        let movi_size = checked_u32(
            movi_end
                .checked_sub(self.movi_size_offset + 4)
                .ok_or(AviMjpegError::ContainerTooLarge)?,
        )?;
        let riff_size = checked_u32(
            file_end
                .checked_sub(self.riff_start + 8)
                .ok_or(AviMjpegError::ContainerTooLarge)?,
        )?;
        let micros_per_frame = ((1_000_000_u128 * u128::from(self.fps_denominator)
            + u128::from(self.fps_numerator) / 2)
            / u128::from(self.fps_numerator))
        .try_into()
        .map_err(|_| AviMjpegError::InvalidConfiguration)?;
        let frames_per_second =
            u64::from(self.fps_numerator).div_ceil(u64::from(self.fps_denominator));
        let maximum_bytes_per_second = u64::from(self.maximum_jpeg_bytes)
            .saturating_mul(frames_per_second)
            .min(u64::from(u32::MAX)) as u32;

        patch_u32(&mut self.writer, self.riff_size_offset, riff_size)?;
        patch_u32(&mut self.writer, self.movi_size_offset, movi_size)?;
        patch_u32(
            &mut self.writer,
            self.header_offsets.microseconds_per_frame,
            micros_per_frame,
        )?;
        patch_u32(
            &mut self.writer,
            self.header_offsets.maximum_bytes_per_second,
            maximum_bytes_per_second,
        )?;
        patch_u32(
            &mut self.writer,
            self.header_offsets.total_frames,
            frame_count,
        )?;
        patch_u32(
            &mut self.writer,
            self.header_offsets.avih_suggested_buffer,
            self.maximum_jpeg_bytes,
        )?;
        patch_u32(
            &mut self.writer,
            self.header_offsets.stream_length,
            frame_count,
        )?;
        patch_u32(
            &mut self.writer,
            self.header_offsets.stream_suggested_buffer,
            self.maximum_jpeg_bytes,
        )?;
        patch_u32(
            &mut self.writer,
            self.header_offsets.bitmap_size_image,
            self.maximum_jpeg_bytes,
        )?;
        self.writer.seek(SeekFrom::Start(file_end))?;
        Ok(AviMjpegOutput {
            writer: self.writer,
            frame_count: u64::from(frame_count),
        })
    }

    fn encode_jpeg(&self, rgba: &[u8]) -> Result<Vec<u8>, AviMjpegError> {
        let image = ImageBuffer::<Rgba<u8>, &[u8]>::from_raw(self.width, self.height, rgba)
            .ok_or(AviMjpegError::InvalidFrame)?;
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, self.quality).encode_image(&image)?;
        Ok(jpeg)
    }

    fn slot_for(&self, presentation_at_ns: u64) -> Result<u64, AviMjpegError> {
        let denominator = NANOS_PER_SECOND
            .checked_mul(u128::from(self.fps_denominator))
            .ok_or(AviMjpegError::InvalidConfiguration)?;
        let slot = u128::from(presentation_at_ns)
            .checked_mul(u128::from(self.fps_numerator))
            .ok_or(AviMjpegError::FrameLimit)?
            / denominator;
        u64::try_from(slot).map_err(|_| AviMjpegError::FrameLimit)
    }

    fn frames_for_duration(&self, duration_ns: u64) -> Result<u64, AviMjpegError> {
        let denominator = NANOS_PER_SECOND
            .checked_mul(u128::from(self.fps_denominator))
            .ok_or(AviMjpegError::InvalidConfiguration)?;
        let numerator = u128::from(duration_ns)
            .checked_mul(u128::from(self.fps_numerator))
            .ok_or(AviMjpegError::FrameLimit)?;
        let frames = numerator.div_ceil(denominator);
        u64::try_from(frames).map_err(|_| AviMjpegError::FrameLimit)
    }

    fn flush_pending_until(&mut self, exclusive_slot: u64) -> Result<(), AviMjpegError> {
        let pending = self
            .pending
            .as_ref()
            .ok_or(AviMjpegError::InvalidTimestamp)?;
        let repeat = exclusive_slot
            .checked_sub(pending.slot)
            .ok_or(AviMjpegError::InvalidTimestamp)?;
        let future_count = (self.index.len() as u64)
            .checked_add(repeat)
            .ok_or(AviMjpegError::FrameLimit)?;
        if repeat == 0 || future_count > MAX_AVI_FRAMES {
            return Err(if repeat == 0 {
                AviMjpegError::InvalidTimestamp
            } else {
                AviMjpegError::FrameLimit
            });
        }
        let pending = self.pending.take().expect("预算验证前已确认存在待写帧");
        for _ in 0..repeat {
            self.write_jpeg_chunk(&pending.jpeg)?;
        }
        Ok(())
    }

    fn write_jpeg_chunk(&mut self, jpeg: &[u8]) -> Result<(), AviMjpegError> {
        let size = u32::try_from(jpeg.len()).map_err(|_| AviMjpegError::ContainerTooLarge)?;
        let chunk_start = self.writer.stream_position()?;
        let offset = checked_u32(
            chunk_start
                .checked_sub(self.movi_fourcc_offset)
                .ok_or(AviMjpegError::ContainerTooLarge)?,
        )?;
        let padding = u64::from(!jpeg.len().is_multiple_of(2));
        let projected_end = chunk_start
            .checked_add(8)
            .and_then(|position| position.checked_add(jpeg.len() as u64))
            .and_then(|position| position.checked_add(padding))
            .ok_or(AviMjpegError::ContainerTooLarge)?;
        if projected_end.saturating_sub(self.riff_start) > u64::from(u32::MAX) {
            return Err(AviMjpegError::ContainerTooLarge);
        }
        self.writer.write_all(b"00dc")?;
        write_u32(&mut self.writer, size)?;
        self.writer.write_all(jpeg)?;
        if !jpeg.len().is_multiple_of(2) {
            self.writer.write_all(&[0])?;
        }
        self.maximum_jpeg_bytes = self.maximum_jpeg_bytes.max(size);
        self.index.push(IndexEntry { offset, size });
        Ok(())
    }
}

fn build_header(
    width: u32,
    height: u32,
    fps_numerator: u32,
    fps_denominator: u32,
) -> (Vec<u8>, HeaderOffsets) {
    let mut bytes = Vec::with_capacity(224);
    bytes.extend_from_slice(b"LIST");
    let hdrl_size = reserve_u32(&mut bytes);
    bytes.extend_from_slice(b"hdrl");

    bytes.extend_from_slice(b"avih");
    push_u32(&mut bytes, 56);
    let microseconds_per_frame = reserve_u32(&mut bytes) as u64;
    let maximum_bytes_per_second = reserve_u32(&mut bytes) as u64;
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, AVIF_HAS_INDEX);
    let total_frames = reserve_u32(&mut bytes) as u64;
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 1);
    let avih_suggested_buffer = reserve_u32(&mut bytes) as u64;
    push_u32(&mut bytes, width);
    push_u32(&mut bytes, height);
    for _ in 0..4 {
        push_u32(&mut bytes, 0);
    }

    bytes.extend_from_slice(b"LIST");
    let strl_size = reserve_u32(&mut bytes);
    bytes.extend_from_slice(b"strl");
    bytes.extend_from_slice(b"strh");
    push_u32(&mut bytes, 56);
    bytes.extend_from_slice(b"vids");
    bytes.extend_from_slice(b"MJPG");
    push_u32(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u16(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, fps_denominator);
    push_u32(&mut bytes, fps_numerator);
    push_u32(&mut bytes, 0);
    let stream_length = reserve_u32(&mut bytes) as u64;
    let stream_suggested_buffer = reserve_u32(&mut bytes) as u64;
    push_u32(&mut bytes, u32::MAX);
    push_u32(&mut bytes, 0);
    push_i16(&mut bytes, 0);
    push_i16(&mut bytes, 0);
    push_i16(&mut bytes, width as i16);
    push_i16(&mut bytes, height as i16);

    bytes.extend_from_slice(b"strf");
    push_u32(&mut bytes, 40);
    push_u32(&mut bytes, 40);
    push_i32(&mut bytes, width as i32);
    push_i32(&mut bytes, height as i32);
    push_u16(&mut bytes, 1);
    push_u16(&mut bytes, 24);
    bytes.extend_from_slice(b"MJPG");
    let bitmap_size_image = reserve_u32(&mut bytes) as u64;
    push_i32(&mut bytes, 0);
    push_i32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);

    let header_length = bytes.len();
    patch_vec_u32(&mut bytes, strl_size, header_length - (strl_size + 4));
    patch_vec_u32(&mut bytes, hdrl_size, header_length - (hdrl_size + 4));
    (
        bytes,
        HeaderOffsets {
            microseconds_per_frame,
            maximum_bytes_per_second,
            total_frames,
            avih_suggested_buffer,
            stream_length,
            stream_suggested_buffer,
            bitmap_size_image,
        },
    )
}

fn checked_u32(value: u64) -> Result<u32, AviMjpegError> {
    u32::try_from(value).map_err(|_| AviMjpegError::ContainerTooLarge)
}

fn write_u32(writer: &mut impl Write, value: u32) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn patch_u32(writer: &mut (impl Write + Seek), offset: u64, value: u32) -> io::Result<()> {
    writer.seek(SeekFrom::Start(offset))?;
    write_u32(writer, value)
}

fn reserve_u32(bytes: &mut Vec<u8>) -> usize {
    let offset = bytes.len();
    push_u32(bytes, 0);
    offset
}

fn patch_vec_u32(bytes: &mut [u8], offset: usize, value: usize) {
    bytes[offset..offset + 4].copy_from_slice(&(value as u32).to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_i16(bytes: &mut Vec<u8>, value: i16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::process::Command;

    fn rgba(width: u32, height: u32, phase: u8) -> Vec<u8> {
        let mut bytes = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                bytes.extend_from_slice(&[
                    (x as u8).wrapping_add(phase),
                    (y as u8).wrapping_add(phase),
                    phase,
                    255,
                ]);
            }
        }
        bytes
    }

    fn encoded_fixture() -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = AviMjpegWriter::new(cursor, 64, 48, 10, 1, 90).unwrap();
        writer.push_rgba(&rgba(64, 48, 0), 0).unwrap();
        writer.push_rgba(&rgba(64, 48, 20), 100_000_000).unwrap();
        writer.finish(200_000_000).unwrap().into_inner()
    }

    #[test]
    fn writes_seekable_indexed_avi_with_two_mjpeg_frames() {
        let bytes = encoded_fixture();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"AVI ");
        assert!(bytes.windows(4).any(|window| window == b"MJPG"));
        let index = bytes
            .windows(4)
            .position(|window| window == b"idx1")
            .expect("缺少 idx1");
        assert_eq!(
            u32::from_le_bytes(bytes[index + 4..index + 8].try_into().unwrap()),
            32
        );
        assert_eq!(&bytes[index + 8..index + 12], b"00dc");
        assert_eq!(&bytes[index + 24..index + 28], b"00dc");
        assert_eq!(
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize,
            bytes.len() - 8
        );
    }

    #[test]
    fn timeline_gaps_duplicate_the_previous_frame_at_fixed_rate() {
        let cursor = Cursor::new(Vec::new());
        let mut writer = AviMjpegWriter::new(cursor, 8, 8, 10, 1, 80).unwrap();
        writer.push_rgba(&rgba(8, 8, 0), 0).unwrap();
        writer.push_rgba(&rgba(8, 8, 1), 350_000_000).unwrap();
        let bytes = writer.finish(450_000_000).unwrap().into_inner();
        let index = bytes
            .windows(4)
            .position(|window| window == b"idx1")
            .unwrap();
        assert_eq!(
            u32::from_le_bytes(bytes[index + 4..index + 8].try_into().unwrap()),
            80
        );
    }

    #[test]
    fn same_output_slot_keeps_only_the_latest_captured_frame() {
        let cursor = Cursor::new(Vec::new());
        let mut writer = AviMjpegWriter::new(cursor, 8, 8, 10, 1, 80).unwrap();
        writer.push_rgba(&rgba(8, 8, 0), 0).unwrap();
        writer.push_rgba(&rgba(8, 8, 1), 20_000_000).unwrap();
        writer.push_rgba(&rgba(8, 8, 2), 90_000_000).unwrap();
        let bytes = writer.finish(100_000_000).unwrap().into_inner();
        let index = bytes
            .windows(4)
            .position(|window| window == b"idx1")
            .unwrap();
        assert_eq!(
            u32::from_le_bytes(bytes[index + 4..index + 8].try_into().unwrap()),
            16
        );
    }

    #[test]
    fn rejects_bad_frame_and_timestamp_without_consuming_the_writer() {
        let cursor = Cursor::new(Vec::new());
        let mut writer = AviMjpegWriter::new(cursor, 8, 8, 10, 1, 80).unwrap();
        assert!(matches!(
            writer.push_rgba(&[0; 4], 0),
            Err(AviMjpegError::InvalidFrame)
        ));
        writer.push_rgba(&rgba(8, 8, 0), 0).unwrap();
        assert!(matches!(
            writer.push_rgba(&rgba(8, 8, 1), 0),
            Err(AviMjpegError::InvalidTimestamp)
        ));
        writer.push_rgba(&rgba(8, 8, 2), 100_000_000).unwrap();
        assert!(!writer.finish(200_000_000).unwrap().into_inner().is_empty());
    }

    #[test]
    fn frame_budget_failure_keeps_the_pending_frame_for_a_valid_retry() {
        let cursor = Cursor::new(Vec::new());
        let mut writer = AviMjpegWriter::new(cursor, 8, 8, 10, 1, 80).unwrap();
        writer.push_rgba(&rgba(8, 8, 0), 0).unwrap();
        assert!(matches!(
            writer.push_rgba(&rgba(8, 8, 1), 1_800_100_000_000),
            Err(AviMjpegError::FrameLimit)
        ));
        writer.push_rgba(&rgba(8, 8, 2), 100_000_000).unwrap();
        assert!(!writer.finish(200_000_000).unwrap().into_inner().is_empty());
    }

    #[test]
    fn writer_requires_a_new_empty_container() {
        let mut cursor = Cursor::new(vec![0]);
        cursor.set_position(1);
        assert!(matches!(
            AviMjpegWriter::new(cursor, 8, 8, 10, 1, 80),
            Err(AviMjpegError::InvalidConfiguration)
        ));
    }

    #[test]
    fn ffprobe_recognizes_codec_geometry_rate_count_and_duration_when_available() {
        if Command::new("ffprobe").arg("-version").output().is_err() {
            return;
        }
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("fixture.avi");
        std::fs::write(&path, encoded_fixture()).unwrap();
        let output = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,width,height,avg_frame_rate,nb_frames,duration",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "ffprobe stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let stream = &value["streams"][0];
        assert_eq!(stream["codec_name"], "mjpeg");
        assert_eq!(stream["width"], 64);
        assert_eq!(stream["height"], 48);
        assert_eq!(stream["avg_frame_rate"], "10/1");
        assert_eq!(stream["nb_frames"], "2");
        assert_eq!(stream["duration"], "0.200000");
    }
}
