//! PX-REC-AUDIO-01 的单音轨 PCM 与时间线合同。
//!
//! 平台适配器必须先把音频归一化为 48 kHz、mono/stereo、交错 `f32`。这里不接设备 API、
//! 编码器或 WebM；只固定块校验、显式会话起点、暂停扣除和有界背压语义。

use std::collections::VecDeque;
use std::sync::Mutex;
use thiserror::Error;

pub(super) const AUDIO_SAMPLE_RATE_HZ: u32 = 48_000;
pub(super) const DEFAULT_OPUS_FRAME_MS: u32 = 20;
const MAX_AUDIO_CHANNELS: u16 = 2;
const MAX_AUDIO_CHUNK_FRAMES: u32 = AUDIO_SAMPLE_RATE_HZ / 10;
const MAX_QUEUED_AUDIO_FRAMES: u64 = AUDIO_SAMPLE_RATE_HZ as u64;
const MAX_QUEUED_AUDIO_BYTES: usize =
    AUDIO_SAMPLE_RATE_HZ as usize * MAX_AUDIO_CHANNELS as usize * std::mem::size_of::<f32>();
const NANOS_PER_SECOND: u64 = 1_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AudioFormat {
    pub sample_rate_hz: u32,
    pub channels: u16,
}

impl AudioFormat {
    pub const fn normalized(channels: u16) -> Self {
        Self {
            sample_rate_hz: AUDIO_SAMPLE_RATE_HZ,
            channels,
        }
    }

