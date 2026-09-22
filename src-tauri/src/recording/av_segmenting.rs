//! 周期封尾的 VP9 + Opus 双轨 writer。
//!
//! 最终文件贯穿一个视频/音频 encoder；每个恢复分段使用独立 Opus encoder，并复用在边界强制
//! keyframe 的全局 VP9 packet。全部 PCM 先映射到 48 kHz frame index，空洞写零，因此 schema v2
//! 的真实 frame 统计可以跨分段精确相加。

use super::audio::{AudioFormat, CapturedAudioChunk, QueuedAudioChunk, AUDIO_SAMPLE_RATE_HZ};
use super::manifest::{PendingFinalOutput, PendingSegment, RecordingJournal, RecordingTrackStats};
use super::mux::av_interleaver::AvPacketInterleaver;
use super::mux::opus_webm::{OpusEncoderStats, OpusPacketEncoder, OpusTrackConfig, OpusWebmError};
use super::mux::vp9_webm::{Vp9PacketEncoder, Vp9WebmError};
use super::pipeline::{PipelineError, RecordingPipeline};
use super::segmenting::RecordingCommittedOutputs;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;

const MAX_SEGMENT_DURATION_NS: u64 = 120_000_000_000;
const AUDIO_BLOCK_FRAMES: u64 = 4_800;
const NANOS_PER_SECOND: u128 = 1_000_000_000;

