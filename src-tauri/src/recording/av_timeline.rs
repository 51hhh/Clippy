//! 视频首帧零点与音频会话零点之间的双轨协调合同。
//!
//! 现有视频 timeline 继续把首个有效帧映射为 `0`；音频先保留相对 session clock 的真实起点。
//! 本层只裁切首视频帧以前的 PCM，并把后续音频平移到视频零点，不编码或写容器。

use super::audio::{frames_to_ns, QueuedAudioChunk, AUDIO_SAMPLE_RATE_HZ};
use super::pipeline::QueuedFrame;
use thiserror::Error;

const NANOS_PER_SECOND: u128 = 1_000_000_000;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(super) enum AvTimelineError {
    #[error("A/V epoch 已经建立")]
    EpochAlreadyEstablished,
    #[error("A/V epoch 尚未建立")]
    EpochNotEstablished,
    #[error("A/V epoch 只能使用 presentation 为零的首视频帧")]
    VideoDoesNotStartAtZero,
    #[error("首视频帧早于录屏会话起点")]
    VideoBeforeSessionOrigin,
    #[error("音频块合同无效")]
    InvalidAudioChunk,
    #[error("A/V 音频输入区间倒退或重叠")]
    AudioInputOverlap,
    #[error("A/V 音频输出区间倒退或重叠")]
    AudioOutputOverlap,
    #[error("A/V 时间线计算溢出")]
    TimelineOverflow,
    #[error("音频在首视频帧以前已经结束")]
    AudioFinishedBeforeVideoEpoch,
    #[error("音频结束时间早于已协调的 PCM")]
    AudioFinishBeforeOutput,
}

