//! 视频与音频 pipeline 到周期双轨 writer 的单一编码 owner。
//!
//! 两个桥接线程各自最多持有一个未确认事件。编码线程取得两轨 head 后按 presentation 合并，
//! 在视频事件处拆分跨界 PCM，再由 writer 完成 packet 级有界重排和周期提交。

use super::audio::{AudioPipeline, AudioPipelineDrain, AudioPipelineError, QueuedAudioChunk};
use super::av_segmenting::{
    SegmentedAvRecordingError, SegmentedAvRecordingOutput, SegmentedAvRecordingWriter,
};
use super::av_timeline::{
    AudioEpochOutcome, AvFinishReport, AvTimelineCoordinator, AvTimelineError,
};
use super::pipeline::{PipelineDrain, PipelineError, QueuedFrame, RecordingPipeline};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use thiserror::Error;

const BRIDGE_WAIT_SLICE: Duration = Duration::from_millis(5);
const NANOS_PER_SECOND: u128 = 1_000_000_000;
const AUDIO_SAMPLE_RATE_HZ_U128: u128 = 48_000;

#[derive(Debug, Error)]
pub(super) enum AvEncoderWorkerError {
    #[error(transparent)]
    VideoPipeline(#[from] PipelineError),
    #[error(transparent)]
    AudioPipeline(#[from] AudioPipelineError),
    #[error(transparent)]
    Timeline(#[from] AvTimelineError),
    #[error(transparent)]
    Writer(#[from] SegmentedAvRecordingError),
    #[error("无法启动双轨录屏编码线程: {0}")]
    ThreadSpawn(String),
    #[error("双轨录屏编码线程异常退出")]
    ThreadPanicked,
    #[error("双轨 pipeline 桥接线程异常退出")]
    BridgePanicked,
    #[error("双轨 pipeline 桥接通道异常关闭")]
    BridgeDisconnected,
}

pub(super) struct AvEncoderReport {
    pub output: SegmentedAvRecordingOutput,
    pub video_input_frames: u64,
    pub audio_input_chunks: u64,
    pub audio_input_frames: u64,
    pub audio_dropped_before_video_frames: u64,
    pub audio_trimmed_before_video_frames: u64,
    pub video_duration_ns: u64,
    pub audio_session_duration_ns: u64,
    pub finish: AvFinishReport,
}

type VideoBridgeEvent = Result<PipelineDrain, PipelineError>;
type AudioBridgeEvent = Result<AudioPipelineDrain, AudioPipelineError>;

pub(super) struct AvEncoderWorker {
    video_pipeline: Arc<RecordingPipeline>,
    audio_pipeline: Arc<AudioPipeline>,
    join: Option<JoinHandle<Result<AvEncoderReport, AvEncoderWorkerError>>>,
}

impl AvEncoderWorker {
    pub fn spawn(
        writer: SegmentedAvRecordingWriter,
        video_pipeline: Arc<RecordingPipeline>,
        audio_pipeline: Arc<AudioPipeline>,
        session_origin_ns: u64,
    ) -> Result<Self, AvEncoderWorkerError> {
        let worker_video = Arc::clone(&video_pipeline);
        let worker_audio = Arc::clone(&audio_pipeline);
        let join = thread::Builder::new()
            .name("clippy-recording-av-encoder".to_string())
            .spawn(move || run(writer, worker_video, worker_audio, session_origin_ns))
            .map_err(|error| {
                let _ = video_pipeline.abort();
                let _ = audio_pipeline.abort();
                AvEncoderWorkerError::ThreadSpawn(error.to_string())
            })?;
        Ok(Self {
            video_pipeline,
            audio_pipeline,
            join: Some(join),
        })
    }

    pub fn wait(mut self) -> Result<AvEncoderReport, AvEncoderWorkerError> {
        self.join_inner()
    }

    fn join_inner(&mut self) -> Result<AvEncoderReport, AvEncoderWorkerError> {
        let Some(join) = self.join.take() else {
            return Err(AvEncoderWorkerError::ThreadPanicked);
        };
        join.join()
            .map_err(|_| AvEncoderWorkerError::ThreadPanicked)?
    }
}

impl Drop for AvEncoderWorker {
    fn drop(&mut self) {
        if let Some(join) = self.join.take() {
            let _ = self.video_pipeline.abort();
            let _ = self.audio_pipeline.abort();
            let _ = join.join();
        }
    }
}

fn run(
    writer: SegmentedAvRecordingWriter,
    video_pipeline: Arc<RecordingPipeline>,
    audio_pipeline: Arc<AudioPipeline>,
    session_origin_ns: u64,
) -> Result<AvEncoderReport, AvEncoderWorkerError> {
    let (video_sender, video_events) = mpsc::sync_channel(1);
    let (audio_sender, audio_events) = mpsc::sync_channel(1);
    let video_bridge_pipeline = Arc::clone(&video_pipeline);
    let video_bridge = thread::Builder::new()
        .name("clippy-recording-video-bridge".to_string())
        .spawn(move || bridge_video(video_bridge_pipeline, video_sender))
        .map_err(|error| AvEncoderWorkerError::ThreadSpawn(error.to_string()))?;
    let audio_bridge_pipeline = Arc::clone(&audio_pipeline);
    let audio_bridge = match thread::Builder::new()
        .name("clippy-recording-audio-bridge".to_string())
        .spawn(move || bridge_audio(audio_bridge_pipeline, audio_sender))
    {
        Ok(bridge) => bridge,
        Err(error) => {
            let _ = video_pipeline.abort();
            drop(video_events);
            let _ = video_bridge.join();
            return Err(AvEncoderWorkerError::ThreadSpawn(error.to_string()));
        }
    };

    let result = run_inner(writer, &video_events, &audio_events, session_origin_ns);
    if result.is_err() {
        let _ = video_pipeline.abort();
        let _ = audio_pipeline.abort();
    }
    drop(video_events);
    drop(audio_events);
    if video_bridge.join().is_err() || audio_bridge.join().is_err() {
        return Err(AvEncoderWorkerError::BridgePanicked);
    }
    result
}

fn bridge_video(pipeline: Arc<RecordingPipeline>, events: SyncSender<VideoBridgeEvent>) {
    loop {
        let event = pipeline.pop_wait();
        let terminal = !matches!(event, Ok(PipelineDrain::Frame(_)));
        if events.send(event).is_err() || terminal {
            return;
        }
    }
}

fn bridge_audio(pipeline: Arc<AudioPipeline>, events: SyncSender<AudioBridgeEvent>) {
    loop {
        let event = pipeline.pop_wait();
        let terminal = !matches!(event, Ok(AudioPipelineDrain::Chunk(_)));
        if events.send(event).is_err() || terminal {
            return;
        }
    }
}

fn run_inner(
    mut writer: SegmentedAvRecordingWriter,
    video_events: &Receiver<VideoBridgeEvent>,
    audio_events: &Receiver<AudioBridgeEvent>,
    session_origin_ns: u64,
) -> Result<AvEncoderReport, AvEncoderWorkerError> {
    let mut coordinator = AvTimelineCoordinator::new(session_origin_ns);
    let mut epoch_established = false;
    let mut video_head: Option<QueuedFrame> = None;
    let mut audio_head: Option<QueuedAudioChunk> = None;
    let mut aligned_audio: Option<QueuedAudioChunk> = None;
    let mut video_duration_ns = None;
    let mut audio_duration_ns = None;
    let mut video_input_frames = 0_u64;
    let mut audio_input_chunks = 0_u64;
    let mut audio_input_frames = 0_u64;
    let mut audio_dropped_before_video_frames = 0_u64;
    let mut audio_trimmed_before_video_frames = 0_u64;

    loop {
        fill_video_head(video_events, &mut video_head, &mut video_duration_ns)?;
        fill_audio_head(audio_events, &mut audio_head, &mut audio_duration_ns)?;

        if !epoch_established {
            if let Some(first_video) = video_head.as_ref() {
                coordinator.establish_video_epoch(first_video)?;
                epoch_established = true;
            } else if video_duration_ns.is_some() {
                return Err(AvTimelineError::EpochNotEstablished.into());
            }
        }

        if epoch_established && aligned_audio.is_none() {
            while let Some(raw) = audio_head.take() {
                audio_input_chunks = audio_input_chunks.saturating_add(1);
                audio_input_frames =
                    audio_input_frames.saturating_add(u64::from(raw.chunk.frame_count));
                match coordinator.align_audio(raw)? {
                    AudioEpochOutcome::DroppedBeforeVideo { frame_count, .. } => {
                        audio_dropped_before_video_frames = audio_dropped_before_video_frames
                            .saturating_add(u64::from(frame_count));
                        fill_audio_head(audio_events, &mut audio_head, &mut audio_duration_ns)?;
                    }
                    AudioEpochOutcome::Aligned {
                        chunk,
                        trimmed_leading_frames,
                    } => {
                        audio_trimmed_before_video_frames = audio_trimmed_before_video_frames
                            .saturating_add(u64::from(trimmed_leading_frames));
                        aligned_audio = Some(chunk);
                        break;
                    }
                }
            }
        }

        match (video_head.as_ref(), aligned_audio.as_ref()) {
            (Some(video), Some(audio)) if audio.presentation_at_ns < video.presentation_at_ns => {
                let video_timestamp = video.presentation_at_ns;
                let queued = aligned_audio.take().expect("匹配分支保证存在对齐音频");
                let (before, after) = split_audio_at(queued, video_timestamp)?;
                writer.push_audio(before)?;
                aligned_audio = after;
            }
            (Some(_), Some(_)) => {
                let video = video_head.take().expect("匹配分支保证存在视频");
                writer.push_video(&video.frame.rgba, video.presentation_at_ns)?;
                video_input_frames = video_input_frames.saturating_add(1);
            }
            (Some(_), None) if audio_duration_ns.is_some() => {
                let video = video_head.take().expect("匹配分支保证存在视频");
                writer.push_video(&video.frame.rgba, video.presentation_at_ns)?;
                video_input_frames = video_input_frames.saturating_add(1);
            }
            (None, Some(_)) if video_duration_ns.is_some() => {
                let audio = aligned_audio.take().expect("匹配分支保证存在音频");
                writer.push_audio(audio)?;
            }
            (None, None)
                if epoch_established
                    && video_duration_ns.is_some()
                    && audio_duration_ns.is_some() =>
            {
                let video_duration_ns = video_duration_ns.expect("已检查视频时长");
                let audio_session_duration_ns = audio_duration_ns.expect("已检查音频时长");
                let finish = coordinator.finish(video_duration_ns, audio_session_duration_ns)?;
                let output = writer.finish(finish.mux_duration_ns)?;
                return Ok(AvEncoderReport {
                    output,
                    video_input_frames,
                    audio_input_chunks,
                    audio_input_frames,
                    audio_dropped_before_video_frames,
                    audio_trimmed_before_video_frames,
                    video_duration_ns,
                    audio_session_duration_ns,
                    finish,
                });
            }
            _ => wait_for_missing_head(
                video_events,
                audio_events,
                &mut video_head,
                &mut audio_head,
                &mut video_duration_ns,
                &mut audio_duration_ns,
            )?,
        }
    }
}

fn fill_video_head(
    events: &Receiver<VideoBridgeEvent>,
    head: &mut Option<QueuedFrame>,
    duration_ns: &mut Option<u64>,
) -> Result<(), AvEncoderWorkerError> {
    if head.is_some() || duration_ns.is_some() {
        return Ok(());
    }
    match events.try_recv() {
        Ok(event) => apply_video_event(event, head, duration_ns),
        Err(TryRecvError::Empty) => Ok(()),
        Err(TryRecvError::Disconnected) => Err(AvEncoderWorkerError::BridgeDisconnected),
    }
}

fn fill_audio_head(
    events: &Receiver<AudioBridgeEvent>,
    head: &mut Option<QueuedAudioChunk>,
    duration_ns: &mut Option<u64>,
) -> Result<(), AvEncoderWorkerError> {
    if head.is_some() || duration_ns.is_some() {
        return Ok(());
    }
    match events.try_recv() {
        Ok(event) => apply_audio_event(event, head, duration_ns),
        Err(TryRecvError::Empty) => Ok(()),
        Err(TryRecvError::Disconnected) => Err(AvEncoderWorkerError::BridgeDisconnected),
    }
}

#[allow(clippy::too_many_arguments)]
fn wait_for_missing_head(
    video_events: &Receiver<VideoBridgeEvent>,
    audio_events: &Receiver<AudioBridgeEvent>,
    video_head: &mut Option<QueuedFrame>,
    audio_head: &mut Option<QueuedAudioChunk>,
    video_duration_ns: &mut Option<u64>,
    audio_duration_ns: &mut Option<u64>,
) -> Result<(), AvEncoderWorkerError> {
    if video_head.is_none() && video_duration_ns.is_none() {
        match video_events.recv_timeout(BRIDGE_WAIT_SLICE) {
            Ok(event) => apply_video_event(event, video_head, video_duration_ns)?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(AvEncoderWorkerError::BridgeDisconnected);
            }
        }
    }
    if audio_head.is_none() && audio_duration_ns.is_none() {
        match audio_events.recv_timeout(BRIDGE_WAIT_SLICE) {
            Ok(event) => apply_audio_event(event, audio_head, audio_duration_ns)?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(AvEncoderWorkerError::BridgeDisconnected);
            }
        }
    }
    Ok(())
}

fn apply_video_event(
    event: VideoBridgeEvent,
    head: &mut Option<QueuedFrame>,
    duration_ns: &mut Option<u64>,
) -> Result<(), AvEncoderWorkerError> {
    match event? {
        PipelineDrain::Frame(frame) => *head = Some(frame),
        PipelineDrain::Finished {
            duration_ns: finished,
        } => *duration_ns = Some(finished),
    }
    Ok(())
}

fn apply_audio_event(
    event: AudioBridgeEvent,
    head: &mut Option<QueuedAudioChunk>,
    duration_ns: &mut Option<u64>,
) -> Result<(), AvEncoderWorkerError> {
    match event? {
        AudioPipelineDrain::Chunk(chunk) => *head = Some(chunk),
        AudioPipelineDrain::Finished {
            duration_ns: finished,
        } => *duration_ns = Some(finished),
    }
    Ok(())
}

fn split_audio_at(
    queued: QueuedAudioChunk,
    boundary_ns: u64,
) -> Result<(QueuedAudioChunk, Option<QueuedAudioChunk>), AvEncoderWorkerError> {
    let start_frame = timestamp_to_audio_frame(queued.presentation_at_ns)?;
    let boundary_frame = timestamp_to_audio_frame(boundary_ns)?;
    let end_frame = start_frame
        .checked_add(u64::from(queued.chunk.frame_count))
        .ok_or(AvTimelineError::TimelineOverflow)?;
    if boundary_frame <= start_frame || boundary_frame >= end_frame {
        return Ok((queued, None));
    }
    let before_frames = u32::try_from(boundary_frame - start_frame)
        .map_err(|_| AvTimelineError::TimelineOverflow)?;
    let after_frames = queued.chunk.frame_count - before_frames;
    let split_samples = usize::try_from(before_frames)
        .ok()
        .and_then(|frames| frames.checked_mul(usize::from(queued.chunk.format.channels)))
        .ok_or(AvTimelineError::InvalidAudioChunk)?;
    let before_samples = queued.chunk.samples[..split_samples]
        .to_vec()
        .into_boxed_slice();
    let after_samples = queued.chunk.samples[split_samples..]
        .to_vec()
        .into_boxed_slice();
    let before_duration_ns = audio_frames_to_ns(before_frames)?;
    let after_duration_ns = audio_frames_to_ns(after_frames)?;
    let after_presentation_ns = queued
        .presentation_at_ns
        .checked_add(before_duration_ns)
        .ok_or(AvTimelineError::TimelineOverflow)?;
    let after_captured_at_ns = queued
        .chunk
        .captured_at_ns
        .checked_add(before_duration_ns)
        .ok_or(AvTimelineError::TimelineOverflow)?;
    let sequence = queued.chunk.sequence;
    let format = queued.chunk.format;
    let before = QueuedAudioChunk {
        chunk: super::audio::CapturedAudioChunk {
            sequence,
            captured_at_ns: queued.chunk.captured_at_ns,
            format,
            frame_count: before_frames,
            samples: before_samples,
        },
        presentation_at_ns: queued.presentation_at_ns,
        duration_ns: before_duration_ns,
        gap_before_ns: queued.gap_before_ns,
    };
    let after = QueuedAudioChunk {
        chunk: super::audio::CapturedAudioChunk {
            sequence,
            captured_at_ns: after_captured_at_ns,
            format,
            frame_count: after_frames,
            samples: after_samples,
        },
        presentation_at_ns: after_presentation_ns,
        duration_ns: after_duration_ns,
        gap_before_ns: 0,
    };
    Ok((before, Some(after)))
}

fn timestamp_to_audio_frame(timestamp_ns: u64) -> Result<u64, AvTimelineError> {
    u128::from(timestamp_ns)
        .checked_mul(AUDIO_SAMPLE_RATE_HZ_U128)
        .and_then(|value| value.checked_add(NANOS_PER_SECOND / 2))
        .and_then(|value| value.checked_div(NANOS_PER_SECOND))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(AvTimelineError::TimelineOverflow)
}

fn audio_frames_to_ns(frames: u32) -> Result<u64, AvTimelineError> {
    u128::from(frames)
        .checked_mul(NANOS_PER_SECOND)
        .and_then(|value| value.checked_div(AUDIO_SAMPLE_RATE_HZ_U128))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(AvTimelineError::TimelineOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::audio::{AudioFormat, CapturedAudioChunk};
    use crate::recording::av_segmenting::SegmentedAvRecordingWriter;
    use crate::recording::frame::CapturedFrame;
    use crate::recording::manifest::{
        RecordingJournal, RecordingJournalAudioConfig, RecordingJournalConfig,
    };
    use crate::recording::mux::opus_webm::OpusPacketEncoder;
    use serde_json::Value;
    use std::fs;

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

    fn audio(sequence: u64, captured_at_ns: u64, frames: u32) -> CapturedAudioChunk {
        CapturedAudioChunk {
            sequence,
            captured_at_ns,
            format: AudioFormat::normalized(2),
            frame_count: frames,
            samples: vec![0.2; frames as usize * 2].into_boxed_slice(),
        }
    }

    fn setup(
        root: &std::path::Path,
        session_id: &str,
    ) -> (
        SegmentedAvRecordingWriter,
        Arc<RecordingPipeline>,
        Arc<AudioPipeline>,
        std::path::PathBuf,
    ) {
        let opus = OpusPacketEncoder::new(2).unwrap();
        let track = opus.track_config().clone();
        let journal = RecordingJournal::create(
            root,
            RecordingJournalConfig {
                session_id: session_id.to_string(),
                source_id: "fixture-av-worker".to_string(),
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
                    sample_rate_hz: 48_000,
                    channels: 2,
                    encoder: "opus".to_string(),
                    pre_skip_frames: track.pre_skip_frames,
                    codec_delay_ns: track.codec_delay_ns,
                    seek_pre_roll_ns: track.seek_pre_roll_ns,
                }),
            },
        )
        .unwrap();
        let session = journal.session_directory().to_path_buf();
        let video = Arc::new(RecordingPipeline::default());
        let audio = Arc::new(AudioPipeline::new(0));
        let writer = SegmentedAvRecordingWriter::new(
            journal,
            Arc::clone(&video),
            2,
            2,
            10,
            2,
            200_000_000,
            opus,
        )
        .unwrap();
        (writer, video, audio, session)
    }

    #[test]
    fn merges_audio_that_arrives_first_trims_epoch_and_splits_at_video_boundary() {
        let temporary = tempfile::tempdir().unwrap();
        let (writer, video, audio_pipeline, session) = setup(temporary.path(), "av-worker");
        let worker =
            AvEncoderWorker::spawn(writer, Arc::clone(&video), Arc::clone(&audio_pipeline), 0)
                .unwrap();

        audio_pipeline.push(audio(0, 90_000_000, 4_800)).unwrap();
        video.push(frame(0, 100_000_000, 1)).unwrap();
        audio_pipeline.push(audio(1, 190_000_000, 4_800)).unwrap();
        video.push(frame(1, 200_000_000, 2)).unwrap();
        video.push(frame(2, 300_000_000, 3)).unwrap();
        video.finish(500_000_000).unwrap();
        audio_pipeline.finish(500_000_000).unwrap();

        let report = worker.wait().unwrap();
        assert_eq!(report.video_input_frames, 3);
        assert_eq!(report.audio_input_chunks, 2);
        assert_eq!(report.audio_input_frames, 9_600);
        assert_eq!(report.audio_trimmed_before_video_frames, 480);
        assert_eq!(report.audio_dropped_before_video_frames, 0);
        assert_eq!(report.video_duration_ns, 400_000_000);
        assert_eq!(report.audio_session_duration_ns, 500_000_000);
        assert_eq!(report.finish.mux_duration_ns, 400_000_000);
        assert_eq!(report.output.audio_pcm_frame_count, 19_200);
        report.output.completion.complete().unwrap();

        let value: Value =
            serde_json::from_slice(&fs::read(session.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(value["state"], "complete");
        assert_eq!(value["segments"].as_array().unwrap().len(), 2);
        assert_eq!(value["finalOutput"]["audio"]["pcmFrameCount"], 19_200);
    }

    #[test]
    fn one_pipeline_abort_wakes_bridges_and_interrupts_the_journal() {
        let temporary = tempfile::tempdir().unwrap();
        let (writer, video, audio_pipeline, session) = setup(temporary.path(), "av-abort");
        let worker =
            AvEncoderWorker::spawn(writer, Arc::clone(&video), Arc::clone(&audio_pipeline), 0)
                .unwrap();
        video.push(frame(0, 100_000_000, 1)).unwrap();
        audio_pipeline.abort().unwrap();

        assert!(matches!(
            worker.wait(),
            Err(AvEncoderWorkerError::AudioPipeline(
                AudioPipelineError::Aborted
            ))
        ));
        assert!(matches!(
            video.push(frame(1, 200_000_000, 2)),
            Err(PipelineError::Aborted)
        ));
        let value: Value =
            serde_json::from_slice(&fs::read(session.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(value["state"], "interrupted");
    }
}
