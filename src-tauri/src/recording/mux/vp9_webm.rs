//! PX-REC-01 的隔离 VP9/WebM 编码原型。
//!
//! 该模块只在 `recording-vp9-prototype` feature 下编译。它先固定 RGBA → I420、固定帧率补帧、
//! VP9 screen-content 参数和可探测 WebM 时长；在 canary libvpx 绑定与三平台供应链通过前不接产品入口。

use super::super::frame::MAX_FRAME_BYTES;
use shiguredo_libvpx::{
    CodecConfig, ContentType, EncodeOptions, Encoder, EncoderConfig, EncodingDeadline, ImageData,
    ImageFormat, RateControlMode, Vp9Config,
};
use std::io::{Seek, Write};
use std::num::NonZeroUsize;
use thiserror::Error;
use webm::mux::{Segment, SegmentBuilder, SegmentMode, VideoCodecId, VideoTrack, Writer};

const NANOS_PER_SECOND: u128 = 1_000_000_000;
const WEBM_TIMECODE_SCALE_NS: u64 = 1_000_000;
const MAX_RECORDING_FRAMES: u64 = 24 * 60 * 60 * 120;
const VP9_CQ_LEVEL: usize = 30;
const VP9_CPU_USED: usize = 6;

#[derive(Debug, Error)]
pub(in crate::recording) enum Vp9WebmError {
    #[error("VP9/WebM 配置无效")]
    InvalidConfiguration,
    #[error("VP9 输入 RGBA 字节数与尺寸不一致")]
    InvalidFrame,
    #[error("VP9 输入时间戳必须严格递增并从零开始")]
    InvalidTimestamp,
    #[error("VP9/WebM 超过 24 小时、120 fps 帧预算")]
    FrameLimit,
    #[error("VP9 编码器没有为每个固定帧率输入产生一个输出帧")]
    UnexpectedPacketCount,
    #[error("VP9 编码失败: {0}")]
    Encode(#[from] shiguredo_libvpx::Error),
    #[error("WebM 封装失败: {0}")]
    Mux(#[from] webm::mux::Error),
    #[error("WebM 封尾失败")]
    Finalize,
}

struct I420Frame {
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

struct PendingFrame {
    slot: u64,
    image: I420Frame,
}

pub(in crate::recording) struct Vp9WebmOutput<W> {
    pub writer: W,
    pub frame_count: u64,
}

pub(in crate::recording) struct Vp9PacketEncoder {
    width: u32,
    height: u32,
    fps_numerator: u32,
    fps_denominator: u32,
    keyframe_interval: u64,
    encoder: Encoder,
    pending: Option<PendingFrame>,
    last_presentation_ns: Option<u64>,
    submitted_frames: u64,
    encoded_frames: u64,
    force_next_keyframe: bool,
}

impl Vp9PacketEncoder {
    pub fn new(
        width: u32,
        height: u32,
        fps_numerator: u32,
        fps_denominator: u32,
    ) -> Result<Self, Vp9WebmError> {
        let expected_bytes = expected_rgba_bytes(width, height)?;
        if width == 0
            || height == 0
            || !width.is_multiple_of(2)
            || !height.is_multiple_of(2)
            || expected_bytes > MAX_FRAME_BYTES
            || fps_numerator == 0
            || fps_denominator == 0
            || u64::from(fps_numerator) > 120 * u64::from(fps_denominator)
        {
            return Err(Vp9WebmError::InvalidConfiguration);
        }

        let pixels_per_second = u128::from(width)
            .checked_mul(u128::from(height))
            .and_then(|pixels| pixels.checked_mul(u128::from(fps_numerator)))
            .and_then(|value| value.checked_div(u128::from(fps_denominator)))
            .ok_or(Vp9WebmError::InvalidConfiguration)?;
        // 约 0.083 bit / pixel / frame，只作为原型的 CQ 码率上界；后续由 1080p/4K 语料校准。
        let target_bitrate = usize::try_from((pixels_per_second / 12).clamp(500_000, 50_000_000))
            .map_err(|_| Vp9WebmError::InvalidConfiguration)?;
        let keyframe_interval = u64::from(fps_numerator)
            .checked_mul(2)
            .and_then(|value| value.checked_div(u64::from(fps_denominator)))
            .filter(|value| *value > 0)
            .ok_or(Vp9WebmError::InvalidConfiguration)?;
        let threads = std::thread::available_parallelism()
            .ok()
            .map(|count| count.get().min(4))
            .and_then(NonZeroUsize::new);

        let vp9 = Vp9Config {
            row_mt: true,
            tune_content: Some(ContentType::Screen),
            ..Vp9Config::default()
        };
        let mut encoder_config = EncoderConfig::new(
            width as usize,
            height as usize,
            ImageFormat::I420,
            CodecConfig::Vp9(vp9),
        );
        encoder_config.fps_numerator = fps_numerator as usize;
        encoder_config.fps_denominator = fps_denominator as usize;
        encoder_config.target_bitrate = target_bitrate;
        encoder_config.cq_level = VP9_CQ_LEVEL;
        encoder_config.cpu_used = Some(VP9_CPU_USED);
        encoder_config.deadline = EncodingDeadline::Realtime;
        encoder_config.rate_control = RateControlMode::Cq;
        // 周期恢复分段必须在边界立即取得完整 packet，不能把帧滞留到整个编码器 finish。
        encoder_config.lag_in_frames = Some(0);
        encoder_config.threads = threads;
        encoder_config.error_resilient = true;
        encoder_config.keyframe_interval = NonZeroUsize::new(
            usize::try_from(keyframe_interval).map_err(|_| Vp9WebmError::InvalidConfiguration)?,
        );
        encoder_config.frame_drop_threshold = None;
        let encoder = Encoder::new(encoder_config)?;

        Ok(Self {
            width,
            height,
            fps_numerator,
            fps_denominator,
            keyframe_interval,
            encoder,
            pending: None,
            last_presentation_ns: None,
            submitted_frames: 0,
            encoded_frames: 0,
            force_next_keyframe: false,
        })
    }

    pub fn push_rgba<F>(
        &mut self,
        rgba: &[u8],
        presentation_at_ns: u64,
        emit: &mut F,
    ) -> Result<(), Vp9WebmError>
    where
        F: FnMut(&[u8], u64, bool) -> Result<(), Vp9WebmError>,
    {
        let expected_bytes = expected_rgba_bytes(self.width, self.height)?;
        if rgba.len() != expected_bytes
            || self
                .last_presentation_ns
                .is_some_and(|last| presentation_at_ns <= last)
            || (self.last_presentation_ns.is_none() && presentation_at_ns != 0)
        {
            return Err(if rgba.len() != expected_bytes {
                Vp9WebmError::InvalidFrame
            } else {
                Vp9WebmError::InvalidTimestamp
            });
        }

        let target_slot = self.slot_for(presentation_at_ns)?;
        if let Some(pending) = self.pending.as_ref() {
            if target_slot < pending.slot {
                return Err(Vp9WebmError::InvalidTimestamp);
            }
            if target_slot > pending.slot {
                self.flush_pending_until(target_slot, emit)?;
            }
        }
        self.pending = Some(PendingFrame {
            slot: target_slot,
            image: rgba_to_i420(rgba, self.width, self.height)?,
        });
        self.last_presentation_ns = Some(presentation_at_ns);
        Ok(())
    }

    pub fn flush_until<F>(
        &mut self,
        presentation_at_ns: u64,
        emit: &mut F,
    ) -> Result<(), Vp9WebmError>
    where
        F: FnMut(&[u8], u64, bool) -> Result<(), Vp9WebmError>,
    {
        let target_slot = self.slot_for(presentation_at_ns)?;
        let minimum_slot = self
            .pending
            .as_ref()
            .map(|pending| pending.slot.saturating_add(1))
            .ok_or(Vp9WebmError::InvalidTimestamp)?;
        let exclusive_slot = target_slot.max(minimum_slot);
        self.flush_pending_until(exclusive_slot, emit)
    }

    pub fn force_next_keyframe(&mut self) {
        self.force_next_keyframe = true;
    }

    pub fn next_timestamp_ns(&self) -> Result<u64, Vp9WebmError> {
        self.timestamp_for_frame(self.encoded_frames)
    }

    pub fn finish<F>(&mut self, duration_ns: u64, emit: &mut F) -> Result<u64, Vp9WebmError>
    where
        F: FnMut(&[u8], u64, bool) -> Result<(), Vp9WebmError>,
    {
        let last = self
            .last_presentation_ns
            .ok_or(Vp9WebmError::InvalidTimestamp)?;
        if duration_ns < last {
            return Err(Vp9WebmError::InvalidTimestamp);
        }
        let minimum_frames = self
            .pending
            .as_ref()
            .map(|pending| pending.slot.saturating_add(1))
            .ok_or(Vp9WebmError::InvalidTimestamp)?;
        let desired_frames = self.frames_for_duration(duration_ns)?.max(minimum_frames);
        self.flush_pending_until(desired_frames, emit)?;

        self.encoder.finish()?;
        self.drain_encoded_packets(emit)?;
        if self.submitted_frames != desired_frames || self.encoded_frames != desired_frames {
            return Err(Vp9WebmError::UnexpectedPacketCount);
        }
        Ok(self.encoded_frames)
    }

    fn flush_pending_until<F>(
        &mut self,
        exclusive_slot: u64,
        emit: &mut F,
    ) -> Result<(), Vp9WebmError>
    where
        F: FnMut(&[u8], u64, bool) -> Result<(), Vp9WebmError>,
    {
        let pending = self.pending.take().ok_or(Vp9WebmError::InvalidTimestamp)?;
        let repeat = exclusive_slot
            .checked_sub(pending.slot)
            .ok_or(Vp9WebmError::InvalidTimestamp)?;
        let future_count = self
            .submitted_frames
            .checked_add(repeat)
            .ok_or(Vp9WebmError::FrameLimit)?;
        if repeat == 0 || future_count > MAX_RECORDING_FRAMES {
            self.pending = Some(pending);
            return Err(if repeat == 0 {
                Vp9WebmError::InvalidTimestamp
            } else {
                Vp9WebmError::FrameLimit
            });
        }
        for _ in 0..repeat {
            self.encode_i420(&pending.image, emit)?;
        }
        Ok(())
    }

    fn encode_i420<F>(&mut self, image: &I420Frame, emit: &mut F) -> Result<(), Vp9WebmError>
    where
        F: FnMut(&[u8], u64, bool) -> Result<(), Vp9WebmError>,
    {
        let force_keyframe = self.force_next_keyframe
            || self.submitted_frames.is_multiple_of(self.keyframe_interval);
        self.encoder.encode(
            &ImageData::I420 {
                y: &image.y,
                u: &image.u,
                v: &image.v,
            },
            &EncodeOptions { force_keyframe },
        )?;
        self.force_next_keyframe = false;
        self.submitted_frames = self.submitted_frames.saturating_add(1);
        self.drain_encoded_packets(emit)
    }

    fn drain_encoded_packets<F>(&mut self, emit: &mut F) -> Result<(), Vp9WebmError>
    where
        F: FnMut(&[u8], u64, bool) -> Result<(), Vp9WebmError>,
    {
        let fps_numerator = self.fps_numerator;
        let fps_denominator = self.fps_denominator;
        while let Some(frame) = self.encoder.next_frame() {
            let timestamp_ns =
                frame_timestamp_ns(self.encoded_frames, fps_numerator, fps_denominator)?;
            emit(frame.data(), timestamp_ns, frame.is_keyframe())?;
            self.encoded_frames = self.encoded_frames.saturating_add(1);
        }
        Ok(())
    }

    fn slot_for(&self, presentation_at_ns: u64) -> Result<u64, Vp9WebmError> {
        let denominator = NANOS_PER_SECOND
            .checked_mul(u128::from(self.fps_denominator))
            .ok_or(Vp9WebmError::InvalidConfiguration)?;
        let slot = u128::from(presentation_at_ns)
            .checked_mul(u128::from(self.fps_numerator))
            .ok_or(Vp9WebmError::FrameLimit)?
            / denominator;
        u64::try_from(slot).map_err(|_| Vp9WebmError::FrameLimit)
    }

    fn frames_for_duration(&self, duration_ns: u64) -> Result<u64, Vp9WebmError> {
        let denominator = NANOS_PER_SECOND
            .checked_mul(u128::from(self.fps_denominator))
            .ok_or(Vp9WebmError::InvalidConfiguration)?;
        let numerator = u128::from(duration_ns)
            .checked_mul(u128::from(self.fps_numerator))
            .ok_or(Vp9WebmError::FrameLimit)?;
        u64::try_from(numerator.div_ceil(denominator)).map_err(|_| Vp9WebmError::FrameLimit)
    }

    fn timestamp_for_frame(&self, frame: u64) -> Result<u64, Vp9WebmError> {
        frame_timestamp_ns(frame, self.fps_numerator, self.fps_denominator)
    }
}

fn frame_timestamp_ns(
    frame: u64,
    fps_numerator: u32,
    fps_denominator: u32,
) -> Result<u64, Vp9WebmError> {
    u128::from(frame)
        .checked_mul(NANOS_PER_SECOND)
        .and_then(|value| value.checked_mul(u128::from(fps_denominator)))
        .and_then(|value| value.checked_div(u128::from(fps_numerator)))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(Vp9WebmError::FrameLimit)
}

pub(in crate::recording) struct WebmPacketMux<W: Write + Seek> {
    segment: Option<Segment<W>>,
    track: VideoTrack,
    frame_count: u64,
}

impl<W: Write + Seek> WebmPacketMux<W> {
    pub fn new(writer: W, width: u32, height: u32) -> Result<Self, Vp9WebmError> {
        let builder = SegmentBuilder::new(Writer::new(writer))?
            .set_writing_app("Clippy")?
            .set_mode(SegmentMode::File)?;
        let (builder, track) =
            builder.add_video_track(width, height, VideoCodecId::VP9, Some(1))?;
        Ok(Self {
            segment: Some(builder.build()),
            track,
            frame_count: 0,
        })
    }

    pub fn add_frame(
        &mut self,
        data: &[u8],
        timestamp_ns: u64,
        is_keyframe: bool,
    ) -> Result<(), Vp9WebmError> {
        self.segment
            .as_mut()
            .ok_or(Vp9WebmError::Finalize)?
            .add_frame(self.track, data, timestamp_ns, is_keyframe)?;
        self.frame_count = self.frame_count.saturating_add(1);
        Ok(())
    }

    pub fn finish(mut self, duration_ns: u64) -> Result<Vp9WebmOutput<W>, Vp9WebmError> {
        // libwebm 的 Segment::set_duration 接收 TimecodeScale tick；其默认 scale 为 1 ms。
        let duration_ticks = duration_ns.div_ceil(WEBM_TIMECODE_SCALE_NS).max(1);
        let writer = self
            .segment
            .take()
            .ok_or(Vp9WebmError::Finalize)?
            .finalize(Some(duration_ticks))
            .map_err(|_| Vp9WebmError::Finalize)?
            .into_inner();
        Ok(Vp9WebmOutput {
            writer,
            frame_count: self.frame_count,
        })
    }
}

pub(in crate::recording) struct Vp9WebmWriter<W: Write + Seek> {
    encoder: Vp9PacketEncoder,
    mux: WebmPacketMux<W>,
}

impl<W: Write + Seek> Vp9WebmWriter<W> {
    pub fn new(
        writer: W,
        width: u32,
        height: u32,
        fps_numerator: u32,
        fps_denominator: u32,
    ) -> Result<Self, Vp9WebmError> {
        Ok(Self {
            encoder: Vp9PacketEncoder::new(width, height, fps_numerator, fps_denominator)?,
            mux: WebmPacketMux::new(writer, width, height)?,
        })
    }

    pub fn push_rgba(&mut self, rgba: &[u8], presentation_at_ns: u64) -> Result<(), Vp9WebmError> {
        let mux = &mut self.mux;
        self.encoder.push_rgba(
            rgba,
            presentation_at_ns,
            &mut |data, timestamp_ns, is_keyframe| mux.add_frame(data, timestamp_ns, is_keyframe),
        )
    }

    pub fn finish_with_stats(mut self, duration_ns: u64) -> Result<Vp9WebmOutput<W>, Vp9WebmError> {
        let mux = &mut self.mux;
        let encoded_frames = self
            .encoder
            .finish(duration_ns, &mut |data, timestamp_ns, is_keyframe| {
                mux.add_frame(data, timestamp_ns, is_keyframe)
            })?;
        let output = self.mux.finish(duration_ns)?;
        if output.frame_count != encoded_frames {
            return Err(Vp9WebmError::UnexpectedPacketCount);
        }
        Ok(output)
    }
}

fn expected_rgba_bytes(width: u32, height: u32) -> Result<usize, Vp9WebmError> {
    u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or(Vp9WebmError::InvalidConfiguration)
}

fn rgba_to_i420(rgba: &[u8], width: u32, height: u32) -> Result<I420Frame, Vp9WebmError> {
    if !width.is_multiple_of(2)
        || !height.is_multiple_of(2)
        || rgba.len() != expected_rgba_bytes(width, height)?
    {
        return Err(Vp9WebmError::InvalidFrame);
    }
    let width = width as usize;
    let height = height as usize;
    let mut y = vec![0; width * height];
    let mut u = vec![0; width * height / 4];
    let mut v = vec![0; width * height / 4];

    for row in 0..height {
        for column in 0..width {
            let offset = (row * width + column) * 4;
            y[row * width + column] = limited_y(rgba[offset], rgba[offset + 1], rgba[offset + 2]);
        }
    }
    for row in (0..height).step_by(2) {
        for column in (0..width).step_by(2) {
            let mut red = 0_u16;
            let mut green = 0_u16;
            let mut blue = 0_u16;
            for dy in 0..2 {
                for dx in 0..2 {
                    let offset = ((row + dy) * width + column + dx) * 4;
                    red += u16::from(rgba[offset]);
                    green += u16::from(rgba[offset + 1]);
                    blue += u16::from(rgba[offset + 2]);
                }
            }
            let red = ((red + 2) / 4) as u8;
            let green = ((green + 2) / 4) as u8;
            let blue = ((blue + 2) / 4) as u8;
            let chroma = (row / 2) * (width / 2) + column / 2;
            u[chroma] = limited_u(red, green, blue);
            v[chroma] = limited_v(red, green, blue);
        }
    }
    Ok(I420Frame { y, u, v })
}

// BT.709 limited-range 8-bit conversion. 桌面帧源保证画面不透明，因此 alpha 不参与转换。
fn limited_y(red: u8, green: u8, blue: u8) -> u8 {
    clamp_u8(16 + (47 * i32::from(red) + 157 * i32::from(green) + 16 * i32::from(blue) + 128) / 256)
}

fn limited_u(red: u8, green: u8, blue: u8) -> u8 {
    clamp_u8(
        128 + (-26 * i32::from(red) - 87 * i32::from(green) + 112 * i32::from(blue) + 128)
            .div_euclid(256),
    )
}

fn limited_v(red: u8, green: u8, blue: u8) -> u8 {
    clamp_u8(
        128 + (112 * i32::from(red) - 102 * i32::from(green) - 10 * i32::from(blue) + 128)
            .div_euclid(256),
    )
}

fn clamp_u8(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::process::Command;

    fn solid_rgba(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
        (0..width * height)
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
            .collect()
    }

    fn encoded_fixture() -> Vp9WebmOutput<Cursor<Vec<u8>>> {
        let mut writer = Vp9WebmWriter::new(Cursor::new(Vec::new()), 64, 48, 10, 1).unwrap();
        writer
            .push_rgba(&solid_rgba(64, 48, [16, 32, 64]), 0)
            .unwrap();
        writer
            .push_rgba(&solid_rgba(64, 48, [240, 220, 200]), 100_000_000)
            .unwrap();
        writer.finish_with_stats(200_000_000).unwrap()
    }

    #[test]
    fn converts_rgba_to_bt709_limited_i420() {
        let rgba = [
            0, 0, 0, 255, 255, 255, 255, 255, 255, 0, 0, 255, 0, 255, 0, 255,
        ];
        let image = rgba_to_i420(&rgba, 2, 2).unwrap();
        assert_eq!(image.y, [16, 235, 63, 172]);
        assert_eq!(image.u.len(), 1);
        assert_eq!(image.v.len(), 1);
    }

    #[test]
    fn writes_vp9_webm_and_preserves_fixed_duration() {
        let output = encoded_fixture();
        let bytes = output.writer.into_inner();
        assert_eq!(output.frame_count, 2);
        assert_eq!(&bytes[0..4], [0x1a, 0x45, 0xdf, 0xa3]);
        assert!(bytes.windows(4).any(|window| window == b"webm"));
        assert!(bytes.windows(5).any(|window| window == b"V_VP9"));
    }

    #[test]
    fn continuous_encoder_can_finalize_independent_packet_segment() {
        let mut encoder = Vp9PacketEncoder::new(64, 48, 10, 1).unwrap();
        let mut final_mux = WebmPacketMux::new(Cursor::new(Vec::new()), 64, 48).unwrap();
        let mut segment_mux = WebmPacketMux::new(Cursor::new(Vec::new()), 64, 48).unwrap();
        encoder
            .push_rgba(
                &solid_rgba(64, 48, [16, 32, 64]),
                0,
                &mut |data, timestamp, keyframe| {
                    final_mux.add_frame(data, timestamp, keyframe)?;
                    segment_mux.add_frame(data, timestamp, keyframe)
                },
            )
            .unwrap();
        encoder
            .flush_until(200_000_000, &mut |data, timestamp, keyframe| {
                final_mux.add_frame(data, timestamp, keyframe)?;
                segment_mux.add_frame(data, timestamp, keyframe)
            })
            .unwrap();
        let segment = segment_mux.finish(200_000_000).unwrap();
        assert_eq!(segment.frame_count, 2);
        assert_eq!(&segment.writer.into_inner()[0..4], [0x1a, 0x45, 0xdf, 0xa3]);
    }

    #[test]
    fn rejects_odd_geometry_bad_frame_and_non_monotonic_time() {
        assert!(matches!(
            Vp9WebmWriter::new(Cursor::new(Vec::new()), 63, 48, 10, 1),
            Err(Vp9WebmError::InvalidConfiguration)
        ));
        let mut writer = Vp9WebmWriter::new(Cursor::new(Vec::new()), 64, 48, 10, 1).unwrap();
        assert!(matches!(
            writer.push_rgba(&[0; 4], 0),
            Err(Vp9WebmError::InvalidFrame)
        ));
        let frame = solid_rgba(64, 48, [0, 0, 0]);
        writer.push_rgba(&frame, 0).unwrap();
        assert!(matches!(
            writer.push_rgba(&frame, 0),
            Err(Vp9WebmError::InvalidTimestamp)
        ));
    }

    #[test]
    fn ffprobe_reads_codec_frame_count_and_duration_when_available() {
        if Command::new("ffprobe").arg("-version").output().is_err() {
            return;
        }
        let output = encoded_fixture();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fixture.webm");
        std::fs::write(&path, output.writer.into_inner()).unwrap();
        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-count_frames",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,nb_read_frames:format=duration",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .unwrap();
        assert!(
            probe.status.success(),
            "{}",
            String::from_utf8_lossy(&probe.stderr)
        );
        let payload: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        assert_eq!(payload["streams"][0]["codec_name"], "vp9");
        assert_eq!(payload["streams"][0]["nb_read_frames"], "2");
        assert_eq!(payload["format"]["duration"], "0.200000");
    }
}