#[derive(Debug)]
pub(super) enum AudioEpochOutcome {
    DroppedBeforeVideo {
        sequence: u64,
        frame_count: u32,
    },
    Aligned {
        chunk: QueuedAudioChunk,
        trimmed_leading_frames: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LeadingTrack {
    Aligned,
    Audio,
    Video,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AvFinishReport {
    pub video_duration_ns: u64,
    pub audio_duration_ns: u64,
    pub mux_duration_ns: u64,
    pub leading_track: LeadingTrack,
    pub track_delta_ns: u64,
}

#[derive(Debug, Clone)]
pub(super) struct AvTimelineCoordinator {
    session_origin_ns: u64,
    audio_epoch_offset_ns: Option<u64>,
    last_audio_input_end_ns: Option<u64>,
    last_audio_output_end_ns: Option<u64>,
}

impl AvTimelineCoordinator {
    pub const fn new(session_origin_ns: u64) -> Self {
        Self {
            session_origin_ns,
            audio_epoch_offset_ns: None,
            last_audio_input_end_ns: None,
            last_audio_output_end_ns: None,
        }
    }

    pub fn establish_video_epoch(
        &mut self,
        first_video: &QueuedFrame,
    ) -> Result<u64, AvTimelineError> {
        if self.audio_epoch_offset_ns.is_some() {
            return Err(AvTimelineError::EpochAlreadyEstablished);
        }
        if first_video.presentation_at_ns != 0 {
            return Err(AvTimelineError::VideoDoesNotStartAtZero);
        }
        let offset = first_video
            .frame
            .captured_at_ns
            .checked_sub(self.session_origin_ns)
            .ok_or(AvTimelineError::VideoBeforeSessionOrigin)?;
        self.audio_epoch_offset_ns = Some(offset);
        Ok(offset)
    }

    pub fn align_audio(
        &mut self,
        mut queued: QueuedAudioChunk,
    ) -> Result<AudioEpochOutcome, AvTimelineError> {
        let epoch = self
            .audio_epoch_offset_ns
            .ok_or(AvTimelineError::EpochNotEstablished)?;
        let expected_duration = frames_to_ns(queued.chunk.frame_count)
            .map_err(|_| AvTimelineError::TimelineOverflow)?;
        let expected_samples = usize::try_from(queued.chunk.frame_count)
            .ok()
            .and_then(|frames| frames.checked_mul(usize::from(queued.chunk.format.channels)))
            .ok_or(AvTimelineError::InvalidAudioChunk)?;
        if queued.chunk.frame_count == 0
            || queued.duration_ns != expected_duration
            || queued.chunk.samples.len() != expected_samples
        {
            return Err(AvTimelineError::InvalidAudioChunk);
        }

        let input_start_ns = queued.presentation_at_ns;
        let input_end_ns = input_start_ns
            .checked_add(queued.duration_ns)
            .ok_or(AvTimelineError::TimelineOverflow)?;
        if self
            .last_audio_input_end_ns
            .is_some_and(|last_end| input_start_ns < last_end)
        {
            return Err(AvTimelineError::AudioInputOverlap);
        }

        if input_end_ns <= epoch {
            self.last_audio_input_end_ns = Some(input_end_ns);
            return Ok(AudioEpochOutcome::DroppedBeforeVideo {
                sequence: queued.chunk.sequence,
                frame_count: queued.chunk.frame_count,
            });
        }

        let trimmed_leading_frames = if input_start_ns < epoch {
            frames_covering_ns(epoch - input_start_ns)?.min(queued.chunk.frame_count)
        } else {
            0
        };
        if trimmed_leading_frames >= queued.chunk.frame_count {
            self.last_audio_input_end_ns = Some(input_end_ns);
            return Ok(AudioEpochOutcome::DroppedBeforeVideo {
                sequence: queued.chunk.sequence,
                frame_count: queued.chunk.frame_count,
            });
        }

        let trimmed_duration_ns =
            frames_to_ns(trimmed_leading_frames).map_err(|_| AvTimelineError::TimelineOverflow)?;
        let retained_start_ns = input_start_ns
            .checked_add(trimmed_duration_ns)
            .ok_or(AvTimelineError::TimelineOverflow)?;
        let presentation_at_ns = retained_start_ns
            .checked_sub(epoch)
            .ok_or(AvTimelineError::TimelineOverflow)?;
        let retained_frames = queued.chunk.frame_count - trimmed_leading_frames;
        let retained_duration_ns =
            frames_to_ns(retained_frames).map_err(|_| AvTimelineError::TimelineOverflow)?;
        let presentation_end_ns = presentation_at_ns
            .checked_add(retained_duration_ns)
            .ok_or(AvTimelineError::TimelineOverflow)?;
        let previous_output_end_ns = self.last_audio_output_end_ns.unwrap_or_default();
        if presentation_at_ns < previous_output_end_ns {
            return Err(AvTimelineError::AudioOutputOverlap);
        }

        if trimmed_leading_frames > 0 {
            let channels = usize::from(queued.chunk.format.channels);
            let trimmed_samples = usize::try_from(trimmed_leading_frames)
                .ok()
                .and_then(|frames| frames.checked_mul(channels))
                .ok_or(AvTimelineError::InvalidAudioChunk)?;
            queued.chunk.samples = queued.chunk.samples[trimmed_samples..]
                .to_vec()
                .into_boxed_slice();
            queued.chunk.frame_count = retained_frames;
            queued.chunk.captured_at_ns = queued
                .chunk
                .captured_at_ns
                .checked_add(trimmed_duration_ns)
                .ok_or(AvTimelineError::TimelineOverflow)?;
        }
        queued.presentation_at_ns = presentation_at_ns;
        queued.duration_ns = retained_duration_ns;
        queued.gap_before_ns = presentation_at_ns - previous_output_end_ns;

        self.last_audio_input_end_ns = Some(input_end_ns);
        self.last_audio_output_end_ns = Some(presentation_end_ns);
        Ok(AudioEpochOutcome::Aligned {
            chunk: queued,
            trimmed_leading_frames,
        })
    }

    pub fn finish(
        &self,
        video_duration_ns: u64,
        audio_session_duration_ns: u64,
    ) -> Result<AvFinishReport, AvTimelineError> {
        let epoch = self
            .audio_epoch_offset_ns
            .ok_or(AvTimelineError::EpochNotEstablished)?;
        let audio_duration_ns = audio_session_duration_ns
            .checked_sub(epoch)
            .ok_or(AvTimelineError::AudioFinishedBeforeVideoEpoch)?;
        if self
            .last_audio_output_end_ns
            .is_some_and(|last_end| audio_duration_ns < last_end)
        {
            return Err(AvTimelineError::AudioFinishBeforeOutput);
        }
        let (leading_track, track_delta_ns) = match audio_duration_ns.cmp(&video_duration_ns) {
            std::cmp::Ordering::Less => {
                (LeadingTrack::Video, video_duration_ns - audio_duration_ns)
            }
            std::cmp::Ordering::Equal => (LeadingTrack::Aligned, 0),
            std::cmp::Ordering::Greater => {
                (LeadingTrack::Audio, audio_duration_ns - video_duration_ns)
            }
        };
        Ok(AvFinishReport {
            video_duration_ns,
            audio_duration_ns,
            mux_duration_ns: video_duration_ns.max(audio_duration_ns),
            leading_track,
            track_delta_ns,
        })
    }
}

fn frames_covering_ns(duration_ns: u64) -> Result<u32, AvTimelineError> {
    let numerator = u128::from(duration_ns)
        .checked_mul(u128::from(AUDIO_SAMPLE_RATE_HZ))
        .ok_or(AvTimelineError::TimelineOverflow)?;
    let frames = numerator
        .checked_add(NANOS_PER_SECOND - 1)
        .ok_or(AvTimelineError::TimelineOverflow)?
        / NANOS_PER_SECOND;
    u32::try_from(frames).map_err(|_| AvTimelineError::TimelineOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::audio::{AudioFormat, AudioPipeline, CapturedAudioChunk};
    use crate::recording::frame::CapturedFrame;
    use crate::recording::pipeline::RecordingPipeline;

    fn video(sequence: u64, captured_at_ns: u64) -> CapturedFrame {
        CapturedFrame {
            sequence,
            captured_at_ns,
            width: 2,
            height: 2,
            stride: 8,
            rgba: vec![0; 16].into_boxed_slice(),
        }
    }

    fn audio(sequence: u64, captured_at_ns: u64, frames: u32) -> CapturedAudioChunk {
        CapturedAudioChunk {
            sequence,
            captured_at_ns,
            format: AudioFormat::normalized(2),
            frame_count: frames,
            samples: vec![sequence as f32; frames as usize * 2].into_boxed_slice(),
        }
    }

    fn first_video(captured_at_ns: u64) -> QueuedFrame {
        let pipeline = RecordingPipeline::default();
        pipeline.push(video(0, captured_at_ns)).unwrap();
        pipeline.pop().unwrap().unwrap()
    }

    #[test]
    fn epoch_requires_the_first_zero_based_video_frame_once() {
        let mut coordinator = AvTimelineCoordinator::new(100);
        let mut frame = first_video(200);
        frame.presentation_at_ns = 1;
        assert_eq!(
            coordinator.establish_video_epoch(&frame),
            Err(AvTimelineError::VideoDoesNotStartAtZero)
        );

        let first = first_video(200);
        assert_eq!(coordinator.establish_video_epoch(&first), Ok(100));
        assert_eq!(
            coordinator.establish_video_epoch(&first),
            Err(AvTimelineError::EpochAlreadyEstablished)
        );

        let mut invalid = AvTimelineCoordinator::new(201);
        assert_eq!(
            invalid.establish_video_epoch(&first),
            Err(AvTimelineError::VideoBeforeSessionOrigin)
        );
    }

    #[test]
    fn complete_audio_chunks_before_video_are_dropped() {
        let pipeline = AudioPipeline::new(0);
        pipeline.push(audio(4, 60_000_000, 960)).unwrap();
        let queued = pipeline.pop().unwrap().unwrap();
        let mut coordinator = AvTimelineCoordinator::new(0);
        coordinator
            .establish_video_epoch(&first_video(100_000_000))
            .unwrap();

        assert!(matches!(
            coordinator.align_audio(queued),
            Ok(AudioEpochOutcome::DroppedBeforeVideo {
                sequence: 4,
                frame_count: 960
            })
        ));
    }

    #[test]
    fn crossing_audio_is_trimmed_to_the_first_sample_at_or_after_video() {
        let pipeline = AudioPipeline::new(0);
        pipeline.push(audio(7, 90_000_000, 960)).unwrap();
        let queued = pipeline.pop().unwrap().unwrap();
        let mut coordinator = AvTimelineCoordinator::new(0);
        coordinator
            .establish_video_epoch(&first_video(100_000_000))
            .unwrap();

        let AudioEpochOutcome::Aligned {
            chunk,
            trimmed_leading_frames,
        } = coordinator.align_audio(queued).unwrap()
        else {
            panic!("跨 epoch 音频必须保留后半段");
        };
        assert_eq!(trimmed_leading_frames, 480);
        assert_eq!(chunk.presentation_at_ns, 0);
        assert_eq!(chunk.duration_ns, 10_000_000);
        assert_eq!(chunk.gap_before_ns, 0);
        assert_eq!(chunk.chunk.frame_count, 480);
        assert_eq!(chunk.chunk.samples.len(), 960);
        assert_eq!(chunk.chunk.captured_at_ns, 100_000_000);
    }

    #[test]
    fn fractional_epoch_rounds_up_without_moving_a_sample_earlier() {
        let pipeline = AudioPipeline::new(0);
        pipeline.push(audio(0, 0, 960)).unwrap();
        let queued = pipeline.pop().unwrap().unwrap();
        let mut coordinator = AvTimelineCoordinator::new(0);
        coordinator
            .establish_video_epoch(&first_video(10_000_001))
            .unwrap();

        let AudioEpochOutcome::Aligned {
            chunk,
            trimmed_leading_frames,
        } = coordinator.align_audio(queued).unwrap()
        else {
            panic!("应保留 epoch 后样本");
        };
        assert_eq!(trimmed_leading_frames, 481);
        assert_eq!(chunk.presentation_at_ns, 20_832);
        assert_eq!(chunk.chunk.captured_at_ns, 10_020_833);
    }

    #[test]
    fn shared_pause_boundaries_keep_audio_and_video_presentations_equal() {
        let video_pipeline = RecordingPipeline::default();
        let audio_pipeline = AudioPipeline::new(0);
        video_pipeline.push(video(0, 100_000_000)).unwrap();
        audio_pipeline.push(audio(0, 100_000_000, 960)).unwrap();
        video_pipeline.pause(200_000_000).unwrap();
        audio_pipeline.pause(200_000_000).unwrap();
        video_pipeline.resume(1_200_000_000).unwrap();
        audio_pipeline.resume(1_200_000_000).unwrap();
        video_pipeline.push(video(1, 1_300_000_000)).unwrap();
        audio_pipeline.push(audio(1, 1_300_000_000, 960)).unwrap();

        let first_video = video_pipeline.pop().unwrap().unwrap();
        let second_video = video_pipeline.pop().unwrap().unwrap();
        let mut coordinator = AvTimelineCoordinator::new(0);
        coordinator.establish_video_epoch(&first_video).unwrap();
        let first_audio = coordinator
            .align_audio(audio_pipeline.pop().unwrap().unwrap())
            .unwrap();
        let second_audio = coordinator
            .align_audio(audio_pipeline.pop().unwrap().unwrap())
            .unwrap();

        let AudioEpochOutcome::Aligned { chunk, .. } = first_audio else {
            panic!("首音频块应保留");
        };
        assert_eq!(chunk.presentation_at_ns, first_video.presentation_at_ns);
        let AudioEpochOutcome::Aligned { chunk, .. } = second_audio else {
            panic!("恢复后的音频块应保留");
        };
        assert_eq!(chunk.presentation_at_ns, second_video.presentation_at_ns);
        assert_eq!(chunk.gap_before_ns, 180_000_000);
    }

    #[test]
    fn invalid_audio_does_not_advance_alignment_state() {
        let mut coordinator = AvTimelineCoordinator::new(0);
        coordinator
            .establish_video_epoch(&first_video(100_000_000))
            .unwrap();
        let pipeline = AudioPipeline::new(0);
        pipeline.push(audio(0, 100_000_000, 960)).unwrap();
        pipeline.push(audio(1, 110_000_000, 960)).unwrap_err();
        let first = pipeline.pop().unwrap().unwrap();
        coordinator.align_audio(first).unwrap();

        let overlapping = QueuedAudioChunk {
            chunk: audio(2, 110_000_000, 960),
            presentation_at_ns: 110_000_000,
            duration_ns: 20_000_000,
            gap_before_ns: 0,
        };
        assert_eq!(
            coordinator.align_audio(overlapping).unwrap_err(),
            AvTimelineError::AudioInputOverlap
        );

        let valid = QueuedAudioChunk {
            chunk: audio(2, 120_000_000, 960),
            presentation_at_ns: 120_000_000,
            duration_ns: 20_000_000,
            gap_before_ns: 0,
        };
        assert!(matches!(
            coordinator.align_audio(valid),
            Ok(AudioEpochOutcome::Aligned { .. })
        ));
    }

    #[test]
    fn finish_preserves_real_track_delta_and_uses_the_longer_duration() {
        let mut coordinator = AvTimelineCoordinator::new(0);
        coordinator
            .establish_video_epoch(&first_video(100_000_000))
            .unwrap();

        assert_eq!(
            coordinator.finish(900_000_000, 1_020_000_000).unwrap(),
            AvFinishReport {
                video_duration_ns: 900_000_000,
                audio_duration_ns: 920_000_000,
                mux_duration_ns: 920_000_000,
                leading_track: LeadingTrack::Audio,
                track_delta_ns: 20_000_000,
            }
        );
        assert_eq!(
            coordinator.finish(950_000_000, 1_020_000_000).unwrap(),
            AvFinishReport {
                video_duration_ns: 950_000_000,
                audio_duration_ns: 920_000_000,
                mux_duration_ns: 950_000_000,
                leading_track: LeadingTrack::Video,
                track_delta_ns: 30_000_000,
            }
        );
        assert_eq!(
            coordinator.finish(0, 99_999_999),
            Err(AvTimelineError::AudioFinishedBeforeVideoEpoch)
        );
    }
}
