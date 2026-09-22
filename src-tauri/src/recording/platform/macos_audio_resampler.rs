//! macOS 麦克风原生采样率到 48 kHz stereo 的流式重采样。
//!
//! ScreenCaptureKit 回调只复制原生 packet；本模块运行在录屏音频 worker，使用固定输入块的
//! band-limited sinc，并把 Rubato 的启动延迟与封尾 padding 从产品时间线裁掉。

use super::macos_audio_contract::{split_stereo_packet, MacAudioContractError, MappedPcmChunk};
use crate::recording::audio::AUDIO_SAMPLE_RATE_HZ;
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Async, FixedAsync, Indexing, Resampler, SincInterpolationParameters};
use std::fmt;
use thiserror::Error;

const CHANNELS: usize = 2;
const RESAMPLER_INPUT_FRAMES: usize = 1_024;
const MAX_NATIVE_SAMPLE_RATE_HZ: u32 = 192_000;
const MIN_NATIVE_SAMPLE_RATE_HZ: u32 = 8_000;
const MAX_NATIVE_PACKET_MILLIS: u64 = 200;
const MAX_FLUSH_PASSES: usize = 16;

#[derive(Debug, Error)]
pub(super) enum MacAudioResamplerError {
    #[error("ScreenCaptureKit 麦克风采样率不受支持: {0} Hz")]
    UnsupportedSampleRate(u32),
    #[error("ScreenCaptureKit 麦克风连续音频段内采样率发生变化: {from} -> {to} Hz")]
    SampleRateChanged { from: u32, to: u32 },
    #[error("ScreenCaptureKit 麦克风 packet 超过 200 ms 预算")]
    PacketTooLong,
    #[error(transparent)]
    Contract(#[from] MacAudioContractError),
    #[error("macOS 麦克风重采样初始化失败: {0}")]
    Initialize(String),
    #[error("macOS 麦克风重采样失败: {0}")]
    Process(String),
    #[error("macOS 麦克风重采样时间线溢出")]
    TimelineOverflow,
    #[error("macOS 麦克风重采样封尾未能产生完整输出")]
    IncompleteFlush,
}

/// 把同一连续原生时间段转换成 48 kHz；PTS 空洞由调用方以 `starts_new_segment` 显式切段。
#[derive(Default)]
pub(super) struct MacAudioStreamResampler {
    active_rate_hz: Option<u32>,
    segment: Option<ResampledSegment>,
}

impl fmt::Debug for MacAudioStreamResampler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MacAudioStreamResampler")
            .field("active_rate_hz", &self.active_rate_hz)
            .field("has_resampled_segment", &self.segment.is_some())
            .finish()
    }
}

impl MacAudioStreamResampler {
    pub fn push_packet(
        &mut self,
        captured_at_ns: u64,
        native_rate_hz: u32,
        frame_count: u32,
        samples: &[f32],
        starts_new_segment: bool,
    ) -> Result<Vec<MappedPcmChunk>, MacAudioResamplerError> {
        validate_native_packet(native_rate_hz, frame_count, samples)?;

        let mut chunks = Vec::new();
        if starts_new_segment {
            chunks.extend(self.finish()?);
        } else if let Some(active_rate_hz) = self.active_rate_hz {
            if active_rate_hz != native_rate_hz {
                return Err(MacAudioResamplerError::SampleRateChanged {
                    from: active_rate_hz,
                    to: native_rate_hz,
                });
            }
        }

        self.active_rate_hz = Some(native_rate_hz);
        if native_rate_hz == AUDIO_SAMPLE_RATE_HZ {
            chunks.extend(split_stereo_packet(captured_at_ns, frame_count, samples)?);
            return Ok(chunks);
        }

        if self.segment.is_none() {
            self.segment = Some(ResampledSegment::new(captured_at_ns, native_rate_hz)?);
        }
        let segment = self
            .segment
            .as_mut()
            .ok_or(MacAudioResamplerError::IncompleteFlush)?;
        chunks.extend(segment.push(frame_count, samples)?);
        Ok(chunks)
    }

    /// 正常 Stop 或 PTS 空洞封尾；输出恰好覆盖已接受的原生 frame，不带启动静音或尾部 padding。
    pub fn finish(&mut self) -> Result<Vec<MappedPcmChunk>, MacAudioResamplerError> {
        let chunks = match self.segment.take() {
            Some(mut segment) => segment.finish()?,
            None => Vec::new(),
        };
        self.active_rate_hz = None;
        Ok(chunks)
    }