    fn validate(self) -> Result<(), AudioPipelineError> {
        if self.sample_rate_hz != AUDIO_SAMPLE_RATE_HZ {
            return Err(AudioPipelineError::UnsupportedSampleRate);
        }
        if !(1..=MAX_AUDIO_CHANNELS).contains(&self.channels) {
            return Err(AudioPipelineError::UnsupportedChannels);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct CapturedAudioChunk {
    pub sequence: u64,
    /// 本块第一个 PCM frame 映射到会话单调时钟后的时间戳，不能使用回调到达时间代替原生 PTS。
    pub captured_at_ns: u64,
    pub format: AudioFormat,
    pub frame_count: u32,
    /// 交错 mono/stereo PCM。定长切片保证预算等于实际所有权，不隐藏备用 capacity。
    pub samples: Box<[f32]>,
}

impl CapturedAudioChunk {
    fn validate(&self) -> Result<AudioChunkSpec, AudioPipelineError> {
        self.format.validate()?;
        if self.frame_count == 0 {
            return Err(AudioPipelineError::EmptyChunk);
        }
        if self.frame_count > MAX_AUDIO_CHUNK_FRAMES {
            return Err(AudioPipelineError::ChunkTooLong);
        }
        let expected_samples = usize::try_from(self.frame_count)
            .ok()
            .and_then(|frames| frames.checked_mul(usize::from(self.format.channels)))
            .ok_or(AudioPipelineError::SampleLengthOverflow)?;
        if self.samples.len() != expected_samples {
            return Err(AudioPipelineError::SampleLengthMismatch);
        }
        if self.samples.iter().any(|sample| !sample.is_finite()) {
            return Err(AudioPipelineError::NonFiniteSample);
        }
        let byte_length = expected_samples
            .checked_mul(std::mem::size_of::<f32>())
            .ok_or(AudioPipelineError::SampleLengthOverflow)?;
        Ok(AudioChunkSpec {
            frame_count: self.frame_count,
            byte_length,
            duration_ns: frames_to_ns(self.frame_count)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AudioChunkSpec {
    frame_count: u32,
    byte_length: usize,
    duration_ns: u64,
}

#[derive(Debug)]
pub(super) struct QueuedAudioChunk {
    pub chunk: CapturedAudioChunk,
    pub presentation_at_ns: u64,
    pub duration_ns: u64,
    /// 首块时表示相对会话起点的延迟；后续块表示相对上一块末尾的真实空洞。
    pub gap_before_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AudioPushOutcome {
    Queued {
        presentation_at_ns: u64,
        duration_ns: u64,
        gap_before_ns: u64,
    },
    IgnoredWhilePaused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AudioPipelineStats {
    pub queued_chunks: usize,
    pub queued_frames: u64,
    pub queued_bytes: usize,
    pub accepted_chunks: u64,
    pub ignored_while_paused: u64,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(super) enum AudioPipelineError {
    #[error("录屏音频只接受 48 kHz PCM")]
    UnsupportedSampleRate,
    #[error("录屏音频只接受单声道或双声道 PCM")]
    UnsupportedChannels,
    #[error("录屏音频块不能为空")]
    EmptyChunk,
    #[error("录屏音频单块不能超过 100 ms")]
    ChunkTooLong,
    #[error("录屏音频样本数量溢出")]
    SampleLengthOverflow,
    #[error("录屏音频样本数量与 frame/channel 不一致")]
    SampleLengthMismatch,
    #[error("录屏音频包含 NaN 或无穷值")]
    NonFiniteSample,
    #[error("录屏音频格式在会话中发生变化")]
    FormatChanged,
    #[error("录屏音频块序号必须严格递增")]
    SequenceNotIncreasing,
    #[error("录屏音频时间戳早于会话起点")]
    SourceBeforeOrigin,
    #[error("录屏音频采集时间戳必须严格递增")]
    SourceTimestampNotIncreasing,
    #[error("录屏音频呈现区间不能重叠")]
    PresentationOverlap,
    #[error("录屏音频时间线计算溢出")]
    TimelineOverflow,
    #[error("录屏音频暂停时间戳无效")]
    InvalidPauseTimestamp,
    #[error("录屏音频恢复时间戳无效")]
    InvalidResumeTimestamp,
    #[error("录屏音频已经暂停")]
    AlreadyPaused,
    #[error("录屏音频尚未暂停")]
    NotPaused,
    #[error("录屏音频停止时间早于已接受样本末尾")]
    FinishBeforeBufferedAudioEnd,
    #[error("录屏音频队列已达到一秒 PCM 上限")]
    Backpressure,
    #[error("录屏音频队列已经正常结束")]
    Closed,
    #[error("录屏音频队列已经异常中止")]
    Aborted,
    #[error("录屏音频队列锁已损坏")]
    Poisoned,
}

#[derive(Debug, Clone)]
struct AudioTimeline {
    origin_ns: u64,
    last_sequence: Option<u64>,
    last_source_ns: Option<u64>,
    last_presentation_end_ns: Option<u64>,
    paused_at_ns: Option<u64>,
    accumulated_pause_ns: u64,
}

impl AudioTimeline {
    const fn new(origin_ns: u64) -> Self {
        Self {
            origin_ns,
            last_sequence: None,
            last_source_ns: None,
            last_presentation_end_ns: None,
            paused_at_ns: None,
            accumulated_pause_ns: 0,
        }
    }

    fn map_chunk(
        &mut self,
        chunk: CapturedAudioChunk,
        spec: AudioChunkSpec,
    ) -> Result<MappedAudioChunk, AudioPipelineError> {
        if self.paused_at_ns.is_some() {
            return Ok(MappedAudioChunk::Ignored(chunk));
        }
        if self
            .last_sequence
            .is_some_and(|sequence| chunk.sequence <= sequence)
        {
            return Err(AudioPipelineError::SequenceNotIncreasing);
        }
        if chunk.captured_at_ns < self.origin_ns {
            return Err(AudioPipelineError::SourceBeforeOrigin);
        }
        if self
            .last_source_ns
            .is_some_and(|timestamp| chunk.captured_at_ns <= timestamp)
        {
            return Err(AudioPipelineError::SourceTimestampNotIncreasing);
        }
        let presentation_at_ns = chunk
            .captured_at_ns
            .checked_sub(self.origin_ns)
            .and_then(|elapsed| elapsed.checked_sub(self.accumulated_pause_ns))
            .ok_or(AudioPipelineError::TimelineOverflow)?;
        let previous_end = self.last_presentation_end_ns.unwrap_or(0);
        if presentation_at_ns < previous_end {
            return Err(AudioPipelineError::PresentationOverlap);
        }
        let presentation_end_ns = presentation_at_ns
            .checked_add(spec.duration_ns)
            .ok_or(AudioPipelineError::TimelineOverflow)?;
        let gap_before_ns = presentation_at_ns - previous_end;
        self.last_sequence = Some(chunk.sequence);
        self.last_source_ns = Some(chunk.captured_at_ns);
        self.last_presentation_end_ns = Some(presentation_end_ns);
        Ok(MappedAudioChunk::Queued(QueuedAudioChunk {
            chunk,
            presentation_at_ns,
            duration_ns: spec.duration_ns,
            gap_before_ns,
        }))
    }

    fn pause(&mut self, captured_at_ns: u64) -> Result<(), AudioPipelineError> {
        if self.paused_at_ns.is_some() {
            return Err(AudioPipelineError::AlreadyPaused);
        }
        if captured_at_ns < self.origin_ns
            || self
                .last_source_ns
                .is_some_and(|timestamp| captured_at_ns < timestamp)
        {
            return Err(AudioPipelineError::InvalidPauseTimestamp);
        }
        let presentation_at_ns = captured_at_ns
            .checked_sub(self.origin_ns)
            .and_then(|elapsed| elapsed.checked_sub(self.accumulated_pause_ns))
            .ok_or(AudioPipelineError::TimelineOverflow)?;
        if self
            .last_presentation_end_ns
            .is_some_and(|end| presentation_at_ns < end)
        {
            return Err(AudioPipelineError::InvalidPauseTimestamp);
        }
        self.last_source_ns = Some(captured_at_ns);
        self.paused_at_ns = Some(captured_at_ns);
        Ok(())
    }

    fn resume(&mut self, captured_at_ns: u64) -> Result<(), AudioPipelineError> {
        let paused_at_ns = self.paused_at_ns.ok_or(AudioPipelineError::NotPaused)?;
        if captured_at_ns <= paused_at_ns {
            return Err(AudioPipelineError::InvalidResumeTimestamp);
        }
        self.accumulated_pause_ns = self
            .accumulated_pause_ns
            .checked_add(captured_at_ns - paused_at_ns)
            .ok_or(AudioPipelineError::TimelineOverflow)?;
        self.last_source_ns = Some(captured_at_ns);
        self.paused_at_ns = None;
        Ok(())
    }

    fn finish(&mut self, captured_at_ns: u64) -> Result<u64, AudioPipelineError> {
        let minimum_source = self.last_source_ns.unwrap_or(self.origin_ns);
        if captured_at_ns < minimum_source {
            return Err(AudioPipelineError::SourceTimestampNotIncreasing);
        }
        let accumulated_pause_ns = match self.paused_at_ns {
            Some(paused_at_ns) => self
                .accumulated_pause_ns
                .checked_add(
                    captured_at_ns
                        .checked_sub(paused_at_ns)
                        .ok_or(AudioPipelineError::SourceTimestampNotIncreasing)?,
                )
                .ok_or(AudioPipelineError::TimelineOverflow)?,
            None => self.accumulated_pause_ns,
        };
        let duration_ns = captured_at_ns
            .checked_sub(self.origin_ns)
            .and_then(|elapsed| elapsed.checked_sub(accumulated_pause_ns))
            .ok_or(AudioPipelineError::TimelineOverflow)?;
        if self
            .last_presentation_end_ns
            .is_some_and(|end| duration_ns < end)
        {
            return Err(AudioPipelineError::FinishBeforeBufferedAudioEnd);
        }
        self.accumulated_pause_ns = accumulated_pause_ns;
        self.paused_at_ns = None;
        self.last_source_ns = Some(captured_at_ns);
        Ok(duration_ns)
    }
}

enum MappedAudioChunk {
    Queued(QueuedAudioChunk),
    Ignored(CapturedAudioChunk),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AudioTerminal {
    Open,
    Finished { duration_ns: u64 },
    Aborted,
}

#[derive(Debug)]
struct AudioPipelineState {
    format: Option<AudioFormat>,
    timeline: AudioTimeline,
    chunks: VecDeque<QueuedAudioChunk>,
    queued_frames: u64,
    queued_bytes: usize,
    accepted_chunks: u64,
    ignored_while_paused: u64,
    terminal: AudioTerminal,
}

#[derive(Debug)]
pub(super) struct AudioPipeline {
    state: Mutex<AudioPipelineState>,
}

impl AudioPipeline {
    pub fn new(origin_ns: u64) -> Self {
        Self {
            state: Mutex::new(AudioPipelineState {
                format: None,
                timeline: AudioTimeline::new(origin_ns),
                chunks: VecDeque::new(),
                queued_frames: 0,
                queued_bytes: 0,
                accepted_chunks: 0,
                ignored_while_paused: 0,
                terminal: AudioTerminal::Open,
            }),
        }
    }

    pub fn push(&self, chunk: CapturedAudioChunk) -> Result<AudioPushOutcome, AudioPipelineError> {
        let spec = chunk.validate()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| AudioPipelineError::Poisoned)?;
        ensure_open(state.terminal)?;
        if state.format.is_some_and(|format| format != chunk.format) {
            return Err(AudioPipelineError::FormatChanged);
        }

        // 先在副本中映射，只有队列预算也通过后才提交时间线；背压失败可安全重试同一块。
        let mut candidate_timeline = state.timeline.clone();
        match candidate_timeline.map_chunk(chunk, spec)? {
            MappedAudioChunk::Ignored(_chunk) => {
                state.ignored_while_paused = state.ignored_while_paused.saturating_add(1);
                Ok(AudioPushOutcome::IgnoredWhilePaused)
            }
            MappedAudioChunk::Queued(queued) => {
                let next_frames = state
                    .queued_frames
                    .checked_add(u64::from(spec.frame_count))
                    .ok_or(AudioPipelineError::Backpressure)?;
                let next_bytes = state
                    .queued_bytes
                    .checked_add(spec.byte_length)
                    .ok_or(AudioPipelineError::Backpressure)?;
                if next_frames > MAX_QUEUED_AUDIO_FRAMES || next_bytes > MAX_QUEUED_AUDIO_BYTES {
                    return Err(AudioPipelineError::Backpressure);
                }
                let outcome = AudioPushOutcome::Queued {
                    presentation_at_ns: queued.presentation_at_ns,
                    duration_ns: queued.duration_ns,
                    gap_before_ns: queued.gap_before_ns,
                };
                state.format.get_or_insert(queued.chunk.format);
                state.timeline = candidate_timeline;
                state.queued_frames = next_frames;
                state.queued_bytes = next_bytes;
                state.accepted_chunks = state.accepted_chunks.saturating_add(1);
                state.chunks.push_back(queued);
                Ok(outcome)
            }
        }
    }

    pub fn pop(&self) -> Result<Option<QueuedAudioChunk>, AudioPipelineError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AudioPipelineError::Poisoned)?;
        let chunk = state.chunks.pop_front();
        if let Some(chunk) = &chunk {
            state.queued_frames -= u64::from(chunk.chunk.frame_count);
            state.queued_bytes -= chunk.chunk.samples.len() * std::mem::size_of::<f32>();
        }
        Ok(chunk)
    }

    pub fn pause(&self, captured_at_ns: u64) -> Result<(), AudioPipelineError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AudioPipelineError::Poisoned)?;
        ensure_open(state.terminal)?;
        state.timeline.pause(captured_at_ns)
    }

    pub fn resume(&self, captured_at_ns: u64) -> Result<(), AudioPipelineError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AudioPipelineError::Poisoned)?;
        ensure_open(state.terminal)?;
        state.timeline.resume(captured_at_ns)
    }

    pub fn finish(&self, captured_at_ns: u64) -> Result<u64, AudioPipelineError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AudioPipelineError::Poisoned)?;
        ensure_open(state.terminal)?;
        let duration_ns = state.timeline.finish(captured_at_ns)?;
        state.terminal = AudioTerminal::Finished { duration_ns };
        Ok(duration_ns)
    }

    pub fn abort(&self) -> Result<(), AudioPipelineError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AudioPipelineError::Poisoned)?;
        if state.terminal == AudioTerminal::Open {
            state.terminal = AudioTerminal::Aborted;
        }
        Ok(())
    }

