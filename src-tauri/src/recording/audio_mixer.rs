//! 两个归一化录屏音源到单一 48 kHz stereo 音轨的有界混音合同。
//!
//! 平台 source 仍由同一个音频 worker 线程持有。本层只按共享会话时钟把 packet 映射到固定
//! sample grid，等待有限乱序窗口后用静音补洞，并以固定 headroom 合成一路 PCM。

use super::audio::{
    AudioFormat, CapturedAudioChunk, AUDIO_SAMPLE_RATE_HZ, DEFAULT_OPUS_FRAME_MS,
    MAX_AUDIO_CHUNK_FRAMES,
};
use super::audio_worker::RecordingAudioSource;
use std::collections::VecDeque;
use std::time::Duration;
use thiserror::Error;

const NANOS_PER_SECOND: u128 = 1_000_000_000;
const OUTPUT_CHANNELS: u16 = 2;
const INPUT_GAIN: f32 = 0.5;
const MAX_JITTER_NS: u64 = 100_000_000;
const MAX_STAGING_FRAMES: u64 = AUDIO_SAMPLE_RATE_HZ as u64;
const MAX_STAGING_CHUNKS: usize = 128;
const MAX_NONBLOCKING_DRAIN: usize = 8;
const OUTPUT_CHUNK_FRAMES: u32 = AUDIO_SAMPLE_RATE_HZ * DEFAULT_OPUS_FRAME_MS / 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MixerInput {
    System,
    Microphone,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(super) enum AudioMixerError {
    #[error("混音输入只接受 48 kHz mono/stereo PCM")]
    UnsupportedFormat,
    #[error("混音输入块为空或超过 100 ms")]
    InvalidFrameCount,
    #[error("混音输入样本数量无效")]
    InvalidSampleLength,
    #[error("混音输入包含 NaN 或无穷值")]
    NonFiniteSample,
    #[error("混音输入序号没有严格递增")]
    SequenceNotIncreasing,
    #[error("混音输入区间重叠")]
    InputOverlap,
    #[error("混音输入晚于已经确认的静音或已经提交的输出")]
    LateInput,
    #[error("混音控制时间戳倒退")]
    ControlTimestampRegressed,
    #[error("混音停止时间戳早于已经接收的样本末尾")]
    StopBeforeLastSample,
    #[error("混音 staging 已达到固定上限")]
    StagingBudgetExceeded,
    #[error("混音时间线计算溢出")]
    TimelineOverflow,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(super) enum MixedAudioSourceError {
    #[error("系统声音源失败: {0}")]
    System(String),
    #[error("麦克风音源失败: {0}")]
    Microphone(String),
    #[error(transparent)]
    Mixer(#[from] AudioMixerError),
    #[error("双源音频控制无法保持一致: {0}")]
    ControlDiverged(String),
}

#[derive(Debug)]
struct StagedChunk {
    start_frame: u64,
    end_frame: u64,
    channels: u16,
    samples: Box<[f32]>,
}

#[derive(Debug, Default)]
struct InputTimeline {
    chunks: VecDeque<StagedChunk>,
    last_sequence: Option<u64>,
    last_end_frame: Option<u64>,
    silence_watermark_frame: u64,
    last_control_frame: Option<u64>,
    staged_frames: u64,
}

impl InputTimeline {
    fn push(
        &mut self,
        chunk: CapturedAudioChunk,
        committed_output_frame: u64,
    ) -> Result<(), AudioMixerError> {
        validate_input_chunk(&chunk)?;
        if self
            .last_sequence
            .is_some_and(|sequence| chunk.sequence <= sequence)
        {
            return Err(AudioMixerError::SequenceNotIncreasing);
        }
        let start_frame = ns_to_frame_nearest(chunk.captured_at_ns)?;
        let end_frame = start_frame
            .checked_add(u64::from(chunk.frame_count))
            .ok_or(AudioMixerError::TimelineOverflow)?;
        if self
            .last_end_frame
            .is_some_and(|last_end| start_frame < last_end)
        {
            return Err(AudioMixerError::InputOverlap);
        }
        if start_frame < committed_output_frame || start_frame < self.silence_watermark_frame {
            return Err(AudioMixerError::LateInput);
        }
        let next_frames = self
            .staged_frames
            .checked_add(u64::from(chunk.frame_count))
            .ok_or(AudioMixerError::StagingBudgetExceeded)?;
        if next_frames > MAX_STAGING_FRAMES || self.chunks.len() >= MAX_STAGING_CHUNKS {
            return Err(AudioMixerError::StagingBudgetExceeded);
        }
        self.last_sequence = Some(chunk.sequence);
        self.last_end_frame = Some(end_frame);
        self.staged_frames = next_frames;
        self.chunks.push_back(StagedChunk {
            start_frame,
            end_frame,
            channels: chunk.format.channels,
            samples: chunk.samples,
        });
        Ok(())
    }

    fn advance_control(&mut self, timestamp_ns: u64) -> Result<(), AudioMixerError> {
        let frame = ns_to_frame_floor(timestamp_ns)?;
        if self.last_control_frame.is_some_and(|last| frame < last) {
            return Err(AudioMixerError::ControlTimestampRegressed);
        }
        self.last_control_frame = Some(frame);
        let safe_ns = timestamp_ns.saturating_sub(MAX_JITTER_NS);
        self.silence_watermark_frame = self
            .silence_watermark_frame
            .max(ns_to_frame_floor(safe_ns)?);
        Ok(())
    }

    fn finish(&mut self, timestamp_ns: u64) -> Result<(), AudioMixerError> {
        let frame = ns_to_frame_floor(timestamp_ns)?;
        if self.last_control_frame.is_some_and(|last| frame < last) {
            return Err(AudioMixerError::ControlTimestampRegressed);
        }
        let inclusive_frame = ns_to_frame_ceil(timestamp_ns)?;
        if self
            .last_end_frame
            .is_some_and(|last_end| inclusive_frame < last_end)
        {
            return Err(AudioMixerError::StopBeforeLastSample);
        }
        self.last_control_frame = Some(frame);
        self.silence_watermark_frame = self.silence_watermark_frame.max(frame);
        Ok(())
    }

    fn watermark(&self) -> u64 {
        self.silence_watermark_frame
            .max(self.last_end_frame.unwrap_or_default())
    }

    fn sample_at(&self, frame: u64, output_channel: usize) -> f32 {
        for chunk in &self.chunks {
            if frame < chunk.start_frame {
                return 0.0;
            }
            if frame >= chunk.end_frame {
                continue;
            }
            let source_channel = if chunk.channels == 1 {
                0
            } else {
                output_channel
            };
            let frame_offset = usize::try_from(frame - chunk.start_frame).unwrap_or(usize::MAX);
            let index = frame_offset
                .checked_mul(usize::from(chunk.channels))
                .and_then(|base| base.checked_add(source_channel));
            return index
                .and_then(|index| chunk.samples.get(index).copied())
                .unwrap_or(0.0);
        }
        0.0
    }

    fn discard_before(&mut self, frame: u64) {
        while self
            .chunks
            .front()
            .is_some_and(|chunk| chunk.end_frame <= frame)
        {
            if let Some(chunk) = self.chunks.pop_front() {
                self.staged_frames = self
                    .staged_frames
                    .saturating_sub(chunk.end_frame - chunk.start_frame);
            }
        }
    }

    fn clear(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, Default)]
struct AudioMixer {
    system: InputTimeline,
    microphone: InputTimeline,
    output_cursor_frame: u64,
    next_sequence: u64,
}

impl AudioMixer {
    fn input_mut(&mut self, input: MixerInput) -> &mut InputTimeline {
        match input {
            MixerInput::System => &mut self.system,
            MixerInput::Microphone => &mut self.microphone,
        }
    }

    fn push(
        &mut self,
        input: MixerInput,
        chunk: CapturedAudioChunk,
    ) -> Result<(), AudioMixerError> {
        let cursor = self.output_cursor_frame;
        self.input_mut(input).push(chunk, cursor)
    }

    fn advance_control(
        &mut self,
        system_timestamp_ns: u64,
        microphone_timestamp_ns: u64,
    ) -> Result<(), AudioMixerError> {
        self.system.advance_control(system_timestamp_ns)?;
        self.microphone.advance_control(microphone_timestamp_ns)?;
        Ok(())
    }

    fn finish(
        &mut self,
        system_timestamp_ns: u64,
        microphone_timestamp_ns: u64,
    ) -> Result<(), AudioMixerError> {
        self.system.finish(system_timestamp_ns)?;
        self.microphone.finish(microphone_timestamp_ns)?;
        Ok(())
    }

    fn pop_ready(&mut self) -> Result<Option<CapturedAudioChunk>, AudioMixerError> {
        let ready_until = self.system.watermark().min(self.microphone.watermark());
        if ready_until <= self.output_cursor_frame {
            return Ok(None);
        }
        let available = ready_until - self.output_cursor_frame;
        let frame_count = u32::try_from(available.min(u64::from(OUTPUT_CHUNK_FRAMES)))
            .map_err(|_| AudioMixerError::TimelineOverflow)?;
        let sample_count = usize::try_from(frame_count)
            .ok()
            .and_then(|frames| frames.checked_mul(usize::from(OUTPUT_CHANNELS)))
            .ok_or(AudioMixerError::TimelineOverflow)?;
        let mut samples = Vec::with_capacity(sample_count);
        for offset in 0..u64::from(frame_count) {
            let frame = self
                .output_cursor_frame
                .checked_add(offset)
                .ok_or(AudioMixerError::TimelineOverflow)?;
            for channel in 0..usize::from(OUTPUT_CHANNELS) {
                let mixed = INPUT_GAIN * self.system.sample_at(frame, channel)
                    + INPUT_GAIN * self.microphone.sample_at(frame, channel);
                samples.push(mixed.clamp(-1.0, 1.0));
            }
        }
        let captured_at_ns = frame_to_ns(self.output_cursor_frame)?;
        self.output_cursor_frame = self
            .output_cursor_frame
            .checked_add(u64::from(frame_count))
            .ok_or(AudioMixerError::TimelineOverflow)?;
        self.system.discard_before(self.output_cursor_frame);
        self.microphone.discard_before(self.output_cursor_frame);
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(AudioMixerError::TimelineOverflow)?;
        Ok(Some(CapturedAudioChunk {
            sequence,
            captured_at_ns,
            format: AudioFormat::normalized(OUTPUT_CHANNELS),
            frame_count,
            samples: samples.into_boxed_slice(),
        }))
    }

    fn restart_at(&mut self, timestamp_ns: u64) -> Result<(), AudioMixerError> {
        self.system.clear();
        self.microphone.clear();
        self.output_cursor_frame = ns_to_frame_nearest(timestamp_ns)?;
        Ok(())
    }

    fn discard(&mut self) {
        self.system.clear();
        self.microphone.clear();
    }
}

/// 同一 worker 线程内持有两个线程亲和的平台 source，并把它们投影为一个普通音频 source。
pub(super) struct MixedAudioSource<S, M> {
    system: S,
    microphone: M,
    mixer: AudioMixer,
    stopped_chunks: Vec<CapturedAudioChunk>,
    poll_system_first: bool,
}

impl<S, M> MixedAudioSource<S, M> {
    pub fn new(system: S, microphone: M) -> Self {
        Self {
            system,
            microphone,
            mixer: AudioMixer::default(),
            stopped_chunks: Vec::new(),
            poll_system_first: true,
        }
    }
}

impl<S, M> MixedAudioSource<S, M>
where
    S: RecordingAudioSource,
    M: RecordingAudioSource,
{
    fn poll_system(&mut self, timeout: Duration) -> Result<bool, MixedAudioSourceError> {
        match self
            .system
            .capture_next_available(timeout)
            .map_err(|error| MixedAudioSourceError::System(error.to_string()))?
        {
            Some(chunk) => {
                self.mixer.push(MixerInput::System, chunk)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn poll_microphone(&mut self, timeout: Duration) -> Result<bool, MixedAudioSourceError> {
        match self
            .microphone
            .capture_next_available(timeout)
            .map_err(|error| MixedAudioSourceError::Microphone(error.to_string()))?
        {
            Some(chunk) => {
                self.mixer.push(MixerInput::Microphone, chunk)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn drain_nonblocking(&mut self) -> Result<(), MixedAudioSourceError> {
        for _ in 0..MAX_NONBLOCKING_DRAIN {
            let system = self.poll_system(Duration::ZERO)?;
            let microphone = self.poll_microphone(Duration::ZERO)?;
            if !system && !microphone {
                break;
            }
        }
        Ok(())
    }

    fn update_watermarks(&mut self) -> Result<(), MixedAudioSourceError> {
        let system = self
            .system
            .control_timestamp_ns()
            .map_err(|error| MixedAudioSourceError::System(error.to_string()))?;
        let microphone = self
            .microphone
            .control_timestamp_ns()
            .map_err(|error| MixedAudioSourceError::Microphone(error.to_string()))?;
        self.mixer.advance_control(system, microphone)?;
        Ok(())
    }

    fn pause_both(&mut self) -> Result<u64, MixedAudioSourceError> {
        let system = self
            .system
            .pause_capture()
            .map_err(|error| MixedAudioSourceError::System(error.to_string()))?;
        match self.microphone.pause_capture() {
            Ok(microphone) => {
                self.mixer.discard();
                Ok(system.max(microphone))
            }
            Err(error) => {
                let rollback = self.system.resume_capture();
                Err(MixedAudioSourceError::ControlDiverged(format!(
                    "麦克风暂停失败: {error}; 系统声恢复结果: {}",
                    rollback
                        .map(|_| "ok".to_string())
                        .unwrap_or_else(|rollback| rollback.to_string())
                )))
            }
        }
    }

    fn resume_both(&mut self) -> Result<u64, MixedAudioSourceError> {
        let system = self
            .system
            .resume_capture()
            .map_err(|error| MixedAudioSourceError::System(error.to_string()))?;
        match self.microphone.resume_capture() {
            Ok(microphone) => {
                let resumed_at = system.max(microphone);
                self.mixer.restart_at(resumed_at)?;
                Ok(resumed_at)
            }
            Err(error) => {
                let rollback = self.system.pause_capture();
                Err(MixedAudioSourceError::ControlDiverged(format!(
                    "麦克风恢复失败: {error}; 系统声重新暂停结果: {}",
                    rollback
                        .map(|_| "ok".to_string())
                        .unwrap_or_else(|rollback| rollback.to_string())
                )))
            }
        }
    }

    fn stop_both(&mut self) -> Result<u64, MixedAudioSourceError> {
        let system_stop = self.system.stop_capture();
        let microphone_stop = self.microphone.stop_capture();
        let system_stop =
            system_stop.map_err(|error| MixedAudioSourceError::System(error.to_string()))?;
        let microphone_stop = microphone_stop
            .map_err(|error| MixedAudioSourceError::Microphone(error.to_string()))?;

        for chunk in self
            .system
            .take_stopped_chunks()
            .map_err(|error| MixedAudioSourceError::System(error.to_string()))?
        {
            self.mixer.push(MixerInput::System, chunk)?;
        }
        for chunk in self
            .microphone
            .take_stopped_chunks()
            .map_err(|error| MixedAudioSourceError::Microphone(error.to_string()))?
        {
            self.mixer.push(MixerInput::Microphone, chunk)?;
        }
        self.mixer.finish(system_stop, microphone_stop)?;
        while let Some(chunk) = self.mixer.pop_ready()? {
            self.stopped_chunks.push(chunk);
        }
        Ok(system_stop.max(microphone_stop))
    }
}

impl<S, M> RecordingAudioSource for MixedAudioSource<S, M>
where
    S: RecordingAudioSource,
    M: RecordingAudioSource,
{
    type Error = MixedAudioSourceError;

    fn capture_next_available(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        self.drain_nonblocking()?;
        self.update_watermarks()?;
        if let Some(chunk) = self.mixer.pop_ready()? {
            return Ok(Some(chunk));
        }

        let first_timeout = timeout / 2;
        let second_timeout = timeout.saturating_sub(first_timeout);
        if self.poll_system_first {
            self.poll_system(first_timeout)?;
            self.poll_microphone(second_timeout)?;
        } else {
            self.poll_microphone(first_timeout)?;
            self.poll_system(second_timeout)?;
        }
        self.poll_system_first = !self.poll_system_first;
        self.drain_nonblocking()?;
        self.update_watermarks()?;
        self.mixer.pop_ready().map_err(Into::into)
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        let system = self
            .system
            .control_timestamp_ns()
            .map_err(|error| MixedAudioSourceError::System(error.to_string()))?;
        let microphone = self
            .microphone
            .control_timestamp_ns()
            .map_err(|error| MixedAudioSourceError::Microphone(error.to_string()))?;
        Ok(system.max(microphone))
    }

    fn pause_capture(&mut self) -> Result<u64, Self::Error> {
        self.pause_both()
    }

    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        self.resume_both()
    }

    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        self.stop_both()
    }

    fn take_stopped_chunks(&mut self) -> Result<Vec<CapturedAudioChunk>, Self::Error> {
        Ok(std::mem::take(&mut self.stopped_chunks))
    }
}

fn validate_input_chunk(chunk: &CapturedAudioChunk) -> Result<(), AudioMixerError> {
    if chunk.format.sample_rate_hz != AUDIO_SAMPLE_RATE_HZ
        || !(1..=OUTPUT_CHANNELS).contains(&chunk.format.channels)
    {
        return Err(AudioMixerError::UnsupportedFormat);
    }
    if chunk.frame_count == 0 || chunk.frame_count > MAX_AUDIO_CHUNK_FRAMES {
        return Err(AudioMixerError::InvalidFrameCount);
    }
    let expected = usize::try_from(chunk.frame_count)
        .ok()
        .and_then(|frames| frames.checked_mul(usize::from(chunk.format.channels)))
        .ok_or(AudioMixerError::InvalidSampleLength)?;
    if chunk.samples.len() != expected {
        return Err(AudioMixerError::InvalidSampleLength);
    }
    if chunk.samples.iter().any(|sample| !sample.is_finite()) {
        return Err(AudioMixerError::NonFiniteSample);
    }
    Ok(())
}

fn ns_to_frame_nearest(timestamp_ns: u64) -> Result<u64, AudioMixerError> {
    let numerator = u128::from(timestamp_ns)
        .checked_mul(u128::from(AUDIO_SAMPLE_RATE_HZ))
        .and_then(|value| value.checked_add(NANOS_PER_SECOND / 2))
        .ok_or(AudioMixerError::TimelineOverflow)?;
    u64::try_from(numerator / NANOS_PER_SECOND).map_err(|_| AudioMixerError::TimelineOverflow)
}

fn ns_to_frame_floor(timestamp_ns: u64) -> Result<u64, AudioMixerError> {
    let numerator = u128::from(timestamp_ns)
        .checked_mul(u128::from(AUDIO_SAMPLE_RATE_HZ))
        .ok_or(AudioMixerError::TimelineOverflow)?;
    u64::try_from(numerator / NANOS_PER_SECOND).map_err(|_| AudioMixerError::TimelineOverflow)
}

fn ns_to_frame_ceil(timestamp_ns: u64) -> Result<u64, AudioMixerError> {
    let numerator = u128::from(timestamp_ns)
        .checked_mul(u128::from(AUDIO_SAMPLE_RATE_HZ))
        .and_then(|value| value.checked_add(NANOS_PER_SECOND - 1))
        .ok_or(AudioMixerError::TimelineOverflow)?;
    u64::try_from(numerator / NANOS_PER_SECOND).map_err(|_| AudioMixerError::TimelineOverflow)
}

fn frame_to_ns(frame: u64) -> Result<u64, AudioMixerError> {
    let numerator = u128::from(frame)
        .checked_mul(NANOS_PER_SECOND)
        .ok_or(AudioMixerError::TimelineOverflow)?;
    u64::try_from(numerator / u128::from(AUDIO_SAMPLE_RATE_HZ))
        .map_err(|_| AudioMixerError::TimelineOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Error)]
    #[error("{0}")]
    struct FixtureError(&'static str);

    struct FixtureSource {
        chunks: VecDeque<CapturedAudioChunk>,
        tails: Vec<CapturedAudioChunk>,
        timestamp_ns: u64,
        events: Arc<Mutex<Vec<&'static str>>>,
        fail_capture: bool,
        fail_pause: bool,
    }

    impl FixtureSource {
        fn new(timestamp_ns: u64, events: Arc<Mutex<Vec<&'static str>>>) -> Self {
            Self {
                chunks: VecDeque::new(),
                tails: Vec::new(),
                timestamp_ns,
                events,
                fail_capture: false,
                fail_pause: false,
            }
        }
    }

    impl RecordingAudioSource for FixtureSource {
        type Error = FixtureError;

        fn capture_next_available(
            &mut self,
            _timeout: Duration,
        ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
            if self.fail_capture {
                Err(FixtureError("capture failed"))
            } else {
                Ok(self.chunks.pop_front())
            }
        }

        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            Ok(self.timestamp_ns)
        }

        fn pause_capture(&mut self) -> Result<u64, Self::Error> {
            self.events.lock().unwrap().push("pause");
            if self.fail_pause {
                Err(FixtureError("pause failed"))
            } else {
                Ok(self.timestamp_ns)
            }
        }

        fn resume_capture(&mut self) -> Result<u64, Self::Error> {
            self.events.lock().unwrap().push("resume");
            Ok(self.timestamp_ns)
        }

        fn stop_capture(&mut self) -> Result<u64, Self::Error> {
            self.events.lock().unwrap().push("stop");
            Ok(self.timestamp_ns)
        }

        fn take_stopped_chunks(&mut self) -> Result<Vec<CapturedAudioChunk>, Self::Error> {
            Ok(std::mem::take(&mut self.tails))
        }
    }

    fn chunk(
        sequence: u64,
        start_frame: u64,
        frames: u32,
        channels: u16,
        samples: Vec<f32>,
    ) -> CapturedAudioChunk {
        CapturedAudioChunk {
            sequence,
            captured_at_ns: frame_to_ns(start_frame).unwrap(),
            format: AudioFormat::normalized(channels),
            frame_count: frames,
            samples: samples.into_boxed_slice(),
        }
    }

    fn constant(
        sequence: u64,
        start_frame: u64,
        frames: u32,
        channels: u16,
        value: f32,
    ) -> CapturedAudioChunk {
        chunk(
            sequence,
            start_frame,
            frames,
            channels,
            vec![value; frames as usize * channels as usize],
        )
    }

    fn finish_at(mixer: &mut AudioMixer, frame: u64) {
        let timestamp = frame_to_ns(frame).unwrap();
        mixer.finish(timestamp, timestamp).unwrap();
    }

    fn drain_samples(mixer: &mut AudioMixer) -> Vec<f32> {
        let mut samples = Vec::new();
        while let Some(chunk) = mixer.pop_ready().unwrap() {
            samples.extend_from_slice(&chunk.samples);
        }
        samples
    }

    #[test]
    fn fixed_headroom_mixes_two_full_scale_sources_without_clipping() {
        let mut mixer = AudioMixer::default();
        mixer
            .push(MixerInput::System, constant(0, 0, 960, 2, 1.0))
            .unwrap();
        mixer
            .push(MixerInput::Microphone, constant(0, 0, 960, 2, 1.0))
            .unwrap();
        finish_at(&mut mixer, 960);
        let output = mixer.pop_ready().unwrap().unwrap();
        assert_eq!(output.frame_count, 960);
        assert!(output.samples.iter().all(|sample| *sample == 1.0));
    }

    #[test]
    fn silent_peer_does_not_change_the_active_source_gain() {
        let mut mixer = AudioMixer::default();
        mixer
            .push(MixerInput::System, constant(0, 0, 960, 2, 0.8))
            .unwrap();
        finish_at(&mut mixer, 960);
        let output = mixer.pop_ready().unwrap().unwrap();
        assert!(output.samples.iter().all(|sample| *sample == 0.4));
    }

    #[test]
    fn mono_is_duplicated_and_stereo_channels_stay_independent() {
        let mut mixer = AudioMixer::default();
        mixer
            .push(
                MixerInput::System,
                chunk(0, 0, 2, 2, vec![1.0, -1.0, 0.5, -0.5]),
            )
            .unwrap();
        mixer
            .push(MixerInput::Microphone, chunk(0, 0, 2, 1, vec![0.5, -0.5]))
            .unwrap();
        finish_at(&mut mixer, 2);
        assert_eq!(drain_samples(&mut mixer), vec![0.75, -0.25, 0.0, -0.5]);
    }

    #[test]
    fn packet_boundaries_do_not_change_the_mix() {
        fn render(split: bool) -> Vec<f32> {
            let mut mixer = AudioMixer::default();
            if split {
                mixer
                    .push(MixerInput::System, constant(0, 0, 480, 2, 0.2))
                    .unwrap();
                mixer
                    .push(MixerInput::System, constant(1, 480, 480, 2, 0.2))
                    .unwrap();
                mixer
                    .push(MixerInput::Microphone, constant(0, 0, 320, 2, 0.4))
                    .unwrap();
                mixer
                    .push(MixerInput::Microphone, constant(1, 320, 640, 2, 0.4))
                    .unwrap();
            } else {
                mixer
                    .push(MixerInput::System, constant(0, 0, 960, 2, 0.2))
                    .unwrap();
                mixer
                    .push(MixerInput::Microphone, constant(0, 0, 960, 2, 0.4))
                    .unwrap();
            }
            finish_at(&mut mixer, 960);
            drain_samples(&mut mixer)
        }
        assert_eq!(render(false), render(true));
    }

    #[test]
    fn control_watermark_turns_missing_packets_into_bounded_silence() {
        let mut mixer = AudioMixer::default();
        mixer
            .push(MixerInput::System, constant(0, 0, 960, 2, 0.5))
            .unwrap();
        mixer.advance_control(120_000_000, 120_000_000).unwrap();
        let output = mixer.pop_ready().unwrap().unwrap();
        assert_eq!(output.frame_count, 960);
        assert!(output.samples.iter().all(|sample| *sample == 0.25));
    }

    #[test]
    fn packets_older_than_confirmed_silence_fail() {
        let mut mixer = AudioMixer::default();
        mixer.advance_control(200_000_000, 200_000_000).unwrap();
        assert_eq!(
            mixer.push(MixerInput::System, constant(0, 0, 960, 2, 0.5)),
            Err(AudioMixerError::LateInput)
        );
    }

    #[test]
    fn overlap_sequence_format_and_non_finite_samples_fail_closed() {
        let mut mixer = AudioMixer::default();
        mixer
            .push(MixerInput::System, constant(1, 0, 960, 2, 0.1))
            .unwrap();
        assert_eq!(
            mixer.push(MixerInput::System, constant(1, 960, 10, 2, 0.1)),
            Err(AudioMixerError::SequenceNotIncreasing)
        );
        assert_eq!(
            mixer.push(MixerInput::System, constant(2, 959, 10, 2, 0.1)),
            Err(AudioMixerError::InputOverlap)
        );
        let mut wrong_rate = constant(0, 0, 10, 2, 0.1);
        wrong_rate.format.sample_rate_hz = 44_100;
        assert_eq!(
            mixer.push(MixerInput::Microphone, wrong_rate),
            Err(AudioMixerError::UnsupportedFormat)
        );
        assert_eq!(
            mixer.push(
                MixerInput::Microphone,
                chunk(0, 0, 1, 2, vec![f32::NAN, 0.0])
            ),
            Err(AudioMixerError::NonFiniteSample)
        );
    }

    #[test]
    fn staging_has_frame_and_chunk_budgets() {
        let mut mixer = AudioMixer::default();
        for sequence in 0..10 {
            mixer
                .push(
                    MixerInput::System,
                    constant(sequence, sequence * 4_800, 4_800, 2, 0.1),
                )
                .unwrap();
        }
        assert_eq!(
            mixer.push(MixerInput::System, constant(10, 48_000, 1, 2, 0.1)),
            Err(AudioMixerError::StagingBudgetExceeded)
        );
    }

    #[test]
    fn stop_before_the_last_sample_fails_closed() {
        let mut mixer = AudioMixer::default();
        mixer
            .push(MixerInput::System, constant(0, 0, 960, 2, 0.1))
            .unwrap();
        let too_early = frame_to_ns(959).unwrap();
        assert_eq!(
            mixer.finish(too_early, frame_to_ns(960).unwrap()),
            Err(AudioMixerError::StopBeforeLastSample)
        );
    }

    #[test]
    fn restart_discards_old_samples_but_keeps_output_sequence_monotonic() {
        let mut mixer = AudioMixer::default();
        mixer
            .push(MixerInput::System, constant(0, 0, 960, 2, 1.0))
            .unwrap();
        finish_at(&mut mixer, 960);
        let first = mixer.pop_ready().unwrap().unwrap();
        mixer.restart_at(1_000_000_000).unwrap();
        mixer
            .push(MixerInput::Microphone, constant(0, 48_000, 960, 2, 1.0))
            .unwrap();
        let timestamp = frame_to_ns(48_960).unwrap();
        mixer.finish(timestamp, timestamp).unwrap();
        let second = mixer.pop_ready().unwrap().unwrap();
        assert_eq!(first.sequence, 0);
        assert_eq!(second.sequence, 1);
        assert_eq!(second.captured_at_ns, 1_000_000_000);
        assert!(second.samples.iter().all(|sample| *sample == 0.5));
    }

    #[test]
    fn mixed_source_polls_both_inputs_without_waiting_for_a_silent_peer() {
        let system_events = Arc::new(Mutex::new(Vec::new()));
        let microphone_events = Arc::new(Mutex::new(Vec::new()));
        let mut system = FixtureSource::new(120_000_000, system_events);
        let microphone = FixtureSource::new(120_000_000, microphone_events);
        system.chunks.push_back(constant(0, 0, 960, 2, 0.8));
        let mut source = MixedAudioSource::new(system, microphone);

        let output = source
            .capture_next_available(Duration::from_millis(50))
            .unwrap()
            .unwrap();
        assert_eq!(output.frame_count, 960);
        assert!(output.samples.iter().all(|sample| *sample == 0.4));
    }

    #[test]
    fn normal_stop_mixes_both_sources_finite_tail_chunks() {
        let system_events = Arc::new(Mutex::new(Vec::new()));
        let microphone_events = Arc::new(Mutex::new(Vec::new()));
        let mut system = FixtureSource::new(20_000_000, Arc::clone(&system_events));
        let mut microphone = FixtureSource::new(20_000_000, Arc::clone(&microphone_events));
        system.tails.push(constant(0, 0, 960, 2, 0.8));
        microphone.tails.push(constant(0, 0, 960, 2, 0.2));
        let mut source = MixedAudioSource::new(system, microphone);

        assert_eq!(source.stop_capture().unwrap(), 20_000_000);
        let tail = source.take_stopped_chunks().unwrap();
        assert_eq!(tail.len(), 1);
        assert!(tail[0].samples.iter().all(|sample| *sample == 0.5));
        assert_eq!(*system_events.lock().unwrap(), vec!["stop"]);
        assert_eq!(*microphone_events.lock().unwrap(), vec!["stop"]);
    }

    #[test]
    fn second_source_pause_failure_rolls_back_the_first_source() {
        let system_events = Arc::new(Mutex::new(Vec::new()));
        let microphone_events = Arc::new(Mutex::new(Vec::new()));
        let system = FixtureSource::new(20_000_000, Arc::clone(&system_events));
        let mut microphone = FixtureSource::new(20_000_000, Arc::clone(&microphone_events));
        microphone.fail_pause = true;
        let mut source = MixedAudioSource::new(system, microphone);

        assert!(matches!(
            source.pause_capture(),
            Err(MixedAudioSourceError::ControlDiverged(_))
        ));
        assert_eq!(*system_events.lock().unwrap(), vec!["pause", "resume"]);
        assert_eq!(*microphone_events.lock().unwrap(), vec!["pause"]);
    }

    #[test]
    fn either_source_capture_failure_keeps_its_identity() {
        let system_events = Arc::new(Mutex::new(Vec::new()));
        let microphone_events = Arc::new(Mutex::new(Vec::new()));
        let mut system = FixtureSource::new(20_000_000, Arc::clone(&system_events));
        system.fail_capture = true;
        let microphone = FixtureSource::new(20_000_000, Arc::clone(&microphone_events));
        let mut source = MixedAudioSource::new(system, microphone);
        assert!(matches!(
            source.capture_next_available(Duration::ZERO),
            Err(MixedAudioSourceError::System(_))
        ));

        let system = FixtureSource::new(20_000_000, system_events);
        let mut microphone = FixtureSource::new(20_000_000, microphone_events);
        microphone.fail_capture = true;
        let mut source = MixedAudioSource::new(system, microphone);
        assert!(matches!(
            source.capture_next_available(Duration::ZERO),
            Err(MixedAudioSourceError::Microphone(_))
        ));
    }
}
