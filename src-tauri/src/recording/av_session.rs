//! VP9 + Opus 双轨录屏会话的单一资源 owner。
//!
//! 会话创建唯一共享时钟，持有视频/音频采集、两条有界 pipeline、双轨编码线程和 schema v2
//! journal。正常停止才提交 complete；任一错误或 `Drop` 会同步中止两轨并保留已提交恢复前缀。

use super::audio::{AudioPipeline, AudioPipelineError, AUDIO_SAMPLE_RATE_HZ};
use super::audio_worker::{
    AudioCaptureWorker, AudioCaptureWorkerError, AudioCaptureWorkerReport, RecordingAudioSource,
};
use super::av_encoder_worker::{AvEncoderReport, AvEncoderWorker, AvEncoderWorkerError};
use super::av_segmenting::{SegmentedAvRecordingError, SegmentedAvRecordingWriter};
use super::clock::RecordingSessionClock;
use super::manifest::{
    discard_unstarted_session, RecordingJournal, RecordingJournalAudioConfig,
    RecordingJournalConfig,
};
use super::mux::opus_webm::{OpusPacketEncoder, OpusWebmError};
use super::pipeline::{PipelineError, RecordingPipeline};
use super::worker::{CaptureWorker, CaptureWorkerError, CaptureWorkerReport, RecordingFrameSource};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Clone)]
pub(super) struct AvRecordingConfig {
    pub session_id: String,
    pub source_id: String,
    pub physical_x: i32,
    pub physical_y: i32,
    pub width: u32,
    pub height: u32,
    pub frames_per_second: u32,
    pub include_cursor: bool,
    pub audio_channels: u16,
    pub segment_duration_ns: u64,
}