    pub fn stats(&self) -> Result<AudioPipelineStats, AudioPipelineError> {
        let state = self
            .state
            .lock()
            .map_err(|_| AudioPipelineError::Poisoned)?;
        Ok(AudioPipelineStats {
            queued_chunks: state.chunks.len(),
            queued_frames: state.queued_frames,
            queued_bytes: state.queued_bytes,
            accepted_chunks: state.accepted_chunks,
            ignored_while_paused: state.ignored_while_paused,
        })
    }
}

fn ensure_open(terminal: AudioTerminal) -> Result<(), AudioPipelineError> {
    match terminal {
        AudioTerminal::Open => Ok(()),
        AudioTerminal::Finished { .. } => Err(AudioPipelineError::Closed),
        AudioTerminal::Aborted => Err(AudioPipelineError::Aborted),
    }
}

pub(super) fn frames_to_ns(frames: u32) -> Result<u64, AudioPipelineError> {
    u64::from(frames)
        .checked_mul(NANOS_PER_SECOND)
        .and_then(|value| value.checked_div(u64::from(AUDIO_SAMPLE_RATE_HZ)))
        .ok_or(AudioPipelineError::TimelineOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(sequence: u64, captured_at_ns: u64, frame_count: u32) -> CapturedAudioChunk {
        let format = AudioFormat::normalized(2);
        CapturedAudioChunk {
            sequence,
            captured_at_ns,
            format,
            frame_count,
            samples: vec![0.25; frame_count as usize * 2].into_boxed_slice(),
        }
    }

    #[test]
    fn validates_normalized_pcm_shape_finite_samples_and_chunk_limit() {
        assert_eq!(DEFAULT_OPUS_FRAME_MS, 20);
        assert!(chunk(0, 0, 960).validate().is_ok());

        let mono = CapturedAudioChunk {
            sequence: 0,
            captured_at_ns: 0,
            format: AudioFormat::normalized(1),
            frame_count: 960,
            samples: vec![0.0; 960].into_boxed_slice(),
        };
        assert!(mono.validate().is_ok());

        let mut bad_rate = chunk(0, 0, 960);
        bad_rate.format.sample_rate_hz = 44_100;
        assert_eq!(
            bad_rate.validate(),
            Err(AudioPipelineError::UnsupportedSampleRate)
        );

        let mut bad_channels = chunk(0, 0, 960);
        bad_channels.format.channels = 3;
        assert_eq!(
            bad_channels.validate(),
            Err(AudioPipelineError::UnsupportedChannels)
        );

        let mut wrong_length = chunk(0, 0, 960);
        wrong_length.samples = vec![0.0; 1_919].into_boxed_slice();
        assert_eq!(
            wrong_length.validate(),
            Err(AudioPipelineError::SampleLengthMismatch)
        );

        let mut non_finite = chunk(0, 0, 960);
        non_finite.samples[100] = f32::NAN;
        assert_eq!(
            non_finite.validate(),
            Err(AudioPipelineError::NonFiniteSample)
        );
        assert_eq!(
            chunk(0, 0, 0).validate(),
            Err(AudioPipelineError::EmptyChunk)
        );
        assert_eq!(
            chunk(0, 0, MAX_AUDIO_CHUNK_FRAMES + 1).validate(),
            Err(AudioPipelineError::ChunkTooLong)
        );
    }

    #[test]
    fn explicit_origin_preserves_startup_delay_and_real_gaps() {
        let origin = 1_000_000_000;
        let pipeline = AudioPipeline::new(origin);
        assert_eq!(
            pipeline.push(chunk(0, origin + 5_000_000, 960)).unwrap(),
            AudioPushOutcome::Queued {
                presentation_at_ns: 5_000_000,
                duration_ns: 20_000_000,
                gap_before_ns: 5_000_000,
            }
        );
        assert_eq!(
            pipeline.push(chunk(1, origin + 35_000_000, 960)).unwrap(),
            AudioPushOutcome::Queued {
                presentation_at_ns: 35_000_000,
                duration_ns: 20_000_000,
                gap_before_ns: 10_000_000,
            }
        );
    }

    #[test]
    fn rejects_overlap_duplicate_sequence_and_source_regression() {
        let pipeline = AudioPipeline::new(1_000_000_000);
        pipeline
            .push(chunk(0, 1_010_000_000, 960))
            .expect("首块应成功");
        assert_eq!(
            pipeline.push(chunk(1, 1_020_000_000, 960)),
            Err(AudioPipelineError::PresentationOverlap)
        );
        assert_eq!(
            pipeline.push(chunk(0, 1_030_000_000, 960)),
            Err(AudioPipelineError::SequenceNotIncreasing)
        );
        assert_eq!(
            pipeline.push(chunk(1, 1_005_000_000, 960)),
            Err(AudioPipelineError::SourceTimestampNotIncreasing)
        );
        let before_origin = AudioPipeline::new(1_000_000_000);
        assert_eq!(
            before_origin.push(chunk(0, 999_999_999, 960)),
            Err(AudioPipelineError::SourceBeforeOrigin)
        );
    }

    #[test]
    fn pause_cannot_cut_through_an_accepted_pcm_block() {
        let origin = 1_000_000_000;
        let pipeline = AudioPipeline::new(origin);
        pipeline.push(chunk(0, origin, 960)).unwrap();
        assert_eq!(
            pipeline.pause(origin + 10_000_000),
            Err(AudioPipelineError::InvalidPauseTimestamp)
        );
        pipeline.pause(origin + 20_000_000).unwrap();
    }

    #[test]
    fn pause_ignores_input_and_resume_removes_the_pause_interval() {
        let origin = 1_000_000_000;
        let pipeline = AudioPipeline::new(origin);
        pipeline.push(chunk(0, origin, 960)).unwrap();
        pipeline.pause(origin + 25_000_000).unwrap();
        assert_eq!(
            pipeline.push(chunk(1, origin + 500_000_000, 960)).unwrap(),
            AudioPushOutcome::IgnoredWhilePaused
        );
        pipeline.resume(origin + 1_025_000_000).unwrap();
        assert_eq!(
            pipeline
                .push(chunk(2, origin + 1_035_000_000, 960))
                .unwrap(),
            AudioPushOutcome::Queued {
                presentation_at_ns: 35_000_000,
                duration_ns: 20_000_000,
                gap_before_ns: 15_000_000,
            }
        );
        assert_eq!(pipeline.finish(origin + 1_060_000_000).unwrap(), 60_000_000);
        assert_eq!(pipeline.stats().unwrap().ignored_while_paused, 1);
    }

    #[test]
    fn one_second_queue_fails_atomically_and_retry_succeeds_after_pop() {
        let pipeline = AudioPipeline::new(0);
        for index in 0..10_u64 {
            pipeline
                .push(chunk(index, index * 100_000_000, MAX_AUDIO_CHUNK_FRAMES))
                .unwrap();
        }
        let overflow = chunk(10, 1_000_000_000, MAX_AUDIO_CHUNK_FRAMES);
        assert_eq!(
            pipeline.push(overflow),
            Err(AudioPipelineError::Backpressure)
        );
        assert_eq!(
            pipeline.stats().unwrap(),
            AudioPipelineStats {
                queued_chunks: 10,
                queued_frames: 48_000,
                queued_bytes: MAX_QUEUED_AUDIO_BYTES,
                accepted_chunks: 10,
                ignored_while_paused: 0,
            }
        );
        pipeline.pop().unwrap().expect("队首块应存在");
        pipeline
            .push(chunk(10, 1_000_000_000, MAX_AUDIO_CHUNK_FRAMES))
            .expect("背压失败不能推进时间线，同一块应可重试");
    }

    #[test]
    fn finish_rejects_time_before_last_audio_end_and_closes_pipeline() {
        let pipeline = AudioPipeline::new(100);
        pipeline.push(chunk(0, 100, 960)).unwrap();
        assert_eq!(
            pipeline.finish(10_000_100),
            Err(AudioPipelineError::FinishBeforeBufferedAudioEnd)
        );
        assert_eq!(pipeline.finish(20_000_100).unwrap(), 20_000_000);
        assert_eq!(
            pipeline.push(chunk(1, 20_000_100, 960)),
            Err(AudioPipelineError::Closed)
        );
    }

    #[test]
    fn abort_rejects_new_audio_without_discarding_queued_prefix() {
        let pipeline = AudioPipeline::new(0);
        pipeline.push(chunk(0, 0, 960)).unwrap();
        pipeline.abort().unwrap();
        assert_eq!(
            pipeline.push(chunk(1, 20_000_000, 960)),
            Err(AudioPipelineError::Aborted)
        );
        assert!(pipeline.pop().unwrap().is_some());
    }
}
