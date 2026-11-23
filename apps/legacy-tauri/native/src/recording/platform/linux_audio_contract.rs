//! PipeWire 音频 packet 到录屏 PCM 块的纯 Rust 合同。
//!
//! 原生层只负责校验 SPA buffer 并复制 F32LE sample。这里固定单调时钟中点校准、原生 PTS
//! 映射、packet 边界、gap 静音与 20 ms 拆块，因此非 Linux 宿主也能运行关键测试。

use super::super::audio::{
    AudioFormat, CapturedAudioChunk, AUDIO_SAMPLE_RATE_HZ, MAX_AUDIO_CHUNK_FRAMES,
};
use std::collections::VecDeque;
use thiserror::Error;

pub(super) const PIPEWIRE_CHANNELS: u16 = 2;
pub(super) const PIPEWIRE_FRAME_BYTES: usize =
    PIPEWIRE_CHANNELS as usize * std::mem::size_of::<f32>();
pub(super) const PIPEWIRE_CHUNK_FRAMES: u32 = AUDIO_SAMPLE_RATE_HZ / 50;
pub(super) const MAX_PIPEWIRE_PACKET_FRAMES: u32 = MAX_AUDIO_CHUNK_FRAMES;
pub(super) const MAX_PIPEWIRE_PACKET_BYTES: usize =
    MAX_PIPEWIRE_PACKET_FRAMES as usize * PIPEWIRE_FRAME_BYTES;
