use super::frame::{CapturedFrame, FrameError, FrameSpec, MAX_FRAME_BYTES};
use super::timeline::{RecordingTimeline, TimelineError};
use std::collections::VecDeque;
use std::sync::Mutex;
use thiserror::Error;

pub(super) const FRAME_QUEUE_CAPACITY: usize = 3;
pub(super) const MAX_QUEUED_FRAME_BYTES: usize = FRAME_QUEUE_CAPACITY * MAX_FRAME_BYTES;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(super) enum PipelineError {
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error(transparent)]
    Timeline(#[from] TimelineError),
    #[error("录屏帧尺寸或行布局在会话中发生变化")]
    GeometryChanged,
    #[error("录屏帧序号必须严格递增")]
    SequenceNotIncreasing,
    #[error("录屏帧队列锁已损坏")]
    Poisoned,
}

#[derive(Debug)]
pub(super) struct QueuedFrame {
    pub frame: CapturedFrame,
    pub presentation_at_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PushOutcome {
    Queued {
        presentation_at_ns: u64,
    },
    QueuedAfterDropping {
        presentation_at_ns: u64,
        dropped_sequence: u64,
    },
    IgnoredWhilePaused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PipelineStats {
    pub queued_frames: usize,
    pub queued_bytes: usize,
    pub accepted_frames: u64,
    pub dropped_by_backpressure: u64,
}

#[derive(Debug, Default)]
struct PipelineState {
    spec: Option<FrameSpec>,
    last_sequence: Option<u64>,
    timeline: RecordingTimeline,
    frames: VecDeque<QueuedFrame>,
    queued_bytes: usize,
    accepted_frames: u64,
    dropped_by_backpressure: u64,
}

#[derive(Debug, Default)]
pub(super) struct RecordingPipeline {
    state: Mutex<PipelineState>,
}

impl RecordingPipeline {
    pub fn push(&self, frame: CapturedFrame) -> Result<PushOutcome, PipelineError> {
        let frame_spec = frame.validate()?;
        let mut state = self.state.lock().map_err(|_| PipelineError::Poisoned)?;
        if state.spec.is_some_and(|spec| spec != frame_spec) {
            return Err(PipelineError::GeometryChanged);
        }
        if state
            .last_sequence
            .is_some_and(|sequence| frame.sequence <= sequence)
        {
            return Err(PipelineError::SequenceNotIncreasing);
        }
        let Some(presentation_at_ns) = state.timeline.map_frame(frame.captured_at_ns)? else {
            return Ok(PushOutcome::IgnoredWhilePaused);
        };

        state.spec.get_or_insert(frame_spec);
        state.last_sequence = Some(frame.sequence);
        state.accepted_frames = state.accepted_frames.saturating_add(1);
        let dropped_sequence = if state.frames.len() == FRAME_QUEUE_CAPACITY {
            // 保留第一个待编码帧；从中间移走最旧的积压帧，使队尾始终接近最新画面。
            let dropped = state.frames.remove(1).expect("容量为三时必有中间帧");
            state.queued_bytes -= dropped.frame.rgba.len();
            state.dropped_by_backpressure = state.dropped_by_backpressure.saturating_add(1);
            Some(dropped.frame.sequence)
        } else {
            None
        };
        state.queued_bytes += frame.rgba.len();
        state.frames.push_back(QueuedFrame {
            frame,
            presentation_at_ns,
        });
        debug_assert!(state.frames.len() <= FRAME_QUEUE_CAPACITY);
        debug_assert!(state.queued_bytes <= MAX_QUEUED_FRAME_BYTES);

        Ok(match dropped_sequence {
            Some(dropped_sequence) => PushOutcome::QueuedAfterDropping {
                presentation_at_ns,
                dropped_sequence,
            },
            None => PushOutcome::Queued { presentation_at_ns },
        })
    }

    pub fn pop(&self) -> Result<Option<QueuedFrame>, PipelineError> {
        let mut state = self.state.lock().map_err(|_| PipelineError::Poisoned)?;
        let frame = state.frames.pop_front();
        if let Some(frame) = &frame {
            state.queued_bytes -= frame.frame.rgba.len();
        }
        Ok(frame)
    }

    pub fn pause(&self, captured_at_ns: u64) -> Result<(), PipelineError> {
        self.state
            .lock()
            .map_err(|_| PipelineError::Poisoned)?
            .timeline
            .pause(captured_at_ns)?;
        Ok(())
    }

    pub fn resume(&self, captured_at_ns: u64) -> Result<(), PipelineError> {
        self.state
            .lock()
            .map_err(|_| PipelineError::Poisoned)?
            .timeline
            .resume(captured_at_ns)?;
        Ok(())
    }

    pub fn stats(&self) -> Result<PipelineStats, PipelineError> {
        let state = self.state.lock().map_err(|_| PipelineError::Poisoned)?;
        Ok(PipelineStats {
            queued_frames: state.frames.len(),
            queued_bytes: state.queued_bytes,
            accepted_frames: state.accepted_frames,
            dropped_by_backpressure: state.dropped_by_backpressure,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(sequence: u64, captured_at_ns: u64, marker: u8) -> CapturedFrame {
        CapturedFrame {
            sequence,
            captured_at_ns,
            width: 2,
            height: 2,
            stride: 8,
            rgba: vec![marker; 16].into_boxed_slice(),
        }
    }

    #[test]
    fn full_queue_keeps_the_pending_head_and_latest_frames() {
        let pipeline = RecordingPipeline::default();
        for sequence in 0..3 {
            assert!(matches!(
                pipeline.push(frame(sequence, 100 + sequence * 10, sequence as u8)),
                Ok(PushOutcome::Queued { .. })
            ));
        }
        assert_eq!(
            pipeline.push(frame(3, 130, 3)).unwrap(),
            PushOutcome::QueuedAfterDropping {
                presentation_at_ns: 30,
                dropped_sequence: 1,
            }
        );
        let stats = pipeline.stats().unwrap();
        assert_eq!(stats.queued_frames, 3);
        assert_eq!(stats.queued_bytes, 48);
        assert_eq!(stats.accepted_frames, 4);
        assert_eq!(stats.dropped_by_backpressure, 1);
        let sequences = [
            pipeline.pop().unwrap().unwrap().frame.sequence,
            pipeline.pop().unwrap().unwrap().frame.sequence,
            pipeline.pop().unwrap().unwrap().frame.sequence,
        ];
        assert_eq!(sequences, [0, 2, 3]);
    }

    #[test]
    fn pause_is_removed_from_presentation_time_without_counting_drops() {
        let pipeline = RecordingPipeline::default();
        assert_eq!(
            pipeline.push(frame(10, 100, 0)).unwrap(),
            PushOutcome::Queued {
                presentation_at_ns: 0
            }
        );
        assert_eq!(
            pipeline.push(frame(11, 120, 0)).unwrap(),
            PushOutcome::Queued {
                presentation_at_ns: 20
            }
        );
        pipeline.pause(130).unwrap();
        assert_eq!(
            pipeline.push(frame(12, 500, 0)).unwrap(),
            PushOutcome::IgnoredWhilePaused
        );
        pipeline.resume(1_130).unwrap();
        assert_eq!(
            pipeline.push(frame(13, 1_150, 0)).unwrap(),
            PushOutcome::Queued {
                presentation_at_ns: 50
            }
        );
        let stats = pipeline.stats().unwrap();
        assert_eq!(stats.accepted_frames, 3);
        assert_eq!(stats.dropped_by_backpressure, 0);
    }

    #[test]
    fn invalid_frame_does_not_advance_sequence_geometry_or_timeline() {
        let pipeline = RecordingPipeline::default();
        let invalid = CapturedFrame {
            sequence: 1,
            captured_at_ns: 100,
            width: 2,
            height: 2,
            stride: 7,
            rgba: vec![0; 14].into_boxed_slice(),
        };
        assert_eq!(
            pipeline.push(invalid),
            Err(PipelineError::Frame(FrameError::InvalidStride))
        );
        assert!(pipeline.push(frame(1, 100, 0)).is_ok());

        let changed = CapturedFrame {
            sequence: 2,
            captured_at_ns: 110,
            width: 1,
            height: 2,
            stride: 4,
            rgba: vec![0; 8].into_boxed_slice(),
        };
        assert_eq!(pipeline.push(changed), Err(PipelineError::GeometryChanged));
        assert!(pipeline.push(frame(2, 110, 0)).is_ok());
    }

    #[test]
    fn stale_sequence_or_timestamp_can_retry_with_the_same_valid_identity() {
        let pipeline = RecordingPipeline::default();
        pipeline.push(frame(4, 1_000, 0)).unwrap();
        assert_eq!(
            pipeline.push(frame(4, 1_010, 0)),
            Err(PipelineError::SequenceNotIncreasing)
        );
        assert_eq!(
            pipeline.push(frame(5, 999, 0)),
            Err(PipelineError::Timeline(
                TimelineError::SourceTimestampNotIncreasing
            ))
        );
        assert_eq!(
            pipeline.push(frame(5, 1_010, 0)).unwrap(),
            PushOutcome::Queued {
                presentation_at_ns: 10
            }
        );
    }

    #[test]
    fn oversized_frame_is_rejected_before_buffer_length() {
        let pipeline = RecordingPipeline::default();
        let oversized = CapturedFrame {
            sequence: 0,
            captured_at_ns: 0,
            width: 8_192,
            height: 8_192,
            stride: 32_768,
            rgba: Vec::new().into_boxed_slice(),
        };
        assert_eq!(
            pipeline.push(oversized),
            Err(PipelineError::Frame(FrameError::FrameTooLarge))
        );
    }

    #[test]
    fn invalid_pause_transitions_do_not_change_the_timeline() {
        let pipeline = RecordingPipeline::default();
        assert_eq!(
            pipeline.pause(10),
            Err(PipelineError::Timeline(
                TimelineError::PauseBeforeFirstFrame
            ))
        );
        pipeline.push(frame(0, 100, 0)).unwrap();
        pipeline.pause(110).unwrap();
        assert_eq!(
            pipeline.pause(120),
            Err(PipelineError::Timeline(TimelineError::AlreadyPaused))
        );
        assert_eq!(
            pipeline.resume(110),
            Err(PipelineError::Timeline(
                TimelineError::SourceTimestampNotIncreasing
            ))
        );
        pipeline.resume(210).unwrap();
        assert_eq!(
            pipeline.resume(220),
            Err(PipelineError::Timeline(TimelineError::NotPaused))
        );
        assert_eq!(
            pipeline.push(frame(1, 220, 0)).unwrap(),
            PushOutcome::Queued {
                presentation_at_ns: 20
            }
        );
    }
}
