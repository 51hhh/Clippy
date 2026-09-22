//! PX-REC-OPUS-WEBM-01 的 Opus 编码与 VP9/Opus WebM 封装合同。
//!
//! 本模块只在显式原型 feature 下编译。平台音源与产品 session 尚未接线；调用方必须先用共同
//! A/V epoch 对齐 PCM，再把两轨 packet 按全局 timestamp 顺序交给 mux。

use super::super::audio::{frames_to_ns, QueuedAudioChunk, AUDIO_SAMPLE_RATE_HZ};
use opusic_c::{Application, Bitrate, Channels, Encoder, SampleRate};
use std::io::{Seek, Write};
use thiserror::Error;
use webm::mux::{
    AudioCodecId, AudioTrack, Segment, SegmentBuilder, SegmentMode, VideoCodecId, VideoTrack,
    Writer,
};

const OPUS_FRAME_FRAMES: usize = 960;
const OPUS_FRAME_DURATION_NS: u64 = 20_000_000;
const MAX_OPUS_PACKET_BYTES: usize = 1_276;
const OPUS_HEAD_BYTES: usize = 19;
const OPUS_SEEK_PRE_ROLL_NS: u64 = 80_000_000;
const NANOS_PER_SECOND: u128 = 1_000_000_000;
const SAMPLE_PERIOD_CEIL_NS: u64 = 20_834;
const MONO_BITRATE: u32 = 96_000;
const STEREO_BITRATE: u32 = 160_000;
// 0.5 ms 可以精确表示固定 libopus 1.6.1 的 312-frame / 6.5 ms lookahead，同时保留
// 约 16 秒的有符号 Block timecode 范围，避免为了纳秒精度频繁切 Cluster。
const WEBM_TIMECODE_SCALE_NS: u64 = 500_000;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(in crate::recording) enum OpusWebmError {
    #[error("Opus 只接受 48 kHz 单声道或双声道 PCM")]
    InvalidFormat,
    #[error("Opus PCM 块内容与元数据不一致")]
    InvalidChunk,
    #[error("Opus PCM 时间线倒退或重叠")]
    InputOverlap,
    #[error("Opus PCM 连续输入中存在未处理空洞")]
    InputGap,
    #[error("Opus 编码器尚未收到 PCM")]
    NoAudio,
    #[error("Opus 编码器已经结束")]
    Finished,
    #[error("Opus 时间线或样本数量溢出")]
    TimelineOverflow,
    #[error("Opus 编码失败: {0}")]
    Encode(String),
    #[error("WebM 配置无效")]
    InvalidMuxConfiguration,
    #[error("WebM packet 时间戳跨轨倒退或同轨重复")]
    InvalidMuxTimestamp,
    #[error("双轨 packet 重排队列超过固定上限")]
    InterleaveQueueFull,
    #[error("WebM 封装失败: {0}")]
    Mux(String),
    #[error("WebM 封尾失败")]
    Finalize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::recording) struct OpusTrackConfig {
    pub channels: u16,
    pub pre_skip_frames: u16,
    pub codec_delay_ns: u64,
    pub seek_pre_roll_ns: u64,
    pub opus_head: [u8; OPUS_HEAD_BYTES],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::recording) struct EncodedOpusPacket {
    pub data: Box<[u8]>,
    pub timestamp_ns: u64,
    pub duration_ns: u64,
    pub discard_padding_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::recording) struct OpusEncoderStats {
    pub real_frames: u64,
    pub encoded_frames: u64,
    pub packet_count: u64,
    pub pre_skip_frames: u16,
    pub end_padding_frames: u32,
    pub media_start_ns: u64,
    pub media_duration_ns: u64,
}

pub(in crate::recording) struct OpusFinish {
    pub packets: Vec<EncodedOpusPacket>,
    pub stats: OpusEncoderStats,
}

pub(in crate::recording) struct OpusPacketEncoder {
    encoder: Encoder,
    channels: u16,
    track: OpusTrackConfig,
    pending: Vec<f32>,
    media_start_ns: Option<u64>,
    real_frames: u64,
    encoded_frames: u64,
    packet_count: u64,
    finished: bool,
}