#[derive(Debug, Error)]
pub(super) enum SegmentedAvRecordingError {
    #[error("双轨录屏周期分段配置无效")]
    InvalidConfiguration,
    #[error("双轨录屏时间线无效")]
    InvalidTimeline,
    #[error("双轨录屏统计溢出或不一致")]
    StatisticsMismatch,
    #[error("双轨录屏 journal 失败: {0}")]
    Journal(String),
    #[error(transparent)]
    Pipeline(#[from] PipelineError),
    #[error(transparent)]
    Vp9(#[from] Vp9WebmError),
    #[error(transparent)]
    Opus(#[from] OpusWebmError),
}

pub(super) struct PendingAvRecordingCompletion {
    journal: Option<RecordingJournal>,
    segment_paths: Vec<PathBuf>,
    final_output_path: Option<PathBuf>,
    settled: bool,
}

impl PendingAvRecordingCompletion {
    pub fn complete(mut self) -> Result<RecordingCommittedOutputs, String> {
        self.journal
            .as_mut()
            .ok_or_else(|| "双轨录屏 journal 已经被消费".to_string())?
            .complete()?;
        self.settled = true;
        Ok(RecordingCommittedOutputs {
            segment_paths: std::mem::take(&mut self.segment_paths),
            final_output_path: self.final_output_path.take(),
        })
    }
}

impl Drop for PendingAvRecordingCompletion {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        if let Some(journal) = self.journal.as_mut() {
            if let Err(error) = journal.interrupt() {
                log::warn!("回收未确认双轨录屏清单时写入 interrupted 状态失败: {error}");
            }
        }
    }
}

pub(super) struct SegmentedAvRecordingOutput {
    pub completion: PendingAvRecordingCompletion,
    pub video_frame_count: u64,
    pub audio_packet_count: u64,
    pub audio_pcm_frame_count: u64,
    pub duration_ns: u64,
}

pub(super) struct SegmentedAvRecordingWriter {
    journal: Option<RecordingJournal>,
    pending_segment: Option<PendingSegment>,
    pending_final: Option<PendingFinalOutput>,
    segment_mux: Option<AvPacketInterleaver<File>>,
    final_mux: Option<AvPacketInterleaver<File>>,
    video_encoder: Vp9PacketEncoder,
    global_audio_encoder: Option<OpusPacketEncoder>,
    segment_audio_encoder: Option<OpusPacketEncoder>,
    audio_track: OpusTrackConfig,
    video_pipeline: Arc<RecordingPipeline>,
    width: u32,
    height: u32,
    frames_per_second: u32,
    channels: u16,
    segment_duration_ns: u64,
    segment_started_at_ns: u64,
    segment_started_at_audio_frame: u64,
    audio_frame_cursor: u64,
    segment_has_video: bool,
    committed_video_frames: u64,
    committed_audio_frames: u64,
    segment_paths: Vec<PathBuf>,
    final_output_path: Option<PathBuf>,
    settled: bool,
}

impl SegmentedAvRecordingWriter {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mut journal: RecordingJournal,
        video_pipeline: Arc<RecordingPipeline>,
        width: u32,
        height: u32,
        frames_per_second: u32,
        channels: u16,
        segment_duration_ns: u64,
        global_audio_encoder: OpusPacketEncoder,
    ) -> Result<Self, SegmentedAvRecordingError> {
        if width == 0
            || height == 0
            || frames_per_second == 0
            || !(1..=2).contains(&channels)
            || segment_duration_ns == 0
            || segment_duration_ns > MAX_SEGMENT_DURATION_NS
            || global_audio_encoder.track_config().channels != channels
        {
            let _ = journal.interrupt();
            return Err(SegmentedAvRecordingError::InvalidConfiguration);
        }
        let audio_track = global_audio_encoder.track_config().clone();
        let segment_audio_encoder = OpusPacketEncoder::new(channels)?;
        if segment_audio_encoder.track_config() != &audio_track {
            let _ = journal.interrupt();
            return Err(SegmentedAvRecordingError::InvalidConfiguration);
        }
        let (segment_file, pending_segment) = journal
            .begin_segment()
            .map_err(SegmentedAvRecordingError::Journal)?;
        let (final_file, pending_final) = match journal.begin_final_output() {
            Ok(output) => output,
            Err(error) => {
                drop(pending_segment);
                let _ = journal.interrupt();
                return Err(SegmentedAvRecordingError::Journal(error));
            }
        };
        let initialized = (|| {
            Ok::<_, SegmentedAvRecordingError>((
                Vp9PacketEncoder::new(width, height, frames_per_second, 1)?,
                AvPacketInterleaver::new(segment_file, width, height, &audio_track)?,
                AvPacketInterleaver::new(final_file, width, height, &audio_track)?,
            ))
        })();
        let (video_encoder, segment_mux, final_mux) = match initialized {
            Ok(initialized) => initialized,
            Err(error) => {
                drop(pending_final);
                drop(pending_segment);
                let _ = journal.interrupt();
                return Err(error);
            }
        };
        Ok(Self {
            journal: Some(journal),
            pending_segment: Some(pending_segment),
            pending_final: Some(pending_final),
            segment_mux: Some(segment_mux),
            final_mux: Some(final_mux),
            video_encoder,
            global_audio_encoder: Some(global_audio_encoder),
            segment_audio_encoder: Some(segment_audio_encoder),
            audio_track,
            video_pipeline,
            width,
            height,
            frames_per_second,
            channels,
            segment_duration_ns,
            segment_started_at_ns: 0,
            segment_started_at_audio_frame: 0,
            audio_frame_cursor: 0,
            segment_has_video: false,
            committed_video_frames: 0,
            committed_audio_frames: 0,
            segment_paths: Vec::new(),
            final_output_path: None,
            settled: false,
        })
    }

    pub fn audio_track(&self) -> &OpusTrackConfig {
        &self.audio_track
    }

    pub fn push_video(
        &mut self,
        rgba: &[u8],
        presentation_at_ns: u64,
    ) -> Result<(), SegmentedAvRecordingError> {
        let elapsed = presentation_at_ns
            .checked_sub(self.segment_started_at_ns)
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?;
        if self.segment_has_video && elapsed >= self.segment_duration_ns {
            self.rotate_at(presentation_at_ns)?;
        }
        self.push_video_into_encoder(rgba, presentation_at_ns)?;
        self.segment_has_video = true;
        self.flush_ready()?;
        Ok(())
    }

    pub fn push_audio(
        &mut self,
        queued: QueuedAudioChunk,
    ) -> Result<(), SegmentedAvRecordingError> {
        if queued.chunk.format != AudioFormat::normalized(self.channels) {
            return Err(SegmentedAvRecordingError::InvalidConfiguration);
        }
        let start_frame = timestamp_to_audio_frame(queued.presentation_at_ns)?;
        if start_frame < self.audio_frame_cursor {
            return Err(SegmentedAvRecordingError::InvalidTimeline);
        }
        self.pad_audio_to_frame(start_frame)?;
        self.feed_audio_samples(&queued.chunk.samples, u64::from(queued.chunk.frame_count))?;
        self.flush_ready()?;
        Ok(())
    }

    pub fn finish(
        mut self,
        duration_ns: u64,
    ) -> Result<SegmentedAvRecordingOutput, SegmentedAvRecordingError> {
        if !self.segment_has_video || duration_ns <= self.segment_started_at_ns {
            return Err(SegmentedAvRecordingError::InvalidTimeline);
        }
        self.pad_audio_until(duration_ns)?;
        self.finish_video_encoder(duration_ns)?;

        let global_audio = self
            .global_audio_encoder
            .take()
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?
            .finish()?;
        for packet in global_audio.packets {
            self.final_mux_mut()?.enqueue_audio(packet)?;
        }
        let segment_audio = self
            .segment_audio_encoder
            .take()
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?
            .finish()?;
        for packet in segment_audio.packets {
            self.segment_mux_mut()?.enqueue_audio(packet)?;
        }

        let local_duration_ns = duration_ns
            .checked_sub(self.segment_started_at_ns)
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?;
        self.commit_current_segment(local_duration_ns, segment_audio.stats)?;
        if global_audio.stats.real_frames != self.committed_audio_frames {
            return Err(SegmentedAvRecordingError::StatisticsMismatch);
        }
        let final_mux = self
            .final_mux
            .take()
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?;
        let final_output = final_mux.finish(duration_ns)?;
        if final_output.video_frame_count != self.committed_video_frames
            || final_output.audio_packet_count != global_audio.stats.packet_count
            || final_output.duration_ns != duration_ns
        {
            return Err(SegmentedAvRecordingError::StatisticsMismatch);
        }
        let pending_final = self
            .pending_final
            .take()
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?;
        let final_path = self
            .journal
            .as_mut()
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?
            .commit_final_output_with_tracks(
                pending_final,
                final_output.writer,
                duration_ns,
                RecordingTrackStats::with_audio(
                    final_output.video_frame_count,
                    final_output.audio_packet_count,
                    global_audio.stats.real_frames,
                ),
            )
            .map_err(SegmentedAvRecordingError::Journal)?;
        self.final_output_path = Some(final_path);

        let completion = PendingAvRecordingCompletion {
            journal: self.journal.take(),
            segment_paths: std::mem::take(&mut self.segment_paths),
            final_output_path: self.final_output_path.take(),
            settled: false,
        };
        self.settled = true;
        Ok(SegmentedAvRecordingOutput {
            completion,
            video_frame_count: final_output.video_frame_count,
            audio_packet_count: final_output.audio_packet_count,
            audio_pcm_frame_count: global_audio.stats.real_frames,
            duration_ns,
        })
    }

    fn rotate_at(&mut self, boundary_ns: u64) -> Result<(), SegmentedAvRecordingError> {
        self.pad_audio_until(boundary_ns)?;
        self.flush_video_until(boundary_ns)?;
        let segment_audio = self
            .segment_audio_encoder
            .take()
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?
            .finish()?;
        for packet in segment_audio.packets {
            self.segment_mux_mut()?.enqueue_audio(packet)?;
        }
        let duration_ns = boundary_ns
            .checked_sub(self.segment_started_at_ns)
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?;
        self.commit_current_segment(duration_ns, segment_audio.stats)?;

        let (file, pending) = self
            .journal
            .as_ref()
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?
            .begin_segment()
            .map_err(SegmentedAvRecordingError::Journal)?;
        let segment_audio_encoder = OpusPacketEncoder::new(self.channels)?;
        if segment_audio_encoder.track_config() != &self.audio_track {
            return Err(SegmentedAvRecordingError::InvalidConfiguration);
        }
        self.segment_mux = Some(AvPacketInterleaver::new(
            file,
            self.width,
            self.height,
            &self.audio_track,
        )?);
        self.pending_segment = Some(pending);
        self.segment_audio_encoder = Some(segment_audio_encoder);
        self.video_encoder.force_next_keyframe();
        self.segment_started_at_ns = boundary_ns;
        self.segment_started_at_audio_frame = self.audio_frame_cursor;
        self.segment_has_video = false;
        Ok(())
    }

    fn commit_current_segment(
        &mut self,
        duration_ns: u64,
        audio: OpusEncoderStats,
    ) -> Result<(), SegmentedAvRecordingError> {
        let segment_mux = self
            .segment_mux
            .take()
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?;
        let output = segment_mux.finish(duration_ns)?;
        let expected_audio_frames = self
            .audio_frame_cursor
            .checked_sub(self.segment_started_at_audio_frame)
            .ok_or(SegmentedAvRecordingError::StatisticsMismatch)?;
        if output.duration_ns != duration_ns
            || output.audio_packet_count != audio.packet_count
            || audio.real_frames != expected_audio_frames
        {
            return Err(SegmentedAvRecordingError::StatisticsMismatch);
        }
        let pending = self
            .pending_segment
            .take()
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?;
        let dropped_frames = self.video_pipeline.stats()?.dropped_by_backpressure;
        let path = self
            .journal
            .as_mut()
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?
            .commit_segment_with_tracks(
                pending,
                output.writer,
                duration_ns,
                RecordingTrackStats::with_audio(
                    output.video_frame_count,
                    output.audio_packet_count,
                    audio.real_frames,
                ),
                dropped_frames,
            )
            .map_err(SegmentedAvRecordingError::Journal)?;
        self.committed_video_frames = self
            .committed_video_frames
            .checked_add(output.video_frame_count)
            .ok_or(SegmentedAvRecordingError::StatisticsMismatch)?;
        self.committed_audio_frames = self
            .committed_audio_frames
            .checked_add(audio.real_frames)
            .ok_or(SegmentedAvRecordingError::StatisticsMismatch)?;
        self.segment_paths.push(path);
        Ok(())
    }

    fn pad_audio_until(&mut self, timestamp_ns: u64) -> Result<(), SegmentedAvRecordingError> {
        self.pad_audio_to_frame(timestamp_to_audio_frame(timestamp_ns)?)
    }

    fn pad_audio_to_frame(&mut self, target_frame: u64) -> Result<(), SegmentedAvRecordingError> {
        if target_frame < self.audio_frame_cursor {
            return Err(SegmentedAvRecordingError::InvalidTimeline);
        }
        let mut remaining = target_frame - self.audio_frame_cursor;
        while remaining > 0 {
            let frames = remaining.min(AUDIO_BLOCK_FRAMES);
            let sample_count = usize::try_from(frames)
                .ok()
                .and_then(|frames| frames.checked_mul(usize::from(self.channels)))
                .ok_or(SegmentedAvRecordingError::StatisticsMismatch)?;
            self.feed_audio_samples(&vec![0.0; sample_count], frames)?;
            remaining -= frames;
        }
        Ok(())
    }

    fn feed_audio_samples(
        &mut self,
        samples: &[f32],
        frame_count: u64,
    ) -> Result<(), SegmentedAvRecordingError> {
        if frame_count == 0 || frame_count > AUDIO_BLOCK_FRAMES {
            return Err(SegmentedAvRecordingError::InvalidConfiguration);
        }
        let frame_count_u32 = u32::try_from(frame_count)
            .map_err(|_| SegmentedAvRecordingError::StatisticsMismatch)?;
        let expected_samples = usize::try_from(frame_count)
            .ok()
            .and_then(|frames| frames.checked_mul(usize::from(self.channels)))
            .ok_or(SegmentedAvRecordingError::StatisticsMismatch)?;
        if samples.len() != expected_samples || samples.iter().any(|sample| !sample.is_finite()) {
            return Err(SegmentedAvRecordingError::InvalidConfiguration);
        }
        let global_start_frame = self.audio_frame_cursor;
        let local_start_frame = global_start_frame
            .checked_sub(self.segment_started_at_audio_frame)
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?;
        let global = queued_audio(
            self.channels,
            global_start_frame,
            frame_count_u32,
            samples.to_vec().into_boxed_slice(),
        )?;
        let local = queued_audio(
            self.channels,
            local_start_frame,
            frame_count_u32,
            samples.to_vec().into_boxed_slice(),
        )?;

        let global_packets = self
            .global_audio_encoder
            .as_mut()
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?
            .push(global)?;
        for packet in global_packets {
            self.final_mux_mut()?.enqueue_audio(packet)?;
        }
        let segment_packets = self
            .segment_audio_encoder
            .as_mut()
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?
            .push(local)?;
        for packet in segment_packets {
            self.segment_mux_mut()?.enqueue_audio(packet)?;
        }
        self.audio_frame_cursor = self
            .audio_frame_cursor
            .checked_add(frame_count)
            .ok_or(SegmentedAvRecordingError::StatisticsMismatch)?;
        self.flush_ready()?;
        Ok(())
    }

    fn push_video_into_encoder(
        &mut self,
        rgba: &[u8],
        presentation_at_ns: u64,
    ) -> Result<(), SegmentedAvRecordingError> {
        let segment_base = self.segment_started_at_ns;
        let global_audio_frontier = self.global_audio_frontier()?;
        let segment_audio_frontier = self.segment_audio_frontier()?;
        let (video_encoder, final_mux, segment_mux) = (
            &mut self.video_encoder,
            self.final_mux
                .as_mut()
                .ok_or(SegmentedAvRecordingError::InvalidTimeline)?,
            self.segment_mux
                .as_mut()
                .ok_or(SegmentedAvRecordingError::InvalidTimeline)?,
        );
        let mut interleave_error = None;
        let result = video_encoder.push_rgba(
            rgba,
            presentation_at_ns,
            &mut |data, timestamp_ns, keyframe| {
                let local_timestamp = timestamp_ns
                    .checked_sub(segment_base)
                    .ok_or(Vp9WebmError::InvalidTimestamp)?;
                if let Err(error) = final_mux
                    .enqueue_video(data, timestamp_ns, keyframe)
                    .and_then(|_| final_mux.flush_ready(timestamp_ns, global_audio_frontier))
                    .and_then(|_| segment_mux.enqueue_video(data, local_timestamp, keyframe))
                    .and_then(|_| segment_mux.flush_ready(local_timestamp, segment_audio_frontier))
                {
                    interleave_error = Some(error);
                    return Err(Vp9WebmError::Finalize);
                }
                Ok(())
            },
        );
        if let Some(error) = interleave_error {
            return Err(error.into());
        }
        result?;
        Ok(())
    }

    fn flush_video_until(
        &mut self,
        presentation_at_ns: u64,
    ) -> Result<(), SegmentedAvRecordingError> {
        let segment_base = self.segment_started_at_ns;
        let global_audio_frontier = self.global_audio_frontier()?;
        let segment_audio_frontier = self.segment_audio_frontier()?;
        let (video_encoder, final_mux, segment_mux) = (
            &mut self.video_encoder,
            self.final_mux
                .as_mut()
                .ok_or(SegmentedAvRecordingError::InvalidTimeline)?,
            self.segment_mux
                .as_mut()
                .ok_or(SegmentedAvRecordingError::InvalidTimeline)?,
        );
        let mut interleave_error = None;
        let result =
            video_encoder.flush_until(presentation_at_ns, &mut |data, timestamp_ns, keyframe| {
                let local_timestamp = timestamp_ns
                    .checked_sub(segment_base)
                    .ok_or(Vp9WebmError::InvalidTimestamp)?;
                if let Err(error) = final_mux
                    .enqueue_video(data, timestamp_ns, keyframe)
                    .and_then(|_| final_mux.flush_ready(timestamp_ns, global_audio_frontier))
                    .and_then(|_| segment_mux.enqueue_video(data, local_timestamp, keyframe))
                    .and_then(|_| segment_mux.flush_ready(local_timestamp, segment_audio_frontier))
                {
                    interleave_error = Some(error);
                    return Err(Vp9WebmError::Finalize);
                }
                Ok(())
            });
        if let Some(error) = interleave_error {
            return Err(error.into());
        }
        result?;
        self.flush_ready()
    }

    fn finish_video_encoder(&mut self, duration_ns: u64) -> Result<(), SegmentedAvRecordingError> {
        let segment_base = self.segment_started_at_ns;
        let global_audio_frontier = self.global_audio_frontier()?;
        let segment_audio_frontier = self.segment_audio_frontier()?;
        let (video_encoder, final_mux, segment_mux) = (
            &mut self.video_encoder,
            self.final_mux
                .as_mut()
                .ok_or(SegmentedAvRecordingError::InvalidTimeline)?,
            self.segment_mux
                .as_mut()
                .ok_or(SegmentedAvRecordingError::InvalidTimeline)?,
        );
        let mut interleave_error = None;
        let result = video_encoder.finish(duration_ns, &mut |data, timestamp_ns, keyframe| {
            let local_timestamp = timestamp_ns
                .checked_sub(segment_base)
                .ok_or(Vp9WebmError::InvalidTimestamp)?;
            if let Err(error) = final_mux
                .enqueue_video(data, timestamp_ns, keyframe)
                .and_then(|_| final_mux.flush_ready(timestamp_ns, global_audio_frontier))
                .and_then(|_| segment_mux.enqueue_video(data, local_timestamp, keyframe))
                .and_then(|_| segment_mux.flush_ready(local_timestamp, segment_audio_frontier))
            {
                interleave_error = Some(error);
                return Err(Vp9WebmError::Finalize);
            }
            Ok(())
        });
        if let Some(error) = interleave_error {
            return Err(error.into());
        }
        result?;
        Ok(())
    }

    fn flush_ready(&mut self) -> Result<(), SegmentedAvRecordingError> {
        let video_frontier = self.video_encoder.next_timestamp_ns()?;
        let global_audio_frontier = self.global_audio_frontier()?;
        let segment_video_frontier = video_frontier
            .checked_sub(self.segment_started_at_ns)
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?;
        let segment_audio_frontier = self.segment_audio_frontier()?;
        self.final_mux_mut()?
            .flush_ready(video_frontier, global_audio_frontier)?;
        self.segment_mux_mut()?
            .flush_ready(segment_video_frontier, segment_audio_frontier)?;
        Ok(())
    }

    fn global_audio_frontier(&self) -> Result<u64, SegmentedAvRecordingError> {
        Ok(self
            .global_audio_encoder
            .as_ref()
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?
            .next_packet_timestamp_ns(0)?)
    }

    fn segment_audio_frontier(&self) -> Result<u64, SegmentedAvRecordingError> {
        Ok(self
            .segment_audio_encoder
            .as_ref()
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)?
            .next_packet_timestamp_ns(0)?)
    }