const NANOS_PER_SECOND: u64 = 1_000_000_000;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(in crate::recording) enum LinuxAudioContractError {
    #[error("PipeWire 单调时钟校准区间无效")]
    InvalidCalibration,
    #[error("PipeWire 音频 PTS 无效")]
    InvalidTimestamp,
    #[error("PipeWire 音频 PTS 无法映射到录屏会话")]
    TimestampOutOfRange,
    #[error("PipeWire 音频 PTS 倒退")]
    TimestampRegression,
    #[error("PipeWire 音频 packet 与上一 packet 重叠")]
    TimestampOverlap,
    #[error("PipeWire 音频 packet 不能为空")]
    EmptyPacket,
    #[error("PipeWire 音频 packet 超过 100 ms 上限")]
    PacketTooLong,
    #[error("PipeWire 音频 packet 长度溢出")]
    SampleLengthOverflow,
    #[error("PipeWire 音频 packet sample 数量不匹配")]
    SampleLengthMismatch,
    #[error("PipeWire 音频包含非有限 sample")]
    NonFiniteSample,
    #[error("PipeWire 音频 chunk 序号耗尽")]
    SequenceExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LinuxAudioPtsMapper {
    native_anchor_ns: u64,
    session_anchor_ns: u64,
    last_native_pts_ns: Option<u64>,
    last_end_ns: Option<u64>,
}

impl LinuxAudioPtsMapper {
    /// `native_clock_ns` 必须在两次 session clock 采样之间读取；中点让校准误差受采样区间约束。
    pub fn from_calibration(
        native_clock_ns: u64,
        session_before_ns: u64,
        session_after_ns: u64,
    ) -> Result<Self, LinuxAudioContractError> {
        if session_after_ns < session_before_ns {
            return Err(LinuxAudioContractError::InvalidCalibration);
        }
        let interval = session_after_ns - session_before_ns;
        let session_anchor_ns = session_before_ns
            .checked_add(interval / 2)
            .ok_or(LinuxAudioContractError::InvalidCalibration)?;
        Ok(Self {
            native_anchor_ns: native_clock_ns,
            session_anchor_ns,
            last_native_pts_ns: None,
            last_end_ns: None,
        })
    }

    pub fn map_packet(
        &mut self,
        native_pts_ns: i64,
        frame_count: u32,
    ) -> Result<u64, LinuxAudioContractError> {
        validate_frame_count(frame_count)?;
        let native_pts_ns =
            u64::try_from(native_pts_ns).map_err(|_| LinuxAudioContractError::InvalidTimestamp)?;
        if self
            .last_native_pts_ns
            .is_some_and(|last| native_pts_ns <= last)
        {
            return Err(LinuxAudioContractError::TimestampRegression);
        }
        let mapped_start =
            map_native_timestamp(self.native_anchor_ns, self.session_anchor_ns, native_pts_ns)?;
        if self
            .last_end_ns
            .is_some_and(|last_end| mapped_start < last_end)
        {
            return Err(LinuxAudioContractError::TimestampOverlap);
        }
        let mapped_end = mapped_start
            .checked_add(frames_to_ns(frame_count)?)
            .ok_or(LinuxAudioContractError::TimestampOutOfRange)?;
        self.last_native_pts_ns = Some(native_pts_ns);
        self.last_end_ns = Some(mapped_end);
        Ok(mapped_start)
    }

    pub fn control_timestamp_ns(self, session_now_ns: u64) -> u64 {
        self.last_end_ns
            .map_or(session_now_ns, |last_end| last_end.max(session_now_ns))
    }
}

fn map_native_timestamp(
    native_anchor_ns: u64,
    session_anchor_ns: u64,
    native_pts_ns: u64,
) -> Result<u64, LinuxAudioContractError> {
    if native_pts_ns >= native_anchor_ns {
        session_anchor_ns
            .checked_add(native_pts_ns - native_anchor_ns)
            .ok_or(LinuxAudioContractError::TimestampOutOfRange)
    } else {
        session_anchor_ns
            .checked_sub(native_anchor_ns - native_pts_ns)
            .ok_or(LinuxAudioContractError::TimestampOutOfRange)
    }
}

pub(super) struct PacketChunks {
    pub chunks: VecDeque<CapturedAudioChunk>,
    pub next_sequence: u64,
    pub end_ns: u64,
}

pub(super) fn packet_to_chunks(
    first_sequence: u64,
    captured_at_ns: u64,
    frame_count: u32,
    samples: Option<&[f32]>,
) -> Result<PacketChunks, LinuxAudioContractError> {
    validate_frame_count(frame_count)?;
    let expected_samples = usize::try_from(frame_count)
        .ok()
        .and_then(|frames| frames.checked_mul(usize::from(PIPEWIRE_CHANNELS)))
        .ok_or(LinuxAudioContractError::SampleLengthOverflow)?;
    if samples.is_some_and(|samples| samples.len() != expected_samples) {
        return Err(LinuxAudioContractError::SampleLengthMismatch);
    }
    if samples.is_some_and(|samples| samples.iter().any(|sample| !sample.is_finite())) {
        return Err(LinuxAudioContractError::NonFiniteSample);
    }

    let mut chunks = VecDeque::new();
    let mut frame_offset = 0_u32;
    let mut sequence = first_sequence;
    while frame_offset < frame_count {
        let chunk_frames = (frame_count - frame_offset).min(PIPEWIRE_CHUNK_FRAMES);
        let sample_start = usize::try_from(frame_offset)
            .ok()
            .and_then(|offset| offset.checked_mul(usize::from(PIPEWIRE_CHANNELS)))
            .ok_or(LinuxAudioContractError::SampleLengthOverflow)?;
        let sample_len = usize::try_from(chunk_frames)
            .ok()
            .and_then(|frames| frames.checked_mul(usize::from(PIPEWIRE_CHANNELS)))
            .ok_or(LinuxAudioContractError::SampleLengthOverflow)?;
        let owned = match samples {
            Some(samples) => samples[sample_start..sample_start + sample_len]
                .to_vec()
                .into_boxed_slice(),
            None => vec![0.0; sample_len].into_boxed_slice(),
        };
        chunks.push_back(CapturedAudioChunk {
            sequence,
            captured_at_ns: captured_at_ns
                .checked_add(frames_to_ns(frame_offset)?)
                .ok_or(LinuxAudioContractError::TimestampOutOfRange)?,
            format: AudioFormat::normalized(PIPEWIRE_CHANNELS),
            frame_count: chunk_frames,
            samples: owned,
        });
        sequence = sequence
            .checked_add(1)
            .ok_or(LinuxAudioContractError::SequenceExhausted)?;
        frame_offset = frame_offset
            .checked_add(chunk_frames)
            .ok_or(LinuxAudioContractError::SampleLengthOverflow)?;
    }
    let end_ns = captured_at_ns
        .checked_add(frames_to_ns(frame_count)?)
        .ok_or(LinuxAudioContractError::TimestampOutOfRange)?;
    Ok(PacketChunks {
        chunks,
        next_sequence: sequence,
        end_ns,
    })
}

fn validate_frame_count(frame_count: u32) -> Result<(), LinuxAudioContractError> {
    if frame_count == 0 {
        Err(LinuxAudioContractError::EmptyPacket)
    } else if frame_count > MAX_PIPEWIRE_PACKET_FRAMES {
        Err(LinuxAudioContractError::PacketTooLong)
    } else {
        Ok(())
    }
}

fn frames_to_ns(frame_count: u32) -> Result<u64, LinuxAudioContractError> {
    u64::from(frame_count)
        .checked_mul(NANOS_PER_SECOND)
        .map(|value| value / u64::from(AUDIO_SAMPLE_RATE_HZ))
        .ok_or(LinuxAudioContractError::TimestampOutOfRange)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midpoint_calibration_maps_pts_on_both_sides_of_anchor() {
        let mapper = LinuxAudioPtsMapper::from_calibration(1_000_000, 400, 600).unwrap();
        assert_eq!(
            map_native_timestamp(mapper.native_anchor_ns, mapper.session_anchor_ns, 1_000_250),
            Ok(750)
        );
        assert_eq!(
            map_native_timestamp(mapper.native_anchor_ns, mapper.session_anchor_ns, 999_900),
            Ok(400)
        );
        assert_eq!(
            LinuxAudioPtsMapper::from_calibration(0, 2, 1),
            Err(LinuxAudioContractError::InvalidCalibration)
        );
    }

    #[test]
    fn pts_mapper_preserves_real_gaps_and_rejects_regression_or_overlap() {
        let mut mapper = LinuxAudioPtsMapper::from_calibration(10_000, 1_000, 1_000).unwrap();
        assert_eq!(mapper.map_packet(10_000, 960), Ok(1_000));
        assert_eq!(mapper.map_packet(60_000_000, 960), Ok(59_991_000));
        assert_eq!(mapper.control_timestamp_ns(2), 79_991_000);

        let mut regressing = LinuxAudioPtsMapper::from_calibration(10_000, 1_000, 1_000).unwrap();
        assert_eq!(regressing.map_packet(10_000, 960), Ok(1_000));
        assert_eq!(
            regressing.map_packet(9_999, 960),
            Err(LinuxAudioContractError::TimestampRegression)
        );

        let mut overlapping = LinuxAudioPtsMapper::from_calibration(10_000, 1_000, 1_000).unwrap();
        assert_eq!(overlapping.map_packet(10_000, 960), Ok(1_000));
        assert_eq!(
            overlapping.map_packet(10_001, 960),
            Err(LinuxAudioContractError::TimestampOverlap)
        );
    }

    #[test]
    fn splits_real_samples_and_gap_silence_at_twenty_milliseconds() {
        let frames = PIPEWIRE_CHUNK_FRAMES * 2 + 17;
        let samples = vec![0.25; frames as usize * usize::from(PIPEWIRE_CHANNELS)];
        let packet = packet_to_chunks(7, 11, frames, Some(&samples)).unwrap();
        assert_eq!(packet.chunks.len(), 3);
        assert_eq!(packet.chunks[0].sequence, 7);
        assert_eq!(packet.chunks[0].frame_count, PIPEWIRE_CHUNK_FRAMES);
        assert_eq!(packet.chunks[1].captured_at_ns, 20_000_011);
        assert_eq!(packet.chunks[2].captured_at_ns, 40_000_011);
        assert_eq!(packet.chunks[2].frame_count, 17);
        assert!(packet
            .chunks
            .iter()
            .flat_map(|chunk| chunk.samples.iter())
            .all(|sample| *sample == 0.25));

        let gap = packet_to_chunks(0, 0, 2, None).unwrap();
        assert_eq!(gap.chunks[0].samples.as_ref(), &[0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn rejects_bad_packet_shape_values_and_sequence_overflow() {
        assert!(matches!(
            packet_to_chunks(0, 0, 0, None),
            Err(LinuxAudioContractError::EmptyPacket)
        ));
        assert!(matches!(
            packet_to_chunks(0, 0, MAX_PIPEWIRE_PACKET_FRAMES + 1, None),
            Err(LinuxAudioContractError::PacketTooLong)
        ));
        assert!(matches!(
            packet_to_chunks(0, 0, 1, Some(&[0.0])),
            Err(LinuxAudioContractError::SampleLengthMismatch)
        ));
        assert!(matches!(
            packet_to_chunks(0, 0, 1, Some(&[f32::NAN, 0.0])),
            Err(LinuxAudioContractError::NonFiniteSample)
        ));
        assert!(matches!(
            packet_to_chunks(u64::MAX, 0, 1, None),
            Err(LinuxAudioContractError::SequenceExhausted)
        ));
    }

    #[test]
    fn negative_and_unmappable_pts_fail_closed() {
        let mut mapper = LinuxAudioPtsMapper::from_calibration(100, 0, 0).unwrap();
        assert_eq!(
            mapper.map_packet(-1, 1),
            Err(LinuxAudioContractError::InvalidTimestamp)
        );
        assert_eq!(
            mapper.map_packet(99, 1),
            Err(LinuxAudioContractError::TimestampOutOfRange)
        );
    }
}
