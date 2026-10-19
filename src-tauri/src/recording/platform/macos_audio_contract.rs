//! ScreenCaptureKit 音频缓冲在进入平台对象前可验证的纯 Rust 合同。
//!
//! 原生层只负责读取 `CMSampleBuffer`/`AudioBufferList`。这里固定 mono 上混、stereo 交错、
//! presentation timestamp 映射与 100 ms 拆块，因此 Linux 也能执行关键时间线测试。

use crate::recording::audio::{AUDIO_SAMPLE_RATE_HZ, MAX_AUDIO_CHUNK_FRAMES};
use thiserror::Error;

const OUTPUT_CHANNELS: usize = 2;
const NANOS_PER_SECOND: u128 = 1_000_000_000;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(super) enum MacAudioContractError {
    #[error("ScreenCaptureKit 音频时间戳无效")]
    InvalidTimestamp,
    #[error("ScreenCaptureKit 音频时间戳超出范围")]
    TimestampOutOfRange,
    #[error("ScreenCaptureKit 音频时间戳早于本次流的原生起点")]
    TimestampRegression,
    #[error("ScreenCaptureKit 音频 packet 与上一 packet 重叠")]
    TimestampOverlap,
    #[error("ScreenCaptureKit 音频 packet 为空")]
    EmptyPacket,
    #[error("ScreenCaptureKit 音频声道数无效")]
    InvalidChannels,
    #[error("ScreenCaptureKit 音频缓冲布局无效")]
    InvalidBufferLayout,
    #[error("ScreenCaptureKit 音频 sample 数量不匹配")]
    SampleLengthMismatch,
    #[error("ScreenCaptureKit 音频包含非有限 sample")]
    NonFiniteSample,
    #[error("ScreenCaptureKit 音频 sample 长度溢出")]
    SampleLengthOverflow,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum NativeFloatPcm<'a> {
    Interleaved(&'a [f32]),
    NonInterleaved(&'a [&'a [f32]]),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct MappedPcmChunk {
    pub captured_at_ns: u64,
    pub frame_count: u32,
    pub samples: Box<[f32]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MappedPacketTiming {
    pub captured_at_ns: u64,
    pub starts_new_segment: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct MacAudioPtsMapper {
    anchor: Option<NativePtsAnchor>,
    last_end_ns: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NativePtsAnchor {
    native_ns: u64,
    session_ns: u64,
}

impl MacAudioPtsMapper {
    pub fn map_packet(
        &mut self,
        pts_value: i64,
        pts_timescale: i32,
        session_now_ns: u64,
        frame_count: u32,
        sample_rate_hz: u32,
    ) -> Result<MappedPacketTiming, MacAudioContractError> {
        if frame_count == 0 {
            return Err(MacAudioContractError::EmptyPacket);
        }
        if sample_rate_hz == 0 {
            return Err(MacAudioContractError::InvalidTimestamp);
        }
        let native_ns = timestamp_to_ns(pts_value, pts_timescale)?;
        let anchor = match self.anchor {
            Some(anchor) => anchor,
            None => {
                let anchor = NativePtsAnchor {
                    native_ns,
                    session_ns: session_now_ns,
                };
                self.anchor = Some(anchor);
                anchor
            }
        };
        let native_delta = native_ns
            .checked_sub(anchor.native_ns)
            .ok_or(MacAudioContractError::TimestampRegression)?;
        let mapped_start = anchor
            .session_ns
            .checked_add(native_delta)
            .ok_or(MacAudioContractError::TimestampOutOfRange)?;
        let starts_new_segment = self
            .last_end_ns
            .is_some_and(|last_end| mapped_start > last_end);
        if self
            .last_end_ns
            .is_some_and(|last_end| mapped_start < last_end)
        {
            return Err(MacAudioContractError::TimestampOverlap);
        }
        let mapped_end = mapped_start
            .checked_add(frames_to_ns_at_rate(frame_count, sample_rate_hz)?)
            .ok_or(MacAudioContractError::TimestampOutOfRange)?;
        self.last_end_ns = Some(mapped_end);
        Ok(MappedPacketTiming {
            captured_at_ns: mapped_start,
            starts_new_segment,
        })
    }

    pub fn control_timestamp_ns(self, session_now_ns: u64) -> u64 {
        self.last_end_ns
            .map_or(session_now_ns, |last_end| last_end.max(session_now_ns))
    }
}

pub(super) fn normalize_float_pcm(
    input: NativeFloatPcm<'_>,
    native_channels: u16,
    frame_count: u32,
) -> Result<Box<[f32]>, MacAudioContractError> {
    if frame_count == 0 {
        return Err(MacAudioContractError::EmptyPacket);
    }
    if !(1..=2).contains(&native_channels) {
        return Err(MacAudioContractError::InvalidChannels);
    }
    let frames =
        usize::try_from(frame_count).map_err(|_| MacAudioContractError::SampleLengthOverflow)?;
    let native_channels = usize::from(native_channels);
    let output_len = frames
        .checked_mul(OUTPUT_CHANNELS)
        .ok_or(MacAudioContractError::SampleLengthOverflow)?;
    let mut output = Vec::with_capacity(output_len);
    match input {
        NativeFloatPcm::Interleaved(samples) => {
            let expected = frames
                .checked_mul(native_channels)
                .ok_or(MacAudioContractError::SampleLengthOverflow)?;
            if samples.len() != expected {
                return Err(MacAudioContractError::SampleLengthMismatch);
            }
            match native_channels {
                1 => {
                    for &sample in samples {
                        output.extend_from_slice(&[sample, sample]);
                    }
                }
                2 => output.extend_from_slice(samples),
                _ => unreachable!("声道数已在外层验证"),
            }
        }
        NativeFloatPcm::NonInterleaved(buffers) => {
            if buffers.len() != native_channels
                || buffers.iter().any(|buffer| buffer.len() != frames)
            {
                return Err(MacAudioContractError::InvalidBufferLayout);
            }
            match native_channels {
                1 => {
                    for &sample in buffers[0] {
                        output.extend_from_slice(&[sample, sample]);
                    }
                }
                2 => {
                    for (&left, &right) in buffers[0].iter().zip(buffers[1]) {
                        output.extend_from_slice(&[left, right]);
                    }
                }
                _ => unreachable!("声道数已在外层验证"),
            }
        }
    }
    if output.iter().any(|sample| !sample.is_finite()) {
        return Err(MacAudioContractError::NonFiniteSample);
    }
    debug_assert_eq!(output.len(), output_len);
    Ok(output.into_boxed_slice())
}

pub(super) fn split_stereo_packet(
    captured_at_ns: u64,
    frame_count: u32,
    samples: &[f32],
) -> Result<Vec<MappedPcmChunk>, MacAudioContractError> {
    if frame_count == 0 {
        return Err(MacAudioContractError::EmptyPacket);
    }
    let expected = usize::try_from(frame_count)
        .ok()
        .and_then(|frames| frames.checked_mul(OUTPUT_CHANNELS))
        .ok_or(MacAudioContractError::SampleLengthOverflow)?;
    if samples.len() != expected {
        return Err(MacAudioContractError::SampleLengthMismatch);
    }
    let mut chunks = Vec::new();
    let mut frame_offset = 0_u32;
    while frame_offset < frame_count {
        let chunk_frames = (frame_count - frame_offset).min(MAX_AUDIO_CHUNK_FRAMES);
        let sample_start = usize::try_from(frame_offset)
            .ok()
            .and_then(|frames| frames.checked_mul(OUTPUT_CHANNELS))
            .ok_or(MacAudioContractError::SampleLengthOverflow)?;
        let sample_len = usize::try_from(chunk_frames)
            .ok()
            .and_then(|frames| frames.checked_mul(OUTPUT_CHANNELS))
            .ok_or(MacAudioContractError::SampleLengthOverflow)?;
        let sample_end = sample_start
            .checked_add(sample_len)
            .ok_or(MacAudioContractError::SampleLengthOverflow)?;
        let timestamp = captured_at_ns
            .checked_add(frames_to_ns(frame_offset)?)
            .ok_or(MacAudioContractError::TimestampOutOfRange)?;
        chunks.push(MappedPcmChunk {
            captured_at_ns: timestamp,
            frame_count: chunk_frames,
            samples: samples[sample_start..sample_end]
                .to_vec()
                .into_boxed_slice(),
        });
        frame_offset = frame_offset
            .checked_add(chunk_frames)
            .ok_or(MacAudioContractError::SampleLengthOverflow)?;
    }
    Ok(chunks)
}

fn timestamp_to_ns(value: i64, timescale: i32) -> Result<u64, MacAudioContractError> {
    let value = u128::try_from(value).map_err(|_| MacAudioContractError::InvalidTimestamp)?;
    let timescale = u128::try_from(timescale)
        .ok()
        .filter(|timescale| *timescale != 0)
        .ok_or(MacAudioContractError::InvalidTimestamp)?;
    let ns = value
        .checked_mul(NANOS_PER_SECOND)
        .ok_or(MacAudioContractError::TimestampOutOfRange)?
        / timescale;
    u64::try_from(ns).map_err(|_| MacAudioContractError::TimestampOutOfRange)
}

fn frames_to_ns(frame_count: u32) -> Result<u64, MacAudioContractError> {
    frames_to_ns_at_rate(frame_count, AUDIO_SAMPLE_RATE_HZ)
}

fn frames_to_ns_at_rate(
    frame_count: u32,
    sample_rate_hz: u32,
) -> Result<u64, MacAudioContractError> {
    if sample_rate_hz == 0 {
        return Err(MacAudioContractError::InvalidTimestamp);
    }
    u64::from(frame_count)
        .checked_mul(1_000_000_000)
        .map(|value| value / u64::from(sample_rate_hz))
        .ok_or(MacAudioContractError::TimestampOutOfRange)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_native_pts_from_one_session_anchor_and_preserves_gaps() {
        let mut mapper = MacAudioPtsMapper::default();
        assert_eq!(
            mapper.map_packet(48_000, 48_000, 7_000, 960, 48_000),
            Ok(MappedPacketTiming {
                captured_at_ns: 7_000,
                starts_new_segment: false,
            })
        );
        assert_eq!(
            mapper.map_packet(50_400, 48_000, 999_999, 882, 44_100),
            Ok(MappedPacketTiming {
                captured_at_ns: 50_007_000,
                starts_new_segment: true,
            })
        );
        assert_eq!(mapper.control_timestamp_ns(1), 70_007_000);
    }

    #[test]
    fn rejects_invalid_regressing_and_overlapping_pts() {
        let mut mapper = MacAudioPtsMapper::default();
        assert_eq!(
            mapper.map_packet(-1, 48_000, 0, 960, 48_000),
            Err(MacAudioContractError::InvalidTimestamp)
        );
        assert_eq!(
            mapper.map_packet(0, 0, 0, 960, 48_000),
            Err(MacAudioContractError::InvalidTimestamp)
        );
        assert_eq!(
            mapper.map_packet(10_000, 48_000, 20, 960, 48_000),
            Ok(MappedPacketTiming {
                captured_at_ns: 20,
                starts_new_segment: false,
            })
        );
        assert_eq!(
            mapper.map_packet(9_999, 48_000, 30, 960, 48_000),
            Err(MacAudioContractError::TimestampRegression)
        );
        assert_eq!(
            mapper.map_packet(10_100, 48_000, 30, 960, 48_000),
            Err(MacAudioContractError::TimestampOverlap)
        );
    }

    #[test]
    fn normalizes_mono_and_both_stereo_layouts() {
        assert_eq!(
            normalize_float_pcm(NativeFloatPcm::Interleaved(&[0.25, -0.5]), 1, 2)
                .unwrap()
                .as_ref(),
            &[0.25, 0.25, -0.5, -0.5]
        );
        assert_eq!(
            normalize_float_pcm(NativeFloatPcm::Interleaved(&[0.1, 0.2, 0.3, 0.4]), 2, 2,)
                .unwrap()
                .as_ref(),
            &[0.1, 0.2, 0.3, 0.4]
        );
        let left = [0.1, 0.3];
        let right = [0.2, 0.4];
        assert_eq!(
            normalize_float_pcm(NativeFloatPcm::NonInterleaved(&[&left, &right]), 2, 2,)
                .unwrap()
                .as_ref(),
            &[0.1, 0.2, 0.3, 0.4]
        );
    }

    #[test]
    fn rejects_bad_pcm_lengths_layouts_and_non_finite_samples() {
        assert_eq!(
            normalize_float_pcm(NativeFloatPcm::Interleaved(&[0.0]), 2, 1),
            Err(MacAudioContractError::SampleLengthMismatch)
        );
        assert_eq!(
            normalize_float_pcm(NativeFloatPcm::NonInterleaved(&[&[0.0]]), 2, 1),
            Err(MacAudioContractError::InvalidBufferLayout)
        );
        assert_eq!(
            normalize_float_pcm(NativeFloatPcm::Interleaved(&[f32::NAN]), 1, 1),
            Err(MacAudioContractError::NonFiniteSample)
        );
    }

    #[test]
    fn splits_packets_at_the_hundred_millisecond_budget() {
        let frames = MAX_AUDIO_CHUNK_FRAMES + 17;
        let samples = vec![0.0; frames as usize * OUTPUT_CHANNELS];
        let chunks = split_stereo_packet(123, frames, &samples).unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].frame_count, MAX_AUDIO_CHUNK_FRAMES);
        assert_eq!(chunks[0].samples.len(), MAX_AUDIO_CHUNK_FRAMES as usize * 2);
        assert_eq!(chunks[1].frame_count, 17);
        assert_eq!(chunks[1].captured_at_ns, 100_000_123);
        assert_eq!(chunks[1].samples.len(), 34);
    }
}