#[derive(Debug, Error)]
pub(super) enum AvRecordingSessionError {
    #[error("双轨录屏配置无效")]
    InvalidConfiguration,
    #[error("双轨录屏 journal 失败: {0}")]
    Journal(String),
    #[error(transparent)]
    Opus(#[from] OpusWebmError),
    #[error(transparent)]
    Segment(#[from] SegmentedAvRecordingError),
    #[error(transparent)]
    VideoCapture(#[from] CaptureWorkerError),
    #[error(transparent)]
    AudioCapture(#[from] AudioCaptureWorkerError),
    #[error(transparent)]
    Encoder(#[from] AvEncoderWorkerError),
    #[error(transparent)]
    VideoPipeline(#[from] PipelineError),
    #[error(transparent)]
    AudioPipeline(#[from] AudioPipelineError),
    #[error("双轨录屏控制失败且无法恢复一致状态: {0}")]
    ControlDiverged(String),
    #[error("双轨录屏采集与编码报告不一致")]
    ReportMismatch,
    #[error("双轨录屏会话资源已经被消费")]
    AlreadySettled,
}

#[derive(Debug)]
pub(super) struct AvRecordingReport {
    pub segment_paths: Vec<PathBuf>,
    pub final_output_path: Option<PathBuf>,
    pub duration_ns: u64,
    pub video_captured_frames: u64,
    pub video_accepted_frames: u64,
    pub video_encoder_input_frames: u64,
    pub video_encoded_frames: u64,
    pub video_dropped_by_backpressure: u64,
    pub audio_captured_chunks: u64,
    pub audio_captured_frames: u64,
    pub audio_accepted_chunks: u64,
    pub audio_encoder_input_chunks: u64,
    pub audio_encoder_input_frames: u64,
    pub audio_encoded_packets: u64,
    pub audio_pcm_frames: u64,
    pub audio_dropped_before_video_frames: u64,
    pub audio_trimmed_before_video_frames: u64,
}

pub(super) struct AvRecordingSession {
    video_pipeline: Arc<RecordingPipeline>,
    audio_pipeline: Arc<AudioPipeline>,
    video_capture: Option<CaptureWorker>,
    audio_capture: Option<AudioCaptureWorker>,
    encoder: Option<AvEncoderWorker>,
    session_directory: PathBuf,
    settled: bool,
}

impl AvRecordingSession {
    pub fn start_with_factories<VF, VS, AF, AS>(
        app_data_dir: &Path,
        config: AvRecordingConfig,
        video_factory: VF,
        audio_factory: AF,
    ) -> Result<Self, AvRecordingSessionError>
    where
        VF: FnOnce(RecordingSessionClock) -> Result<VS, String> + Send + 'static,
        VS: RecordingFrameSource,
        AF: FnOnce(RecordingSessionClock) -> Result<AS, String> + Send + 'static,
        AS: RecordingAudioSource,
    {
        if !(1..=120).contains(&config.frames_per_second)
            || !(1..=2).contains(&config.audio_channels)
            || config.width == 0
            || config.height == 0
            || config.segment_duration_ns == 0
        {
            return Err(AvRecordingSessionError::InvalidConfiguration);
        }
        let session_id = config.session_id.clone();
        let clock = RecordingSessionClock::new();
        let opus = OpusPacketEncoder::new(config.audio_channels)?;
        let audio_track = opus.track_config().clone();
        let journal = RecordingJournal::create(
            app_data_dir,
            RecordingJournalConfig {
                session_id: config.session_id,
                source_id: config.source_id,
                physical_x: config.physical_x,
                physical_y: config.physical_y,
                width: config.width,
                height: config.height,
                target_fps_numerator: config.frames_per_second,
                target_fps_denominator: 1,
                encoder: "vp9-prototype".to_string(),
                container: "webm".to_string(),
                include_cursor: config.include_cursor,
                audio: Some(RecordingJournalAudioConfig {
                    sample_rate_hz: AUDIO_SAMPLE_RATE_HZ,
                    channels: config.audio_channels,
                    encoder: "opus".to_string(),
                    pre_skip_frames: audio_track.pre_skip_frames,
                    codec_delay_ns: audio_track.codec_delay_ns,
                    seek_pre_roll_ns: audio_track.seek_pre_roll_ns,
                }),
            },
        )
        .map_err(AvRecordingSessionError::Journal)?;
        let session_directory = journal.session_directory().to_path_buf();
        let video_pipeline = Arc::new(RecordingPipeline::default());
        let audio_pipeline = Arc::new(AudioPipeline::new(0));
        let writer = match SegmentedAvRecordingWriter::new(
            journal,
            Arc::clone(&video_pipeline),
            config.width,
            config.height,
            config.frames_per_second,
            config.audio_channels,
            config.segment_duration_ns,
            opus,
        ) {
            Ok(writer) => writer,
            Err(error) => {
                discard_failed_start(app_data_dir, &session_id);
                return Err(error.into());
            }
        };
        let encoder = match AvEncoderWorker::spawn(
            writer,
            Arc::clone(&video_pipeline),
            Arc::clone(&audio_pipeline),
            0,
        ) {
            Ok(encoder) => encoder,
            Err(error) => {
                discard_failed_start(app_data_dir, &session_id);
                return Err(error.into());
            }
        };
        let audio_capture = match AudioCaptureWorker::spawn_with_factory(
            audio_factory,
            clock.clone(),
            Arc::clone(&audio_pipeline),
        ) {
            Ok(capture) => capture,
            Err(error) => {
                drop(encoder);
                discard_failed_start(app_data_dir, &session_id);
                return Err(error.into());
            }
        };
        let video_clock = clock;
        let video_capture = match CaptureWorker::spawn_with_factory(
            move || video_factory(video_clock),
            Arc::clone(&video_pipeline),
            config.frames_per_second,
        ) {
            Ok(capture) => capture,
            Err(error) => {
                drop(audio_capture);
                drop(encoder);
                discard_failed_start(app_data_dir, &session_id);
                return Err(error.into());
            }
        };
        Ok(Self {
            video_pipeline,
            audio_pipeline,
            video_capture: Some(video_capture),
            audio_capture: Some(audio_capture),
            encoder: Some(encoder),
            session_directory,
            settled: false,
        })
    }

    pub fn pause(&self) -> Result<(), AvRecordingSessionError> {
        let video = self
            .video_capture
            .as_ref()
            .ok_or(AvRecordingSessionError::AlreadySettled)?;
        let audio = self
            .audio_capture
            .as_ref()
            .ok_or(AvRecordingSessionError::AlreadySettled)?;
        video.pause()?;
        if let Err(error) = audio.pause() {
            if let Err(rollback) = video.resume() {
                self.abort_pipelines();
                return Err(AvRecordingSessionError::ControlDiverged(format!(
                    "音频暂停失败: {error}; 视频恢复失败: {rollback}"
                )));
            }
            return Err(error.into());
        }
        Ok(())
    }

    pub fn resume(&self) -> Result<(), AvRecordingSessionError> {
        let video = self
            .video_capture
            .as_ref()
            .ok_or(AvRecordingSessionError::AlreadySettled)?;
        let audio = self
            .audio_capture
            .as_ref()
            .ok_or(AvRecordingSessionError::AlreadySettled)?;
        video.resume()?;
        if let Err(error) = audio.resume() {
            if let Err(rollback) = video.pause() {
                self.abort_pipelines();
                return Err(AvRecordingSessionError::ControlDiverged(format!(
                    "音频恢复失败: {error}; 视频重新暂停失败: {rollback}"
                )));
            }
            return Err(error.into());
        }
        Ok(())
    }

    pub fn stop(mut self) -> Result<AvRecordingReport, AvRecordingSessionError> {
        let video_capture = self
            .video_capture
            .take()
            .ok_or(AvRecordingSessionError::AlreadySettled)?;
        let audio_capture = self
            .audio_capture
            .take()
            .ok_or(AvRecordingSessionError::AlreadySettled)?;
        let encoder = self
            .encoder
            .take()
            .ok_or(AvRecordingSessionError::AlreadySettled)?;

        let video_result = video_capture.stop();
        let audio_result = audio_capture.stop();
        let encoder_result = encoder.wait();
        let (video, audio, encoder) = match (video_result, audio_result, encoder_result) {
            (Ok(video), Ok(audio), Ok(encoder)) => (video, audio, encoder),
            (Err(error), _, _) if !linked_video_abort(&error) => {
                return self.fail(error.into());
            }
            (_, Err(error), _) if !linked_audio_abort(&error) => {
                return self.fail(error.into());
            }
            (_, _, Err(error)) => return self.fail(error.into()),
            (Err(error), _, _) => return self.fail(error.into()),
            (_, Err(error), _) => return self.fail(error.into()),
        };
        match self.commit_reports(video, audio, encoder) {
            Ok(report) => {
                self.settled = true;
                Ok(report)
            }
            Err(error) => self.fail(error),
        }
    }

    pub fn session_directory(&self) -> &Path {
        &self.session_directory
    }

    /// 任一 worker 在 owner 发出 Stop 前退出都表示双轨会话已经失败。其余 worker 会由 pipeline
    /// 联动中止；控制面用这个只读信号进入唯一的生命周期清理路径。
    pub fn has_terminated_worker(&self) -> bool {
        self.video_capture
            .as_ref()
            .is_some_and(CaptureWorker::is_finished)
            || self
                .audio_capture
                .as_ref()
                .is_some_and(AudioCaptureWorker::is_finished)
            || self
                .encoder
                .as_ref()
                .is_some_and(AvEncoderWorker::is_finished)
    }

    fn commit_reports(
        &mut self,
        video: CaptureWorkerReport,
        audio: AudioCaptureWorkerReport,
        encoder: AvEncoderReport,
    ) -> Result<AvRecordingReport, AvRecordingSessionError> {
        let video_duration_ns = video
            .duration_ns
            .ok_or(AvRecordingSessionError::ReportMismatch)?;
        let audio_duration_ns = audio
            .duration_ns
            .ok_or(AvRecordingSessionError::ReportMismatch)?;
        let video_stats = self.video_pipeline.stats()?;
        let audio_stats = self.audio_pipeline.stats()?;
        if video_duration_ns != encoder.video_duration_ns
            || audio_duration_ns != encoder.audio_session_duration_ns
            || video.dropped_by_backpressure != video_stats.dropped_by_backpressure
            || audio.queued_chunks != audio_stats.accepted_chunks
            || encoder.video_input_frames
                != video_stats
                    .accepted_frames
                    .saturating_sub(video_stats.dropped_by_backpressure)
            || encoder.audio_input_chunks != audio_stats.accepted_chunks
            || encoder.output.video_frame_count == 0
            || encoder.output.audio_packet_count == 0
            || encoder.output.audio_pcm_frame_count == 0
            || encoder.output.duration_ns != encoder.finish.mux_duration_ns
        {
            return Err(AvRecordingSessionError::ReportMismatch);
        }
        let output = encoder
            .output
            .completion
            .complete()
            .map_err(AvRecordingSessionError::Journal)?;
        Ok(AvRecordingReport {
            segment_paths: output.segment_paths,
            final_output_path: output.final_output_path,
            duration_ns: encoder.finish.mux_duration_ns,
            video_captured_frames: video.captured_frames,
            video_accepted_frames: video_stats.accepted_frames,
            video_encoder_input_frames: encoder.video_input_frames,
            video_encoded_frames: encoder.output.video_frame_count,
            video_dropped_by_backpressure: video_stats.dropped_by_backpressure,
            audio_captured_chunks: audio.captured_chunks,
            audio_captured_frames: audio.captured_frames,
            audio_accepted_chunks: audio_stats.accepted_chunks,
            audio_encoder_input_chunks: encoder.audio_input_chunks,
            audio_encoder_input_frames: encoder.audio_input_frames,
            audio_encoded_packets: encoder.output.audio_packet_count,
            audio_pcm_frames: encoder.output.audio_pcm_frame_count,
            audio_dropped_before_video_frames: encoder.audio_dropped_before_video_frames,
            audio_trimmed_before_video_frames: encoder.audio_trimmed_before_video_frames,
        })
    }

    fn abort_pipelines(&self) {
        let _ = self.video_pipeline.abort();
        let _ = self.audio_pipeline.abort();
    }

    fn fail<T>(&mut self, error: AvRecordingSessionError) -> Result<T, AvRecordingSessionError> {
        self.abort_pipelines();
        drop(self.video_capture.take());
        drop(self.audio_capture.take());
        drop(self.encoder.take());
        self.settled = true;
        Err(error)
    }
}

impl Drop for AvRecordingSession {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        self.abort_pipelines();
        drop(self.video_capture.take());
        drop(self.audio_capture.take());
        drop(self.encoder.take());
        self.settled = true;
    }
}

fn linked_video_abort(error: &CaptureWorkerError) -> bool {
    matches!(error, CaptureWorkerError::Pipeline(PipelineError::Aborted))
}

fn linked_audio_abort(error: &AudioCaptureWorkerError) -> bool {
    matches!(
        error,
        AudioCaptureWorkerError::Pipeline(AudioPipelineError::Aborted)
    )
}

fn discard_failed_start(app_data_dir: &Path, session_id: &str) {
    if let Err(error) = discard_unstarted_session(app_data_dir, session_id) {
        log::error!("双轨录屏启动失败后无法删除空会话 {session_id}: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::audio::{AudioFormat, CapturedAudioChunk};
    use crate::recording::frame::CapturedFrame;
    use serde_json::Value;
    use std::collections::VecDeque;
    use std::error::Error;
    use std::fmt;
    use std::fs;
    use std::sync::Mutex;
    use std::thread;
    use std::time::Duration;

    #[derive(Debug, Clone, Copy)]
    struct FixtureError(&'static str);

    impl fmt::Display for FixtureError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.0)
        }
    }

    impl Error for FixtureError {}

    struct SessionVideoSource {
        frames: VecDeque<CapturedFrame>,
        stop_at_ns: u64,
        hooks: Option<Arc<Mutex<Vec<&'static str>>>>,
    }

    impl RecordingFrameSource for SessionVideoSource {
        type Error = FixtureError;

        fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
            self.frames
                .pop_front()
                .ok_or(FixtureError("video fixture exhausted"))
        }

        fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
            Ok(self.frames.pop_front())
        }

        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            Ok(self.stop_at_ns)
        }

        fn pause_capture(&mut self) -> Result<u64, Self::Error> {
            if let Some(hooks) = &self.hooks {
                hooks.lock().unwrap().push("pause");
            }
            Ok(500_000_000)
        }

        fn resume_capture(&mut self) -> Result<u64, Self::Error> {
            if let Some(hooks) = &self.hooks {
                hooks.lock().unwrap().push("resume");
            }
            Ok(600_000_000)
        }

        fn stop_capture(&mut self) -> Result<u64, Self::Error> {
            if let Some(hooks) = &self.hooks {
                hooks.lock().unwrap().push("stop");
            }
            Ok(self.stop_at_ns)
        }
    }

    struct SessionAudioSource {
        chunks: VecDeque<CapturedAudioChunk>,
        stop_at_ns: u64,
        hooks: Option<Arc<Mutex<Vec<&'static str>>>>,
    }

    impl RecordingAudioSource for SessionAudioSource {
        type Error = FixtureError;

        fn capture_next_available(
            &mut self,
            timeout: Duration,
        ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
            let chunk = self.chunks.pop_front();
            if chunk.is_none() {
                thread::sleep(timeout);
            }
            Ok(chunk)
        }

        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            Ok(self.stop_at_ns)
        }

        fn pause_capture(&mut self) -> Result<u64, Self::Error> {
            if let Some(hooks) = &self.hooks {
                hooks.lock().unwrap().push("pause");
            }
            Ok(500_000_000)
        }

        fn resume_capture(&mut self) -> Result<u64, Self::Error> {
            if let Some(hooks) = &self.hooks {
                hooks.lock().unwrap().push("resume");
            }
            Ok(600_000_000)
        }

        fn stop_capture(&mut self) -> Result<u64, Self::Error> {
            if let Some(hooks) = &self.hooks {
                hooks.lock().unwrap().push("stop");
            }
            Ok(self.stop_at_ns)
        }
    }

    fn frame(sequence: u64, captured_at_ns: u64) -> CapturedFrame {
        CapturedFrame {
            sequence,
            captured_at_ns,
            width: 2,
            height: 2,
            stride: 8,
            rgba: vec![sequence as u8; 16].into_boxed_slice(),
        }
    }

    fn chunk(sequence: u64, captured_at_ns: u64) -> CapturedAudioChunk {
        CapturedAudioChunk {
            sequence,
            captured_at_ns,
            format: AudioFormat::normalized(2),
            frame_count: 4_800,
            samples: vec![0.1; 9_600].into_boxed_slice(),
        }
    }

    fn video_source(hooks: Option<Arc<Mutex<Vec<&'static str>>>>) -> SessionVideoSource {
        SessionVideoSource {
            frames: [
                frame(0, 0),
                frame(1, 100_000_000),
                frame(2, 200_000_000),
                frame(3, 300_000_000),
            ]
            .into(),
            stop_at_ns: 400_000_000,
            hooks,
        }
    }

    fn audio_source(hooks: Option<Arc<Mutex<Vec<&'static str>>>>) -> SessionAudioSource {
        SessionAudioSource {
            chunks: [
                chunk(0, 0),
                chunk(1, 100_000_000),
                chunk(2, 200_000_000),
                chunk(3, 300_000_000),
            ]
            .into(),
            stop_at_ns: 400_000_000,
            hooks,
        }
    }

    fn idle_video_source(hooks: Arc<Mutex<Vec<&'static str>>>) -> SessionVideoSource {
        SessionVideoSource {
            frames: [frame(0, 0)].into(),
            stop_at_ns: 700_000_000,
            hooks: Some(hooks),
        }
    }

    fn idle_audio_source(hooks: Arc<Mutex<Vec<&'static str>>>) -> SessionAudioSource {
        SessionAudioSource {
            chunks: [chunk(0, 0)].into(),
            stop_at_ns: 700_000_000,
            hooks: Some(hooks),
        }
    }

    fn config(session_id: &str) -> AvRecordingConfig {
        AvRecordingConfig {
            session_id: session_id.to_string(),
            source_id: "fixture-av-session".to_string(),
            physical_x: -100,
            physical_y: 20,
            width: 2,
            height: 2,
            frames_per_second: 10,
            include_cursor: false,
            audio_channels: 2,
            segment_duration_ns: 200_000_000,
        }
    }

    #[test]
    fn owns_both_capture_tracks_and_commits_only_after_report_validation() {
        let temporary = tempfile::tempdir().unwrap();
        let session = AvRecordingSession::start_with_factories(
            temporary.path(),
            config("av-session-complete"),
            |_| Ok(video_source(None)),
            |_| Ok(audio_source(None)),
        )
        .unwrap();
        let session_directory = session.session_directory().to_path_buf();
        thread::sleep(Duration::from_millis(360));

        let report = session.stop().unwrap();
        assert_eq!(report.segment_paths.len(), 2);
        assert!(report.final_output_path.is_some());
        assert_eq!(report.duration_ns, 400_000_000);
        assert_eq!(report.video_captured_frames, 4);
        assert_eq!(report.video_accepted_frames, 4);
        assert_eq!(report.video_encoder_input_frames, 4);
        assert_eq!(report.video_encoded_frames, 4);
        assert_eq!(report.video_dropped_by_backpressure, 0);
        assert_eq!(report.audio_captured_chunks, 4);
        assert_eq!(report.audio_captured_frames, 19_200);
        assert_eq!(report.audio_accepted_chunks, 4);
        assert_eq!(report.audio_encoder_input_chunks, 4);
        assert_eq!(report.audio_encoder_input_frames, 19_200);
        assert!(report.audio_encoded_packets > 0);
        assert_eq!(report.audio_pcm_frames, 19_200);
        assert_eq!(report.audio_dropped_before_video_frames, 0);
        assert_eq!(report.audio_trimmed_before_video_frames, 0);

        let manifest: Value =
            serde_json::from_slice(&fs::read(session_directory.join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["schemaVersion"], 2);
        assert_eq!(manifest["state"], "complete");
        assert_eq!(manifest["segments"].as_array().unwrap().len(), 2);
        assert_eq!(manifest["finalOutput"]["audio"]["pcmFrameCount"], 19_200);
    }

    #[test]
    fn pause_and_resume_reach_both_sources_and_drop_interrupts_the_session() {
        let temporary = tempfile::tempdir().unwrap();
        let video_hooks = Arc::new(Mutex::new(Vec::new()));
        let audio_hooks = Arc::new(Mutex::new(Vec::new()));
        let video_factory_hooks = Arc::clone(&video_hooks);
        let audio_factory_hooks = Arc::clone(&audio_hooks);
        let session = AvRecordingSession::start_with_factories(
            temporary.path(),
            config("av-session-drop"),
            move |_| Ok(idle_video_source(video_factory_hooks)),
            move |_| Ok(idle_audio_source(audio_factory_hooks)),
        )
        .unwrap();
        let session_directory = session.session_directory().to_path_buf();

        session.pause().unwrap();
        session.resume().unwrap();
        assert_eq!(&*video_hooks.lock().unwrap(), &["pause", "resume"]);
        assert_eq!(&*audio_hooks.lock().unwrap(), &["pause", "resume"]);
        drop(session);

        let manifest: Value =
            serde_json::from_slice(&fs::read(session_directory.join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["state"], "interrupted");
        assert!(!session_directory
            .join("segment-000000.webm.partial")
            .exists());
        assert!(!session_directory.join("recording.webm.partial").exists());
    }

    #[test]
    fn startup_failure_removes_the_empty_session_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let result = AvRecordingSession::start_with_factories(
            temporary.path(),
            config("av-session-start-failure"),
            |_| Ok(video_source(None)),
            |_| Err::<SessionAudioSource, _>("audio fixture unavailable".to_string()),
        );
        assert!(matches!(
            result,
            Err(AvRecordingSessionError::AudioCapture(
                AudioCaptureWorkerError::SourceInitialization(_)
            ))
        ));
        assert!(!temporary
            .path()
            .join("recordings")
            .join("av-session-start-failure")
            .exists());
    }
}