    /// Pause、Drop 与错误路径不把滤波尾部伪装成已采集音频。
    pub fn discard(&mut self) {
        self.segment = None;
        self.active_rate_hz = None;
    }
}

fn validate_native_packet(
    native_rate_hz: u32,
    frame_count: u32,
    samples: &[f32],
) -> Result<(), MacAudioResamplerError> {
    validate_native_sample_rate(native_rate_hz)?;
    validate_native_frame_count(native_rate_hz, frame_count)?;
    let expected_samples = usize::try_from(frame_count)
        .ok()
        .and_then(|frames| frames.checked_mul(CHANNELS))
        .ok_or(MacAudioContractError::SampleLengthOverflow)?;
    if samples.len() != expected_samples {
        return Err(MacAudioContractError::SampleLengthMismatch.into());
    }
    if samples.iter().any(|sample| !sample.is_finite()) {
        return Err(MacAudioContractError::NonFiniteSample.into());
    }
    Ok(())
}

pub(super) fn validate_native_frame_count(
    native_rate_hz: u32,
    frame_count: u32,
) -> Result<(), MacAudioResamplerError> {
    validate_native_sample_rate(native_rate_hz)?;
    if frame_count == 0 {
        return Err(MacAudioContractError::EmptyPacket.into());
    }
    let max_frames = u64::from(native_rate_hz)
        .checked_mul(MAX_NATIVE_PACKET_MILLIS)
        .ok_or(MacAudioResamplerError::TimelineOverflow)?
        / 1_000;
    if u64::from(frame_count) > max_frames {
        return Err(MacAudioResamplerError::PacketTooLong);
    }
    Ok(())
}

pub(super) fn validate_native_sample_rate(
    native_rate_hz: u32,
) -> Result<(), MacAudioResamplerError> {
    if (MIN_NATIVE_SAMPLE_RATE_HZ..=MAX_NATIVE_SAMPLE_RATE_HZ).contains(&native_rate_hz) {
        Ok(())
    } else {
        Err(MacAudioResamplerError::UnsupportedSampleRate(
            native_rate_hz,
        ))
    }
}

struct ResampledSegment {
    native_rate_hz: u32,
    segment_start_ns: u64,
    resampler: Async<f32>,
    pending_samples: Vec<f32>,
    pending_offset_samples: usize,
    work_input: Vec<f32>,
    ready_samples: Vec<f32>,
    ready_offset_samples: usize,
    delay_frames_remaining: usize,
    total_input_frames: u64,
    processed_input_frames: u64,
    emitted_output_frames: u64,
}

impl ResampledSegment {
    fn new(segment_start_ns: u64, native_rate_hz: u32) -> Result<Self, MacAudioResamplerError> {
        let ratio = f64::from(AUDIO_SAMPLE_RATE_HZ) / f64::from(native_rate_hz);
        let parameters = SincInterpolationParameters::default();
        let resampler = Async::<f32>::new_sinc(
            ratio,
            1.0,
            &parameters,
            RESAMPLER_INPUT_FRAMES,
            CHANNELS,
            FixedAsync::Input,
        )
        .map_err(|error| MacAudioResamplerError::Initialize(error.to_string()))?;
        let delay_frames_remaining = resampler.output_delay();
        Ok(Self {
            native_rate_hz,
            segment_start_ns,
            resampler,
            pending_samples: Vec::new(),
            pending_offset_samples: 0,
            work_input: vec![0.0; RESAMPLER_INPUT_FRAMES * CHANNELS],
            ready_samples: Vec::new(),
            ready_offset_samples: 0,
            delay_frames_remaining,
            total_input_frames: 0,
            processed_input_frames: 0,
            emitted_output_frames: 0,
        })
    }

    fn push(
        &mut self,
        frame_count: u32,
        samples: &[f32],
    ) -> Result<Vec<MappedPcmChunk>, MacAudioResamplerError> {
        self.total_input_frames = self
            .total_input_frames
            .checked_add(u64::from(frame_count))
            .ok_or(MacAudioResamplerError::TimelineOverflow)?;
        self.pending_samples.extend_from_slice(samples);

        let mut chunks = Vec::new();
        while self.pending_frames() >= RESAMPLER_INPUT_FRAMES {
            self.process_next_block(RESAMPLER_INPUT_FRAMES)?;
            self.pending_offset_samples = self
                .pending_offset_samples
                .checked_add(RESAMPLER_INPUT_FRAMES * CHANNELS)
                .ok_or(MacAudioResamplerError::TimelineOverflow)?;
            let safe_output_frames =
                resampled_frames_floor(self.processed_input_frames, self.native_rate_hz)?;
            chunks.extend(self.drain_ready(safe_output_frames)?);
        }
        self.compact_pending();
        Ok(chunks)
    }

