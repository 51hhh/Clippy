//! WASAPI packet 到录屏 PCM 块的纯函数合同。
//!
//! 这里不引用 Windows API，因此 Linux 本地门禁也能验证 QPC 映射、静音和拆块边界。

use super::super::audio::{AudioFormat, CapturedAudioChunk, AUDIO_SAMPLE_RATE_HZ};
use std::collections::VecDeque;
use thiserror::Error;

pub(super) const WASAPI_CHANNELS: u16 = 2;
pub(super) const WASAPI_CHUNK_FRAMES: u32 = AUDIO_SAMPLE_RATE_HZ / 50;
const HUNDRED_NS_PER_SECOND: u128 = 10_000_000;
const NANOS_PER_HUNDRED_NS: u64 = 100;
const NANOS_PER_SECOND: u64 = 1_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WindowsAudioSourceKind {
    SystemLoopback,
    DefaultMicrophone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WindowsAudioEndpointFlow {
    Render,
    Capture,
}

impl WindowsAudioSourceKind {
    pub const fn uses_loopback(self) -> bool {
        matches!(self, Self::SystemLoopback)
    }

    pub const fn endpoint_flow(self) -> WindowsAudioEndpointFlow {
        match self {
            Self::SystemLoopback => WindowsAudioEndpointFlow::Render,
            Self::DefaultMicrophone => WindowsAudioEndpointFlow::Capture,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(in crate::recording) enum WindowsAudioContractError {
    #[error("WASAPI QPC 频率无效")]
    InvalidQpcFrequency,
    #[error("WASAPI QPC 计数器无效")]
    InvalidQpcCounter,
    #[error("WASAPI 时间戳无法映射到录屏会话")]
    TimestampOutOfRange,
    #[error("WASAPI packet 不能为空")]
    EmptyPacket,
    #[error("WASAPI packet 样本长度溢出")]
    SampleLengthOverflow,
    #[error("WASAPI packet 样本长度不匹配")]
    SampleLengthMismatch,
    #[error("WASAPI packet 序号耗尽")]
    SequenceExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct QpcClockMapper {
    qpc_anchor_100ns: u64,
    session_anchor_ns: u64,
}

impl QpcClockMapper {
    pub fn from_calibration(
        qpc_counter: i64,
        qpc_frequency: i64,
        session_before_ns: u64,
        session_after_ns: u64,
    ) -> Result<Self, WindowsAudioContractError> {
        let qpc_anchor_100ns = qpc_ticks_to_100ns(qpc_counter, qpc_frequency)?;
        let elapsed = session_after_ns.saturating_sub(session_before_ns);
        let session_anchor_ns = session_before_ns.saturating_add(elapsed / 2);
        Ok(Self {
            qpc_anchor_100ns,
            session_anchor_ns,
        })
    }

    pub fn map_100ns(self, qpc_position_100ns: u64) -> Result<u64, WindowsAudioContractError> {
        if qpc_position_100ns >= self.qpc_anchor_100ns {
            let delta_ns = qpc_position_100ns
                .checked_sub(self.qpc_anchor_100ns)
                .and_then(|delta| delta.checked_mul(NANOS_PER_HUNDRED_NS))
                .ok_or(WindowsAudioContractError::TimestampOutOfRange)?;
            self.session_anchor_ns
                .checked_add(delta_ns)
                .ok_or(WindowsAudioContractError::TimestampOutOfRange)
        } else {
            let delta_ns = self
                .qpc_anchor_100ns
                .checked_sub(qpc_position_100ns)
                .and_then(|delta| delta.checked_mul(NANOS_PER_HUNDRED_NS))
                .ok_or(WindowsAudioContractError::TimestampOutOfRange)?;
            self.session_anchor_ns
                .checked_sub(delta_ns)
                .ok_or(WindowsAudioContractError::TimestampOutOfRange)
        }
    }
}

fn qpc_ticks_to_100ns(
    qpc_counter: i64,
    qpc_frequency: i64,
) -> Result<u64, WindowsAudioContractError> {
    let counter =
        u128::try_from(qpc_counter).map_err(|_| WindowsAudioContractError::InvalidQpcCounter)?;
    let frequency = u128::try_from(qpc_frequency)
        .ok()
        .filter(|frequency| *frequency > 0)
        .ok_or(WindowsAudioContractError::InvalidQpcFrequency)?;
    let value = counter
        .checked_mul(HUNDRED_NS_PER_SECOND)
        .ok_or(WindowsAudioContractError::TimestampOutOfRange)?
        / frequency;
    u64::try_from(value).map_err(|_| WindowsAudioContractError::TimestampOutOfRange)
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
) -> Result<PacketChunks, WindowsAudioContractError> {
    if frame_count == 0 {
        return Err(WindowsAudioContractError::EmptyPacket);
    }
    let sample_count = usize::try_from(frame_count)
        .ok()
        .and_then(|frames| frames.checked_mul(usize::from(WASAPI_CHANNELS)))
        .ok_or(WindowsAudioContractError::SampleLengthOverflow)?;
    if samples.is_some_and(|samples| samples.len() != sample_count) {
        return Err(WindowsAudioContractError::SampleLengthMismatch);
    }

    let mut chunks = VecDeque::new();
    let mut frame_offset = 0_u32;
    let mut sequence = first_sequence;
    while frame_offset < frame_count {
        let chunk_frames = (frame_count - frame_offset).min(WASAPI_CHUNK_FRAMES);
        let chunk_start = usize::try_from(frame_offset)
            .ok()
            .and_then(|offset| offset.checked_mul(usize::from(WASAPI_CHANNELS)))
            .ok_or(WindowsAudioContractError::SampleLengthOverflow)?;
        let chunk_samples = usize::try_from(chunk_frames)
            .ok()
            .and_then(|frames| frames.checked_mul(usize::from(WASAPI_CHANNELS)))
            .ok_or(WindowsAudioContractError::SampleLengthOverflow)?;
        let owned_samples = match samples {
            Some(samples) => samples[chunk_start..chunk_start + chunk_samples]
                .to_vec()
                .into_boxed_slice(),
            None => vec![0.0; chunk_samples].into_boxed_slice(),
        };
        chunks.push_back(CapturedAudioChunk {
            sequence,
            captured_at_ns: captured_at_ns
                .checked_add(frames_to_ns(frame_offset)?)
                .ok_or(WindowsAudioContractError::TimestampOutOfRange)?,
            format: AudioFormat::normalized(WASAPI_CHANNELS),
            frame_count: chunk_frames,
            samples: owned_samples,
        });
        sequence = sequence
            .checked_add(1)
            .ok_or(WindowsAudioContractError::SequenceExhausted)?;
        frame_offset += chunk_frames;
    }

    let end_ns = captured_at_ns
        .checked_add(frames_to_ns(frame_count)?)
        .ok_or(WindowsAudioContractError::TimestampOutOfRange)?;
    Ok(PacketChunks {
        chunks,
        next_sequence: sequence,
        end_ns,
    })
}

fn frames_to_ns(frame_count: u32) -> Result<u64, WindowsAudioContractError> {
    u64::from(frame_count)
        .checked_mul(NANOS_PER_SECOND)
        .map(|value| value / u64::from(AUDIO_SAMPLE_RATE_HZ))
        .ok_or(WindowsAudioContractError::TimestampOutOfRange)
}

pub(super) fn safe_control_timestamp(current_ns: u64, last_packet_end_ns: Option<u64>) -> u64 {
    current_ns.max(last_packet_end_ns.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_kind_only_enables_loopback_for_system_sound() {
        assert!(WindowsAudioSourceKind::SystemLoopback.uses_loopback());
        assert!(!WindowsAudioSourceKind::DefaultMicrophone.uses_loopback());
        assert_eq!(
            WindowsAudioSourceKind::SystemLoopback.endpoint_flow(),
            WindowsAudioEndpointFlow::Render
        );
        assert_eq!(
            WindowsAudioSourceKind::DefaultMicrophone.endpoint_flow(),
            WindowsAudioEndpointFlow::Capture
        );
    }

    #[test]
    fn qpc_mapper_uses_midpoint_and_maps_both_directions() {
        let mapper = QpcClockMapper::from_calibration(25_000, 10_000, 8_000, 8_200).unwrap();

        assert_eq!(mapper.map_100ns(25_001_000).unwrap(), 108_100);
        assert_eq!(mapper.map_100ns(24_999_919).unwrap(), 0);
    }

    #[test]
    fn qpc_mapper_rejects_invalid_frequency_and_underflow() {
        assert_eq!(
            QpcClockMapper::from_calibration(1, 0, 0, 0),
            Err(WindowsAudioContractError::InvalidQpcFrequency)
        );
        let mapper = QpcClockMapper::from_calibration(10, 10, 0, 0).unwrap();
        assert_eq!(
            mapper.map_100ns(0),
            Err(WindowsAudioContractError::TimestampOutOfRange)
        );
    }

    #[test]
    fn packet_is_split_on_twenty_millisecond_boundaries() {
        let frames = WASAPI_CHUNK_FRAMES * 2 + 17;
        let samples = vec![0.25; frames as usize * usize::from(WASAPI_CHANNELS)];
        let packet = packet_to_chunks(7, 2_000_000, frames, Some(&samples)).unwrap();

        assert_eq!(packet.chunks.len(), 3);
        assert_eq!(packet.chunks[0].sequence, 7);
        assert_eq!(packet.chunks[0].frame_count, WASAPI_CHUNK_FRAMES);
        assert_eq!(packet.chunks[1].captured_at_ns, 22_000_000);
        assert_eq!(packet.chunks[2].frame_count, 17);
        assert_eq!(packet.next_sequence, 10);
        assert_eq!(packet.end_ns, 42_354_166);
    }

    #[test]
    fn silent_packet_allocates_exact_zeroed_pcm() {
        let packet = packet_to_chunks(0, 0, 480, None).unwrap();
        let chunk = &packet.chunks[0];

        assert_eq!(chunk.samples.len(), 960);
        assert!(chunk.samples.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn packet_rejects_empty_or_mismatched_samples() {
        assert!(matches!(
            packet_to_chunks(0, 0, 0, None),
            Err(WindowsAudioContractError::EmptyPacket)
        ));
        assert!(matches!(
            packet_to_chunks(0, 0, 4, Some(&[0.0; 7])),
            Err(WindowsAudioContractError::SampleLengthMismatch)
        ));
    }

    #[test]
    fn control_timestamp_never_precedes_the_last_pcm_frame() {
        assert_eq!(safe_control_timestamp(300, None), 300);
        assert_eq!(safe_control_timestamp(300, Some(250)), 300);
        assert_eq!(safe_control_timestamp(300, Some(450)), 450);
    }
}