impl OpusPacketEncoder {
    pub fn new(channels: u16) -> Result<Self, OpusWebmError> {
        let opus_channels = match channels {
            1 => Channels::Mono,
            2 => Channels::Stereo,
            _ => return Err(OpusWebmError::InvalidFormat),
        };
        let mut encoder = Encoder::new(opus_channels, SampleRate::Hz48000, Application::Audio)
            .map_err(opus_error)?;
        encoder
            .set_bitrate(Bitrate::Value(if channels == 1 {
                MONO_BITRATE
            } else {
                STEREO_BITRATE
            }))
            .map_err(opus_error)?;
        encoder.set_vbr(true).map_err(opus_error)?;
        encoder.set_vbr_constraint(true).map_err(opus_error)?;
        encoder.set_complexity(10).map_err(opus_error)?;
        encoder.set_lsb_depth(24).map_err(opus_error)?;

        let lookahead = encoder.get_look_ahead().map_err(opus_error)?;
        let pre_skip_frames =
            u16::try_from(lookahead).map_err(|_| OpusWebmError::InvalidMuxConfiguration)?;
        if pre_skip_frames == 0 {
            return Err(OpusWebmError::InvalidMuxConfiguration);
        }
        let codec_delay_ns = frames_to_ns_u64(u64::from(pre_skip_frames))?;
        let opus_head = build_opus_head(channels, pre_skip_frames)?;

        Ok(Self {
            encoder,
            channels,
            track: OpusTrackConfig {
                channels,
                pre_skip_frames,
                codec_delay_ns,
                seek_pre_roll_ns: OPUS_SEEK_PRE_ROLL_NS,
                opus_head,
            },
            pending: Vec::new(),
            media_start_ns: None,
            real_frames: 0,
            encoded_frames: 0,
            packet_count: 0,
            finished: false,
        })
    }

    pub fn track_config(&self) -> &OpusTrackConfig {
        &self.track
    }

    /// 返回下一枚 Opus packet 的最早 timestamp。双轨 interleaver 用它证明另一轨已经越过
    /// 待写 packet；首块尚未抵达时，调用方必须传入确定的媒体起点（产品 session 固定为 0）。
    pub fn next_packet_timestamp_ns(
        &self,
        fallback_media_start_ns: u64,
    ) -> Result<u64, OpusWebmError> {
        self.media_start_ns
            .unwrap_or(fallback_media_start_ns)
            .checked_add(self.track.codec_delay_ns)
            .and_then(|value| value.checked_add(frames_to_ns_u64(self.encoded_frames).ok()?))
            .ok_or(OpusWebmError::TimelineOverflow)
    }

    pub fn push(
        &mut self,
        queued: QueuedAudioChunk,
    ) -> Result<Vec<EncodedOpusPacket>, OpusWebmError> {
        if self.finished {
            return Err(OpusWebmError::Finished);
        }
        self.validate_chunk(&queued)?;

        let input_start_ns = queued.presentation_at_ns;
        let normalized_start_ns = match self.media_start_ns {
            None => input_start_ns,
            Some(media_start_ns) => {
                let expected = media_start_ns
                    .checked_add(frames_to_ns_u64(self.real_frames)?)
                    .ok_or(OpusWebmError::TimelineOverflow)?;
                normalize_sample_boundary(expected, input_start_ns)?
            }
        };
        if self.media_start_ns.is_none() {
            self.media_start_ns = Some(normalized_start_ns);
        }

        let frame_count = u64::from(queued.chunk.frame_count);
        self.real_frames = self
            .real_frames
            .checked_add(frame_count)
            .ok_or(OpusWebmError::TimelineOverflow)?;
        self.pending.extend_from_slice(&queued.chunk.samples);

        let mut packets = Vec::new();
        while self.pending_frames()? >= OPUS_FRAME_FRAMES {
            packets.push(self.encode_one(0)?);
        }
        Ok(packets)
    }