    fn segment_mux_mut(
        &mut self,
    ) -> Result<&mut AvPacketInterleaver<File>, SegmentedAvRecordingError> {
        self.segment_mux
            .as_mut()
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)
    }

    fn final_mux_mut(
        &mut self,
    ) -> Result<&mut AvPacketInterleaver<File>, SegmentedAvRecordingError> {
        self.final_mux
            .as_mut()
            .ok_or(SegmentedAvRecordingError::InvalidTimeline)
    }
}

impl Drop for SegmentedAvRecordingWriter {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        drop(self.segment_mux.take());
        drop(self.final_mux.take());
        drop(self.pending_segment.take());
        drop(self.pending_final.take());
        drop(self.global_audio_encoder.take());
        drop(self.segment_audio_encoder.take());
        if let Some(journal) = self.journal.as_mut() {
            if let Err(error) = journal.interrupt() {
                log::warn!("回收双轨录屏周期分段时写入 interrupted 状态失败: {error}");
            }
        }
    }
}

fn queued_audio(
    channels: u16,
    start_frame: u64,
    frame_count: u32,
    samples: Box<[f32]>,
) -> Result<QueuedAudioChunk, SegmentedAvRecordingError> {
    let presentation_at_ns = audio_frame_to_ns(start_frame)?;
    let duration_ns = audio_frame_to_ns(u64::from(frame_count))?;
    Ok(QueuedAudioChunk {
        chunk: CapturedAudioChunk {
            sequence: start_frame,
            captured_at_ns: presentation_at_ns,
            format: AudioFormat::normalized(channels),
            frame_count,
            samples,
        },
        presentation_at_ns,
        duration_ns,
        gap_before_ns: 0,
    })
}