    fn finish(&mut self) -> Result<Vec<MappedPcmChunk>, MacAudioResamplerError> {
        let mut chunks = Vec::new();
        let pending_frames = self.pending_frames();
        if pending_frames > 0 {
            self.process_next_block(pending_frames)?;
            self.pending_offset_samples = self.pending_samples.len();
            self.compact_pending();
        }

        let target_frames = resampled_frames_ceil(self.total_input_frames, self.native_rate_hz)?;
        for _ in 0..MAX_FLUSH_PASSES {
            chunks.extend(self.drain_ready(target_frames)?);
            if self.emitted_output_frames == target_frames {
                self.ready_samples.clear();
                self.ready_offset_samples = 0;
                return Ok(chunks);
            }
            self.process_next_block(0)?;
        }
        Err(MacAudioResamplerError::IncompleteFlush)
    }

    fn pending_frames(&self) -> usize {
        self.pending_samples
            .len()
            .saturating_sub(self.pending_offset_samples)
            / CHANNELS
    }

    fn compact_pending(&mut self) {
        if self.pending_offset_samples == 0 {
            return;
        }
        self.pending_samples.drain(..self.pending_offset_samples);
        self.pending_offset_samples = 0;
    }

    fn process_next_block(
        &mut self,
        actual_input_frames: usize,
    ) -> Result<(), MacAudioResamplerError> {
        self.work_input.fill(0.0);
        let actual_samples = actual_input_frames
            .checked_mul(CHANNELS)
            .ok_or(MacAudioResamplerError::TimelineOverflow)?;
        if actual_samples > 0 {
            let source_end = self
                .pending_offset_samples
                .checked_add(actual_samples)
                .ok_or(MacAudioResamplerError::TimelineOverflow)?;
            self.work_input[..actual_samples].copy_from_slice(
                self.pending_samples
                    .get(self.pending_offset_samples..source_end)
                    .ok_or(MacAudioContractError::SampleLengthMismatch)?,
            );
        }
        let input = InterleavedSlice::new(&self.work_input, CHANNELS, RESAMPLER_INPUT_FRAMES)
            .map_err(|error| MacAudioResamplerError::Process(error.to_string()))?;
        let indexing = (actual_input_frames != RESAMPLER_INPUT_FRAMES).then(|| Indexing {
            partial_len: Some(actual_input_frames),
            ..Indexing::new()
        });
        let output = self
            .resampler
            .process(&input, indexing.as_ref())
            .map_err(|error| MacAudioResamplerError::Process(error.to_string()))?;
        self.processed_input_frames = self
            .processed_input_frames
            .checked_add(
                u64::try_from(actual_input_frames)
                    .map_err(|_| MacAudioResamplerError::TimelineOverflow)?,
            )
            .ok_or(MacAudioResamplerError::TimelineOverflow)?;
        let output_samples = output.take_data();
        let output_frames = output_samples.len() / CHANNELS;
        let trim_frames = self.delay_frames_remaining.min(output_frames);
        self.delay_frames_remaining -= trim_frames;
        let trim_samples = trim_frames * CHANNELS;
        self.ready_samples
            .extend_from_slice(&output_samples[trim_samples..]);
        Ok(())
    }

    fn drain_ready(
        &mut self,
        allowed_total_frames: u64,
    ) -> Result<Vec<MappedPcmChunk>, MacAudioResamplerError> {
        let allowed_frames = allowed_total_frames
            .checked_sub(self.emitted_output_frames)
            .ok_or(MacAudioResamplerError::TimelineOverflow)?;
        let ready_frames = self
            .ready_samples
            .len()
            .saturating_sub(self.ready_offset_samples)
            / CHANNELS;
        let take_frames = ready_frames.min(usize::try_from(allowed_frames).unwrap_or(usize::MAX));
        if take_frames == 0 {
            return Ok(Vec::new());
        }
        let take_samples = take_frames
            .checked_mul(CHANNELS)
            .ok_or(MacAudioResamplerError::TimelineOverflow)?;
        let end = self
            .ready_offset_samples
            .checked_add(take_samples)
            .ok_or(MacAudioResamplerError::TimelineOverflow)?;
        let timestamp = self
            .segment_start_ns
            .checked_add(output_frames_to_ns(self.emitted_output_frames)?)
            .ok_or(MacAudioResamplerError::TimelineOverflow)?;
        let frame_count =
            u32::try_from(take_frames).map_err(|_| MacAudioResamplerError::TimelineOverflow)?;
        let chunks = split_stereo_packet(
            timestamp,
            frame_count,
            self.ready_samples
                .get(self.ready_offset_samples..end)
                .ok_or(MacAudioContractError::SampleLengthMismatch)?,
        )?;
        self.ready_offset_samples = end;
        self.emitted_output_frames = self
            .emitted_output_frames
            .checked_add(
                u64::try_from(take_frames).map_err(|_| MacAudioResamplerError::TimelineOverflow)?,
            )
            .ok_or(MacAudioResamplerError::TimelineOverflow)?;
        if self.ready_offset_samples == self.ready_samples.len() {
            self.ready_samples.clear();
            self.ready_offset_samples = 0;
        }
        Ok(chunks)
    }
}