    pub fn finish(mut self) -> Result<OpusFinish, OpusWebmError> {
        if self.finished {
            return Err(OpusWebmError::Finished);
        }
        self.finished = true;
        let media_start_ns = self.media_start_ns.ok_or(OpusWebmError::NoAudio)?;
        let pending_frames = self.pending_frames()?;
        let lookahead_frames = usize::from(self.track.pre_skip_frames);
        let tail_required = pending_frames
            .checked_add(lookahead_frames)
            .ok_or(OpusWebmError::TimelineOverflow)?;
        let tail_encoded_frames = tail_required
            .div_ceil(OPUS_FRAME_FRAMES)
            .checked_mul(OPUS_FRAME_FRAMES)
            .ok_or(OpusWebmError::TimelineOverflow)?;
        let end_padding_frames = tail_encoded_frames
            .checked_sub(tail_required)
            .ok_or(OpusWebmError::TimelineOverflow)?;
        let zero_frames = lookahead_frames
            .checked_add(end_padding_frames)
            .ok_or(OpusWebmError::TimelineOverflow)?;
        let zero_samples = zero_frames
            .checked_mul(usize::from(self.channels))
            .ok_or(OpusWebmError::TimelineOverflow)?;
        self.pending.resize(
            self.pending
                .len()
                .checked_add(zero_samples)
                .ok_or(OpusWebmError::TimelineOverflow)?,
            0.0,
        );

        let packet_total = tail_encoded_frames / OPUS_FRAME_FRAMES;
        let discard_padding_ns = frames_to_ns_u64(
            u64::try_from(end_padding_frames).map_err(|_| OpusWebmError::TimelineOverflow)?,
        )?;
        let mut packets = Vec::with_capacity(packet_total);
        for packet_index in 0..packet_total {
            let is_last = packet_index + 1 == packet_total;
            packets.push(self.encode_one(if is_last { discard_padding_ns } else { 0 })?);
        }
        if !self.pending.is_empty() {
            return Err(OpusWebmError::TimelineOverflow);
        }

        let media_duration_ns = frames_to_ns_u64(self.real_frames)?;
        let stats = OpusEncoderStats {
            real_frames: self.real_frames,
            encoded_frames: self.encoded_frames,
            packet_count: self.packet_count,
            pre_skip_frames: self.track.pre_skip_frames,
            end_padding_frames: u32::try_from(end_padding_frames)
                .map_err(|_| OpusWebmError::TimelineOverflow)?,
            media_start_ns,
            media_duration_ns,
        };
        Ok(OpusFinish { packets, stats })
    }

    fn validate_chunk(&self, queued: &QueuedAudioChunk) -> Result<(), OpusWebmError> {
        if queued.chunk.format.sample_rate_hz != AUDIO_SAMPLE_RATE_HZ
            || queued.chunk.format.channels != self.channels
            || queued.chunk.frame_count == 0
        {
            return Err(OpusWebmError::InvalidFormat);
        }
        let expected_samples = usize::try_from(queued.chunk.frame_count)
            .ok()
            .and_then(|frames| frames.checked_mul(usize::from(self.channels)))
            .ok_or(OpusWebmError::TimelineOverflow)?;
        let expected_duration =
            frames_to_ns(queued.chunk.frame_count).map_err(|_| OpusWebmError::TimelineOverflow)?;
        if queued.chunk.samples.len() != expected_samples
            || queued.duration_ns != expected_duration
            || queued
                .chunk
                .samples
                .iter()
                .any(|sample| !sample.is_finite())
        {
            return Err(OpusWebmError::InvalidChunk);
        }
        Ok(())
    }

    fn pending_frames(&self) -> Result<usize, OpusWebmError> {
        let channels = usize::from(self.channels);
        if !self.pending.len().is_multiple_of(channels) {
            return Err(OpusWebmError::InvalidChunk);
        }
        Ok(self.pending.len() / channels)
    }

    fn encode_one(&mut self, discard_padding_ns: u64) -> Result<EncodedOpusPacket, OpusWebmError> {
        let sample_count = OPUS_FRAME_FRAMES
            .checked_mul(usize::from(self.channels))
            .ok_or(OpusWebmError::TimelineOverflow)?;
        if self.pending.len() < sample_count {
            return Err(OpusWebmError::InvalidChunk);
        }
        let mut encoded = vec![0_u8; MAX_OPUS_PACKET_BYTES];
        let encoded_len = self
            .encoder
            .encode_float_to_slice(&self.pending[..sample_count], &mut encoded)
            .map_err(opus_error)?;
        if encoded_len == 0 || encoded_len > encoded.len() {
            return Err(OpusWebmError::Encode(
                "libopus 返回了无效 packet 长度".to_string(),
            ));
        }
        encoded.truncate(encoded_len);
        self.pending.drain(..sample_count);

        let media_start_ns = self.media_start_ns.ok_or(OpusWebmError::NoAudio)?;
        let encoded_offset_ns = frames_to_ns_u64(self.encoded_frames)?;
        let timestamp_ns = media_start_ns
            .checked_add(self.track.codec_delay_ns)
            .and_then(|value| value.checked_add(encoded_offset_ns))
            .ok_or(OpusWebmError::TimelineOverflow)?;
        self.encoded_frames = self
            .encoded_frames
            .checked_add(OPUS_FRAME_FRAMES as u64)
            .ok_or(OpusWebmError::TimelineOverflow)?;
        self.packet_count = self
            .packet_count
            .checked_add(1)
            .ok_or(OpusWebmError::TimelineOverflow)?;
        Ok(EncodedOpusPacket {
            data: encoded.into_boxed_slice(),
            timestamp_ns,
            duration_ns: OPUS_FRAME_DURATION_NS,
            discard_padding_ns,
        })
    }
}