fn timestamp_to_audio_frame(timestamp_ns: u64) -> Result<u64, SegmentedAvRecordingError> {
    u128::from(timestamp_ns)
        .checked_mul(u128::from(AUDIO_SAMPLE_RATE_HZ))
        .and_then(|value| value.checked_add(NANOS_PER_SECOND / 2))
        .and_then(|value| value.checked_div(NANOS_PER_SECOND))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(SegmentedAvRecordingError::StatisticsMismatch)
}

fn audio_frame_to_ns(frame: u64) -> Result<u64, SegmentedAvRecordingError> {
    u128::from(frame)
        .checked_mul(NANOS_PER_SECOND)
        .and_then(|value| value.checked_div(u128::from(AUDIO_SAMPLE_RATE_HZ)))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(SegmentedAvRecordingError::StatisticsMismatch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::manifest::{RecordingJournalAudioConfig, RecordingJournalConfig};
    use serde_json::Value;
    use std::fs;
    use std::process::Command;

    fn journal_config(session_id: &str, audio: &OpusTrackConfig) -> RecordingJournalConfig {
        RecordingJournalConfig {
            session_id: session_id.to_string(),
            source_id: "fixture-av".to_string(),
            physical_x: 0,
            physical_y: 0,
            width: 2,
            height: 2,
            target_fps_numerator: 10,
            target_fps_denominator: 1,
            encoder: "vp9-prototype".to_string(),
            container: "webm".to_string(),
            include_cursor: false,
            audio: Some(RecordingJournalAudioConfig {
                sample_rate_hz: AUDIO_SAMPLE_RATE_HZ,
                channels: audio.channels,
                encoder: "opus".to_string(),
                pre_skip_frames: audio.pre_skip_frames,
                codec_delay_ns: audio.codec_delay_ns,
                seek_pre_roll_ns: audio.seek_pre_roll_ns,
            }),
        }
    }

    fn writer(
        root: &std::path::Path,
        session_id: &str,
        segment_duration_ns: u64,
    ) -> (SegmentedAvRecordingWriter, PathBuf) {
        let audio = OpusPacketEncoder::new(2).unwrap();
        let journal =
            RecordingJournal::create(root, journal_config(session_id, audio.track_config()))
                .unwrap();
        let session = journal.session_directory().to_path_buf();
        let video_pipeline = Arc::new(RecordingPipeline::default());
        (
            SegmentedAvRecordingWriter::new(
                journal,
                video_pipeline,
                2,
                2,
                10,
                2,
                segment_duration_ns,
                audio,
            )
            .unwrap(),
            session,
        )
    }

    fn rgba(marker: u8) -> [u8; 16] {
        [marker; 16]
    }

    fn pcm(start_frame: u64, frames: u32) -> QueuedAudioChunk {
        queued_audio(
            2,
            start_frame,
            frames,
            vec![0.1; frames as usize * 2].into_boxed_slice(),
        )
        .unwrap()
    }

    fn manifest(session: &std::path::Path) -> Value {
        serde_json::from_slice(&fs::read(session.join("manifest.json")).unwrap()).unwrap()
    }

    fn assert_segment_starts_with_zero_keyframe(path: &std::path::Path) {
        if Command::new("ffprobe").arg("-version").output().is_err() {
            return;
        }
        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "packet=pts_time,flags",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .unwrap();
        assert!(probe.status.success());
        let value: Value = serde_json::from_slice(&probe.stdout).unwrap();
        let first = &value["packets"][0];
        assert_eq!(first["pts_time"], "0.000000");
        assert!(first["flags"].as_str().unwrap().contains('K'));
    }

    #[test]
    fn writes_independent_recovery_segments_and_one_continuous_final_file() {
        let temporary = tempfile::tempdir().unwrap();
        let (mut writer, session) = writer(temporary.path(), "av-segments", 200_000_000);

        writer.push_video(&rgba(1), 0).unwrap();
        writer.push_audio(pcm(0, 4_800)).unwrap();
        writer.push_video(&rgba(2), 100_000_000).unwrap();
        writer.push_audio(pcm(4_800, 4_800)).unwrap();
        writer.push_video(&rgba(3), 200_000_000).unwrap();
        writer.push_audio(pcm(9_600, 4_800)).unwrap();
        writer.push_video(&rgba(4), 300_000_000).unwrap();
        writer.push_audio(pcm(14_400, 4_800)).unwrap();

        let output = writer.finish(400_000_000).unwrap();
        assert_eq!(output.video_frame_count, 4);
        assert_eq!(output.audio_pcm_frame_count, 19_200);
        assert!(output.audio_packet_count > 0);
        let committed = output.completion.complete().unwrap();
        assert_eq!(committed.segment_paths.len(), 2);
        assert!(committed.final_output_path.is_some());

        let value = manifest(&session);
        assert_eq!(value["schemaVersion"], 2);
        assert_eq!(value["state"], "complete");
        let segments = value["segments"].as_array().unwrap();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0]["audio"]["pcmFrameCount"], 9_600);
        assert_eq!(segments[1]["audio"]["pcmFrameCount"], 9_600);
        assert_eq!(value["finalOutput"]["audio"]["pcmFrameCount"], 19_200);
        for path in &committed.segment_paths {
            assert_segment_starts_with_zero_keyframe(path);
        }
        for path in committed
            .segment_paths
            .iter()
            .chain(committed.final_output_path.iter())
        {
            let bytes = fs::read(path).unwrap();
            assert!(bytes.windows(5).any(|window| window == b"V_VP9"));
            assert!(bytes.windows(6).any(|window| window == b"A_OPUS"));
            assert!(bytes.windows(8).any(|window| window == b"OpusHead"));
        }
    }

    #[test]
    fn fills_leading_internal_and_trailing_gaps_without_counting_codec_padding() {
        let temporary = tempfile::tempdir().unwrap();
        let (mut writer, session) = writer(temporary.path(), "av-silence", 1_000_000_000);
        writer.push_video(&rgba(1), 0).unwrap();
        writer.push_audio(pcm(2_400, 960)).unwrap();
        writer.push_video(&rgba(2), 100_000_000).unwrap();
        writer.push_audio(pcm(4_800, 960)).unwrap();

        let output = writer.finish(200_000_000).unwrap();
        assert_eq!(output.audio_pcm_frame_count, 9_600);
        output.completion.complete().unwrap();
        let value = manifest(&session);
        assert_eq!(value["segments"][0]["audio"]["pcmFrameCount"], 9_600);
        assert_eq!(value["finalOutput"]["audio"]["pcmFrameCount"], 9_600);
    }

    #[test]
    fn overlap_fails_and_drop_leaves_only_the_committed_prefix() {
        let temporary = tempfile::tempdir().unwrap();
        let (mut writer, session) = writer(temporary.path(), "av-interrupted", 200_000_000);
        writer.push_video(&rgba(1), 0).unwrap();
        writer.push_audio(pcm(0, 4_800)).unwrap();
        writer.push_video(&rgba(2), 100_000_000).unwrap();
        writer.push_audio(pcm(4_800, 4_800)).unwrap();
        writer.push_video(&rgba(3), 200_000_000).unwrap();
        assert!(matches!(
            writer.push_audio(pcm(9_599, 960)),
            Err(SegmentedAvRecordingError::InvalidTimeline)
        ));
        drop(writer);

        let value = manifest(&session);
        assert_eq!(value["state"], "interrupted");
        assert_eq!(value["segments"].as_array().unwrap().len(), 1);
        assert_eq!(value["segments"][0]["audio"]["pcmFrameCount"], 9_600);
        assert!(!session.join("segment-000001.webm.partial").exists());
        assert!(!session.join("recording.webm.partial").exists());
    }
}