fn resampled_frames_floor(
    input_frames: u64,
    input_rate_hz: u32,
) -> Result<u64, MacAudioResamplerError> {
    let numerator = u128::from(input_frames)
        .checked_mul(u128::from(AUDIO_SAMPLE_RATE_HZ))
        .ok_or(MacAudioResamplerError::TimelineOverflow)?;
    u64::try_from(numerator / u128::from(input_rate_hz))
        .map_err(|_| MacAudioResamplerError::TimelineOverflow)
}

fn resampled_frames_ceil(
    input_frames: u64,
    input_rate_hz: u32,
) -> Result<u64, MacAudioResamplerError> {
    let denominator = u128::from(input_rate_hz);
    let numerator = u128::from(input_frames)
        .checked_mul(u128::from(AUDIO_SAMPLE_RATE_HZ))
        .and_then(|value| value.checked_add(denominator - 1))
        .ok_or(MacAudioResamplerError::TimelineOverflow)?;
    u64::try_from(numerator / denominator).map_err(|_| MacAudioResamplerError::TimelineOverflow)
}

fn output_frames_to_ns(frame_count: u64) -> Result<u64, MacAudioResamplerError> {
    let ns = u128::from(frame_count)
        .checked_mul(1_000_000_000)
        .ok_or(MacAudioResamplerError::TimelineOverflow)?
        / u128::from(AUDIO_SAMPLE_RATE_HZ);
    u64::try_from(ns).map_err(|_| MacAudioResamplerError::TimelineOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn sine(rate: u32, frames: usize, frequency_hz: f32) -> Vec<f32> {
        (0..frames)
            .flat_map(|frame| {
                let sample = (TAU * frequency_hz * frame as f32 / rate as f32).sin();
                [sample, sample]
            })
            .collect()
    }

    fn run_packetized(rate: u32, samples: &[f32], packet_frames: usize) -> Vec<f32> {
        let mut stream = MacAudioStreamResampler::default();
        let total_frames = samples.len() / CHANNELS;
        let mut output = Vec::new();
        let mut offset_frames = 0_usize;
        while offset_frames < total_frames {
            let frames = (total_frames - offset_frames).min(packet_frames);
            let start = offset_frames * CHANNELS;
            let end = start + frames * CHANNELS;
            let timestamp = u64::try_from(offset_frames).unwrap() * 1_000_000_000 / u64::from(rate);
            let chunks = stream
                .push_packet(
                    timestamp,
                    rate,
                    u32::try_from(frames).unwrap(),
                    &samples[start..end],
                    false,
                )
                .unwrap();
            output.extend(
                chunks
                    .into_iter()
                    .flat_map(|chunk| chunk.samples.into_vec()),
            );
            offset_frames += frames;
        }
        output.extend(
            stream
                .finish()
                .unwrap()
                .into_iter()
                .flat_map(|chunk| chunk.samples.into_vec()),
        );
        output
    }

    fn rms(samples: &[f32]) -> f32 {
        let mono = samples.iter().step_by(CHANNELS);
        let count = mono.clone().count();
        (mono.map(|sample| sample * sample).sum::<f32>() / count as f32).sqrt()
    }

    #[test]
    fn bypasses_48khz_without_changing_samples() {
        let input = sine(48_000, 4_800, 1_000.0);
        assert_eq!(run_packetized(48_000, &input, 777), input);
    }

    #[test]
    fn resamples_common_rates_to_exact_48khz_length() {
        for rate in [44_100, 88_200, 96_000] {
            let input = sine(rate, rate as usize, 1_000.0);
            let output = run_packetized(rate, &input, 733);
            assert_eq!(output.len(), 48_000 * CHANNELS, "rate={rate}");
            let level = rms(&output[4_800 * CHANNELS..43_200 * CHANNELS]);
            assert!((0.65..=0.75).contains(&level), "rate={rate}, rms={level}");
        }
    }

    #[test]
    fn output_chunks_follow_the_exact_48khz_timeline_and_budget() {
        let rate = 96_000;
        let input = sine(rate, rate as usize, 1_000.0);
        let mut stream = MacAudioStreamResampler::default();
        let mut chunks = Vec::new();
        for (packet_index, packet) in input.chunks(733 * CHANNELS).enumerate() {
            let frames = packet.len() / CHANNELS;
            chunks.extend(
                stream
                    .push_packet(
                        123 + packet_index as u64 * 733 * 1_000_000_000 / u64::from(rate),
                        rate,
                        u32::try_from(frames).unwrap(),
                        packet,
                        false,
                    )
                    .unwrap(),
            );
        }
        chunks.extend(stream.finish().unwrap());

        let mut emitted_frames = 0_u64;
        for chunk in &chunks {
            assert_eq!(
                chunk.captured_at_ns,
                123 + emitted_frames * 1_000_000_000 / u64::from(AUDIO_SAMPLE_RATE_HZ)
            );
            assert!(chunk.frame_count <= 4_800);
            emitted_frames += u64::from(chunk.frame_count);
        }
        assert_eq!(emitted_frames, 48_000);
    }

    #[test]
    fn flushes_short_segments_to_the_exact_duration() {
        for frames in [1_usize, 37, 500] {
            let input = sine(44_100, frames, 1_000.0);
            let output = run_packetized(44_100, &input, frames);
            let expected_frames = (frames * 48_000).div_ceil(44_100);
            assert_eq!(output.len(), expected_frames * CHANNELS, "frames={frames}");
            assert!(output.iter().all(|sample| sample.is_finite()));
        }
    }

    #[test]
    fn packet_boundaries_do_not_change_resampled_output() {
        let input = sine(44_100, 44_100, 3_217.0);
        let small = run_packetized(44_100, &input, 137);
        let large = run_packetized(44_100, &input, 4_000);
        assert_eq!(small, large);
    }

    #[test]
    fn downsampling_suppresses_content_above_output_nyquist() {
        let passband = run_packetized(96_000, &sine(96_000, 96_000, 1_000.0), 960);
        let stopband = run_packetized(96_000, &sine(96_000, 96_000, 30_000.0), 960);
        let middle = 4_800 * CHANNELS..43_200 * CHANNELS;
        let passband_rms = rms(&passband[middle.clone()]);
        let stopband_rms = rms(&stopband[middle]);
        assert!(passband_rms > 0.65);
        assert!(stopband_rms < passband_rms * 0.01, "{stopband_rms}");
    }

    #[test]
    fn rejects_invalid_packets_and_rate_changes_inside_a_segment() {
        let mut stream = MacAudioStreamResampler::default();
        assert!(matches!(
            stream.push_packet(0, 7_999, 1, &[0.0, 0.0], false),
            Err(MacAudioResamplerError::UnsupportedSampleRate(7_999))
        ));
        assert!(matches!(
            validate_native_frame_count(44_100, 8_821),
            Err(MacAudioResamplerError::PacketTooLong)
        ));
        assert!(matches!(
            stream.push_packet(0, 44_100, 1, &[f32::NAN, 0.0], false),
            Err(MacAudioResamplerError::Contract(
                MacAudioContractError::NonFiniteSample
            ))
        ));
        stream
            .push_packet(0, 44_100, 1, &[0.0, 0.0], false)
            .unwrap();
        assert!(matches!(
            stream.push_packet(1, 48_000, 1, &[0.0, 0.0], false),
            Err(MacAudioResamplerError::SampleRateChanged {
                from: 44_100,
                to: 48_000
            })
        ));
    }

    #[test]
    fn discard_prevents_old_tail_from_leaking_after_resume() {
        let mut stream = MacAudioStreamResampler::default();
        let first = sine(44_100, 500, 1_000.0);
        stream.push_packet(0, 44_100, 500, &first, false).unwrap();
        stream.discard();
        let silence = vec![0.0; 2_000 * CHANNELS];
        let mut output = stream
            .push_packet(9_000_000, 44_100, 2_000, &silence, false)
            .unwrap();
        output.extend(stream.finish().unwrap());
        assert!(output
            .iter()
            .flat_map(|chunk| chunk.samples.iter())
            .all(|sample| *sample == 0.0));
    }
}