fn build_opus_head(
    channels: u16,
    pre_skip_frames: u16,
) -> Result<[u8; OPUS_HEAD_BYTES], OpusWebmError> {
    let channels = u8::try_from(channels).map_err(|_| OpusWebmError::InvalidFormat)?;
    if !(1..=2).contains(&channels) || pre_skip_frames == 0 {
        return Err(OpusWebmError::InvalidMuxConfiguration);
    }
    let mut head = [0_u8; OPUS_HEAD_BYTES];
    head[0..8].copy_from_slice(b"OpusHead");
    head[8] = 1;
    head[9] = channels;
    head[10..12].copy_from_slice(&pre_skip_frames.to_le_bytes());
    head[12..16].copy_from_slice(&AUDIO_SAMPLE_RATE_HZ.to_le_bytes());
    head[16..18].copy_from_slice(&0_i16.to_le_bytes());
    head[18] = 0;
    Ok(head)
}

fn normalize_sample_boundary(expected: u64, actual: u64) -> Result<u64, OpusWebmError> {
    if actual < expected {
        if expected - actual < SAMPLE_PERIOD_CEIL_NS {
            return Ok(expected);
        }
        return Err(OpusWebmError::InputOverlap);
    }
    if actual - expected < SAMPLE_PERIOD_CEIL_NS {
        return Ok(expected);
    }
    Err(OpusWebmError::InputGap)
}

fn frames_to_ns_u64(frames: u64) -> Result<u64, OpusWebmError> {
    u128::from(frames)
        .checked_mul(NANOS_PER_SECOND)
        .and_then(|value| value.checked_div(u128::from(AUDIO_SAMPLE_RATE_HZ)))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(OpusWebmError::TimelineOverflow)
}

fn opus_error(error: opusic_c::ErrorCode) -> OpusWebmError {
    OpusWebmError::Encode(format!("{error:?}: {}", error.message()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MuxTrackKind {
    Video,
    Audio,
}

pub(in crate::recording) struct AvWebmOutput<W> {
    pub writer: W,
    pub video_frame_count: u64,
    pub audio_packet_count: u64,
    pub duration_ns: u64,
}

pub(in crate::recording) struct AvWebmPacketMux<W: Write + Seek> {
    segment: Option<Segment<W>>,
    video_track: VideoTrack,
    audio_track: AudioTrack,
    last_timestamp_ns: Option<u64>,
    last_video_timestamp_ns: Option<u64>,
    last_audio_timestamp_ns: Option<u64>,
    video_frame_count: u64,
    audio_packet_count: u64,
}

impl<W: Write + Seek> AvWebmPacketMux<W> {
    pub fn new(
        writer: W,
        width: u32,
        height: u32,
        audio: &OpusTrackConfig,
    ) -> Result<Self, OpusWebmError> {
        if width == 0
            || height == 0
            || !(1..=2).contains(&audio.channels)
            || audio.pre_skip_frames == 0
            || audio.codec_delay_ns != frames_to_ns_u64(u64::from(audio.pre_skip_frames))?
            || !audio.codec_delay_ns.is_multiple_of(WEBM_TIMECODE_SCALE_NS)
            || audio.seek_pre_roll_ns != OPUS_SEEK_PRE_ROLL_NS
            || audio.opus_head != build_opus_head(audio.channels, audio.pre_skip_frames)?
        {
            return Err(OpusWebmError::InvalidMuxConfiguration);
        }
        let builder = SegmentBuilder::new(Writer::new(writer))
            .map_err(mux_error)?
            .set_writing_app("Clippy")
            .map_err(mux_error)?
            .set_mode(SegmentMode::File)
            .map_err(mux_error)?
            .set_timecode_scale(WEBM_TIMECODE_SCALE_NS)
            .map_err(mux_error)?;
        let (builder, video_track) = builder
            .add_video_track(width, height, VideoCodecId::VP9, Some(1))
            .map_err(mux_error)?;
        let (builder, audio_track) = builder
            .add_audio_track(
                AUDIO_SAMPLE_RATE_HZ,
                u32::from(audio.channels),
                AudioCodecId::Opus,
                Some(2),
            )
            .map_err(mux_error)?;
        let builder = builder
            .set_codec_private(audio_track, &audio.opus_head)
            .map_err(mux_error)?
            .set_audio_codec_delay(audio_track, audio.codec_delay_ns)
            .map_err(mux_error)?
            .set_audio_seek_pre_roll(audio_track, audio.seek_pre_roll_ns)
            .map_err(mux_error)?;
        Ok(Self {
            segment: Some(builder.build()),
            video_track,
            audio_track,
            last_timestamp_ns: None,
            last_video_timestamp_ns: None,
            last_audio_timestamp_ns: None,
            video_frame_count: 0,
            audio_packet_count: 0,
        })
    }

    pub fn add_video_packet(
        &mut self,
        data: &[u8],
        timestamp_ns: u64,
        keyframe: bool,
    ) -> Result<(), OpusWebmError> {
        self.validate_timestamp(timestamp_ns, MuxTrackKind::Video)?;
        self.segment
            .as_mut()
            .ok_or(OpusWebmError::Finalize)?
            .add_frame(self.video_track, data, timestamp_ns, keyframe)
            .map_err(mux_error)?;
        self.last_timestamp_ns = Some(timestamp_ns);
        self.last_video_timestamp_ns = Some(timestamp_ns);
        self.video_frame_count = self
            .video_frame_count
            .checked_add(1)
            .ok_or(OpusWebmError::TimelineOverflow)?;
        Ok(())
    }

    pub fn add_audio_packet(&mut self, packet: &EncodedOpusPacket) -> Result<(), OpusWebmError> {
        self.validate_timestamp(packet.timestamp_ns, MuxTrackKind::Audio)?;
        let segment = self.segment.as_mut().ok_or(OpusWebmError::Finalize)?;
        if packet.data.is_empty() || packet.duration_ns != OPUS_FRAME_DURATION_NS {
            return Err(OpusWebmError::InvalidMuxConfiguration);
        }
        if packet.discard_padding_ns == 0 {
            segment
                .add_frame(self.audio_track, &packet.data, packet.timestamp_ns, true)
                .map_err(mux_error)?;
        } else {
            let discard_padding = i64::try_from(packet.discard_padding_ns)
                .map_err(|_| OpusWebmError::TimelineOverflow)?;
            segment
                .add_frame_with_discard_padding(
                    self.audio_track,
                    &packet.data,
                    packet.timestamp_ns,
                    true,
                    discard_padding,
                )
                .map_err(mux_error)?;
        }
        self.last_timestamp_ns = Some(packet.timestamp_ns);
        self.last_audio_timestamp_ns = Some(packet.timestamp_ns);
        self.audio_packet_count = self
            .audio_packet_count
            .checked_add(1)
            .ok_or(OpusWebmError::TimelineOverflow)?;
        Ok(())
    }

    pub fn finish(mut self, duration_ns: u64) -> Result<AvWebmOutput<W>, OpusWebmError> {
        if duration_ns == 0 || self.video_frame_count == 0 || self.audio_packet_count == 0 {
            return Err(OpusWebmError::InvalidMuxConfiguration);
        }
        let duration_ticks = duration_ns.div_ceil(WEBM_TIMECODE_SCALE_NS).max(1);
        let writer = self
            .segment
            .take()
            .ok_or(OpusWebmError::Finalize)?
            .finalize(Some(duration_ticks))
            .map_err(|_| OpusWebmError::Finalize)?
            .into_inner();
        Ok(AvWebmOutput {
            writer,
            video_frame_count: self.video_frame_count,
            audio_packet_count: self.audio_packet_count,
            duration_ns,
        })
    }

    fn validate_timestamp(
        &self,
        timestamp_ns: u64,
        track: MuxTrackKind,
    ) -> Result<(), OpusWebmError> {
        let track_timestamp = match track {
            MuxTrackKind::Video => self.last_video_timestamp_ns,
            MuxTrackKind::Audio => self.last_audio_timestamp_ns,
        };
        if self
            .last_timestamp_ns
            .is_some_and(|last_timestamp| timestamp_ns < last_timestamp)
            || track_timestamp.is_some_and(|last_timestamp| timestamp_ns <= last_timestamp)
        {
            return Err(OpusWebmError::InvalidMuxTimestamp);
        }
        Ok(())
    }
}

fn mux_error(error: webm::mux::Error) -> OpusWebmError {
    OpusWebmError::Mux(format!("{error:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::audio::{AudioFormat, CapturedAudioChunk};
    use crate::recording::mux::vp9_webm::Vp9PacketEncoder;
    use opusic_c::Decoder;
    use std::f32::consts::TAU;
    use std::io::{Cursor, Write};
    use std::process::Command;

    fn queued(
        sequence: u64,
        presentation_at_ns: u64,
        frames: u32,
        channels: u16,
        phase_frames: u64,
    ) -> QueuedAudioChunk {
        let mut samples = Vec::with_capacity(frames as usize * channels as usize);
        for frame in 0..frames {
            let phase = ((phase_frames + u64::from(frame)) as f32 * 440.0 * TAU)
                / AUDIO_SAMPLE_RATE_HZ as f32;
            let value = phase.sin() * 0.35;
            for channel in 0..channels {
                samples.push(if channel == 0 { value } else { value * 0.5 });
            }
        }
        QueuedAudioChunk {
            chunk: CapturedAudioChunk {
                sequence,
                captured_at_ns: presentation_at_ns,
                format: AudioFormat::normalized(channels),
                frame_count: frames,
                samples: samples.into_boxed_slice(),
            },
            presentation_at_ns,
            duration_ns: frames_to_ns(frames).unwrap(),
            gap_before_ns: if sequence == 0 { presentation_at_ns } else { 0 },
        }
    }

    fn decode_and_trim(
        channels: u16,
        config: &OpusTrackConfig,
        packets: &[EncodedOpusPacket],
        end_padding_frames: u32,
    ) -> Vec<f32> {
        let opus_channels = if channels == 1 {
            Channels::Mono
        } else {
            Channels::Stereo
        };
        let mut decoder = Decoder::new(opus_channels, SampleRate::Hz48000).unwrap();
        let mut decoded = Vec::new();
        for packet in packets {
            let mut frame = vec![0.0_f32; OPUS_FRAME_FRAMES * channels as usize];
            let frames = decoder
                .decode_float_to_slice(&packet.data, &mut frame, false)
                .unwrap();
            assert_eq!(frames, OPUS_FRAME_FRAMES);
            decoded.extend_from_slice(&frame[..frames * channels as usize]);
        }
        let start = usize::from(config.pre_skip_frames) * channels as usize;
        let end = decoded.len() - end_padding_frames as usize * channels as usize;
        decoded[start..end].to_vec()
    }

    #[test]
    fn opus_head_uses_encoder_lookahead_and_mapping_family_zero() {
        for channels in [1, 2] {
            let encoder = OpusPacketEncoder::new(channels).unwrap();
            let config = encoder.track_config();
            assert_eq!(&config.opus_head[0..8], b"OpusHead");
            assert_eq!(config.opus_head[8], 1);
            assert_eq!(config.opus_head[9], channels as u8);
            assert_eq!(
                u16::from_le_bytes(config.opus_head[10..12].try_into().unwrap()),
                config.pre_skip_frames
            );
            assert_eq!(
                u32::from_le_bytes(config.opus_head[12..16].try_into().unwrap()),
                AUDIO_SAMPLE_RATE_HZ
            );
            assert_eq!(&config.opus_head[16..18], &[0, 0]);
            assert_eq!(config.opus_head[18], 0);
            assert_eq!(
                config.codec_delay_ns,
                frames_to_ns_u64(u64::from(config.pre_skip_frames)).unwrap()
            );
        }
    }

    #[test]
    fn arbitrary_chunks_roundtrip_with_exact_real_sample_count() {
        for channels in [1, 2] {
            let mut encoder = OpusPacketEncoder::new(channels).unwrap();
            let config = encoder.track_config().clone();
            let mut packets = encoder.push(queued(0, 0, 333, channels, 0)).unwrap();
            packets.extend(
                encoder
                    .push(queued(
                        1,
                        frames_to_ns_u64(333).unwrap(),
                        667,
                        channels,
                        333,
                    ))
                    .unwrap(),
            );
            let finished = encoder.finish().unwrap();
            packets.extend(finished.packets);

            assert_eq!(packets[0].timestamp_ns, config.codec_delay_ns);
            let decoded = decode_and_trim(
                channels,
                &config,
                &packets,
                finished.stats.end_padding_frames,
            );
            assert_eq!(finished.stats.real_frames, 1_000);
            assert_eq!(decoded.len(), 1_000 * channels as usize);
            assert!(decoded.iter().all(|sample| sample.is_finite()));
            let rms = (decoded.iter().map(|sample| sample * sample).sum::<f32>()
                / decoded.len() as f32)
                .sqrt();
            assert!(rms > 0.05);
            assert_eq!(
                finished.stats.encoded_frames,
                1_000
                    + u64::from(finished.stats.pre_skip_frames)
                    + u64::from(finished.stats.end_padding_frames)
            );
            assert_eq!(
                packets.last().unwrap().discard_padding_ns,
                frames_to_ns_u64(u64::from(finished.stats.end_padding_frames)).unwrap()
            );
        }
    }

    #[test]
    fn encoder_rejects_real_gaps_but_absorbs_sub_sample_rounding() {
        let mut encoder = OpusPacketEncoder::new(2).unwrap();
        encoder.push(queued(0, 0, 480, 2, 0)).unwrap();
        assert!(encoder.push(queued(1, 10_000_001, 480, 2, 480)).is_ok());

        let mut gap = OpusPacketEncoder::new(2).unwrap();
        gap.push(queued(0, 0, 480, 2, 0)).unwrap();
        assert_eq!(
            gap.push(queued(1, 10_100_000, 480, 2, 480)),
            Err(OpusWebmError::InputGap)
        );

        let mut overlap = OpusPacketEncoder::new(2).unwrap();
        overlap.push(queued(0, 0, 480, 2, 0)).unwrap();
        assert_eq!(
            overlap.push(queued(1, 9_900_000, 480, 2, 480)),
            Err(OpusWebmError::InputOverlap)
        );
    }

    #[test]
    fn finish_flushes_lookahead_and_marks_only_real_end_padding() {
        let mut encoder = OpusPacketEncoder::new(2).unwrap();
        let config = encoder.track_config().clone();
        let mut packets = encoder.push(queued(0, 0, 960, 2, 0)).unwrap();
        assert_eq!(packets.len(), 1);
        let finished = encoder.finish().unwrap();
        assert!(!finished.packets.is_empty());
        assert!(finished.packets.last().unwrap().discard_padding_ns > 0);
        packets.extend(finished.packets);
        assert_eq!(
            decode_and_trim(2, &config, &packets, finished.stats.end_padding_frames,).len(),
            1_920
        );
        assert_eq!(finished.stats.real_frames, 960);
    }

    #[test]
    fn dual_track_webm_contains_opus_timing_metadata_and_tail_padding() {
        let width = 64;
        let height = 48;
        let rgba = vec![0x80_u8; width * height * 4];
        let mut video = Vp9PacketEncoder::new(width as u32, height as u32, 10, 1).unwrap();
        let mut video_packets = Vec::new();
        video
            .push_rgba(&rgba, 0, &mut |data, timestamp, keyframe| {
                video_packets.push((data.to_vec(), timestamp, keyframe));
                Ok(())
            })
            .unwrap();
        video
            .finish(100_000_000, &mut |data, timestamp, keyframe| {
                video_packets.push((data.to_vec(), timestamp, keyframe));
                Ok(())
            })
            .unwrap();

        let mut audio = OpusPacketEncoder::new(2).unwrap();
        let audio_config = audio.track_config().clone();
        let mut audio_packets = audio.push(queued(0, 0, 1_000, 2, 0)).unwrap();
        let audio_finish = audio.finish().unwrap();
        audio_packets.extend(audio_finish.packets);

        enum Packet {
            Video(Vec<u8>, u64, bool),
            Audio(EncodedOpusPacket),
        }
        let mut packets = video_packets
            .into_iter()
            .map(|(data, timestamp, keyframe)| Packet::Video(data, timestamp, keyframe))
            .chain(audio_packets.into_iter().map(Packet::Audio))
            .collect::<Vec<_>>();
        packets.sort_by_key(|packet| match packet {
            Packet::Video(_, timestamp, _) => (*timestamp, 0_u8),
            Packet::Audio(packet) => (packet.timestamp_ns, 1_u8),
        });

        let mut mux = AvWebmPacketMux::new(
            Cursor::new(Vec::new()),
            width as u32,
            height as u32,
            &audio_config,
        )
        .unwrap();
        for packet in &packets {
            match packet {
                Packet::Video(data, timestamp, keyframe) => {
                    mux.add_video_packet(data, *timestamp, *keyframe).unwrap()
                }
                Packet::Audio(packet) => mux.add_audio_packet(packet).unwrap(),
            }
        }
        let output = mux.finish(100_000_000).unwrap();
        assert_eq!(output.video_frame_count, 1);
        assert_eq!(output.audio_packet_count, audio_finish.stats.packet_count);
        assert_eq!(output.duration_ns, 100_000_000);
        let bytes = output.writer.into_inner();

        assert!(bytes.windows(5).any(|window| window == b"V_VP9"));
        assert!(bytes.windows(6).any(|window| window == b"A_OPUS"));
        assert!(bytes
            .windows(OPUS_HEAD_BYTES)
            .any(|window| window == audio_config.opus_head));
        assert_eq!(
            read_unsigned_element(&bytes, &[0x56, 0xAA]),
            Some(audio_config.codec_delay_ns)
        );
        assert_eq!(
            read_unsigned_element(&bytes, &[0x56, 0xBB]),
            Some(OPUS_SEEK_PRE_ROLL_NS)
        );
        assert_eq!(
            read_unsigned_element(&bytes, &[0x2A, 0xD7, 0xB1]),
            Some(WEBM_TIMECODE_SCALE_NS)
        );
        assert_eq!(
            read_signed_element(&bytes, &[0x75, 0xA2]),
            Some(
                i64::try_from(
                    packets
                        .iter()
                        .filter_map(|packet| match packet {
                            Packet::Audio(packet) => Some(packet.discard_padding_ns),
                            Packet::Video(..) => None,
                        })
                        .max()
                        .unwrap()
                )
                .unwrap()
            )
        );

        if Command::new("ffprobe").arg("-version").output().is_ok() {
            let mut file = tempfile::NamedTempFile::new().unwrap();
            file.write_all(&bytes).unwrap();
            let probe = Command::new("ffprobe")
                .args([
                    "-v",
                    "error",
                    "-show_entries",
                    "stream=codec_name:packet=codec_type,pts_time",
                    "-of",
                    "json",
                ])
                .arg(file.path())
                .output()
                .unwrap();
            assert!(
                probe.status.success(),
                "{}",
                String::from_utf8_lossy(&probe.stderr)
            );
            let report: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
            let codecs = report["streams"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|stream| stream["codec_name"].as_str())
                .collect::<Vec<_>>();
            assert!(codecs.contains(&"vp9"));
            assert!(codecs.contains(&"opus"));
            let first_audio_pts = report["packets"]
                .as_array()
                .unwrap()
                .iter()
                .find(|packet| packet["codec_type"] == "audio")
                .and_then(|packet| packet["pts_time"].as_str())
                .unwrap()
                .parse::<f64>()
                .unwrap();
            assert!(first_audio_pts >= 0.0);
            assert!(first_audio_pts < 0.001);
        }
    }

    #[test]
    fn mux_rejects_cross_track_reordering_and_same_track_duplicate_timestamps() {
        let encoder = OpusPacketEncoder::new(1).unwrap();
        let config = encoder.track_config().clone();
        let mut mux = AvWebmPacketMux::new(Cursor::new(Vec::new()), 64, 48, &config).unwrap();
        mux.add_video_packet(&[1], 10, true).unwrap();
        let audio = EncodedOpusPacket {
            data: vec![1].into_boxed_slice(),
            timestamp_ns: 9,
            duration_ns: OPUS_FRAME_DURATION_NS,
            discard_padding_ns: 0,
        };
        assert_eq!(
            mux.add_audio_packet(&audio),
            Err(OpusWebmError::InvalidMuxTimestamp)
        );
        let audio = EncodedOpusPacket {
            timestamp_ns: 10,
            ..audio
        };
        mux.add_audio_packet(&audio).unwrap();
        assert_eq!(
            mux.add_video_packet(&[2], 10, false),
            Err(OpusWebmError::InvalidMuxTimestamp)
        );
    }

    fn read_unsigned_element(bytes: &[u8], id: &[u8]) -> Option<u64> {
        let start = find_subslice(bytes, id)?.checked_add(id.len())?;
        let (size, size_len) = read_vint(&bytes[start..])?;
        let data_start = start.checked_add(size_len)?;
        let data_end = data_start.checked_add(size)?;
        let data = bytes.get(data_start..data_end)?;
        if data.is_empty() || data.len() > 8 {
            return None;
        }
        Some(
            data.iter()
                .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte)),
        )
    }

    fn read_signed_element(bytes: &[u8], id: &[u8]) -> Option<i64> {
        let start = find_subslice(bytes, id)?.checked_add(id.len())?;
        let (size, size_len) = read_vint(&bytes[start..])?;
        let data_start = start.checked_add(size_len)?;
        let data_end = data_start.checked_add(size)?;
        let data = bytes.get(data_start..data_end)?;
        if data.is_empty() || data.len() > 8 {
            return None;
        }
        let unsigned = data
            .iter()
            .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte));
        let shift = (8 - data.len()) * 8;
        Some(((unsigned << shift) as i64) >> shift)
    }

    fn find_subslice(bytes: &[u8], needle: &[u8]) -> Option<usize> {
        bytes
            .windows(needle.len())
            .position(|window| window == needle)
    }

    fn read_vint(bytes: &[u8]) -> Option<(usize, usize)> {
        let first = *bytes.first()?;
        let length = first.leading_zeros() as usize + 1;
        if length > 8 || bytes.len() < length {
            return None;
        }
        let marker = 1_u8 << (8 - length);
        let mut value = usize::from(first & (marker - 1));
        for byte in &bytes[1..length] {
            value = value.checked_shl(8)?.checked_add(usize::from(*byte))?;
        }
        Some((value, length))
    }
}
