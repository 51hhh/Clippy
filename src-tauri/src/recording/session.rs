//! 诊断录屏会话的单一资源 owner。
//!
//! 会话同时持有 journal、临时分段、采集线程、三槽 pipeline 和编码线程。正常停止只由这一个 owner
//! 提交分段与 complete；任一错误或 `Drop` 都中止两条线程、清理未提交临时文件并把 journal 标为
//! interrupted，避免各层分别猜测资源是否已经释放。

use super::encoder_worker::{EncoderReport, EncoderWorker, EncoderWorkerError};
use super::manifest::{RecordingJournal, RecordingJournalConfig};
use super::pipeline::{PipelineError, RecordingPipeline};
use super::segmenting::{
    PendingRecordingCompletion, RecordingEncoder, SegmentedRecordingError, SegmentedRecordingWriter,
};
use super::worker::{CaptureWorker, CaptureWorkerError, CaptureWorkerReport, RecordingFrameSource};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Clone)]
pub(super) struct DiagnosticRecordingConfig {
    pub session_id: String,
    pub source_id: String,
    pub physical_x: i32,
    pub physical_y: i32,
    pub width: u32,
    pub height: u32,
    pub frames_per_second: u32,
    pub include_cursor: bool,
    pub encoder: RecordingEncoder,
    pub segment_duration_ns: u64,
}

#[derive(Debug, Error)]
pub(super) enum DiagnosticRecordingError {
    #[error("诊断录屏配置无效")]
    InvalidConfiguration,
    #[error("录屏 journal 失败: {0}")]
    Journal(String),
    #[error(transparent)]
    Capture(#[from] CaptureWorkerError),
    #[error(transparent)]
    Segment(#[from] SegmentedRecordingError),
    #[error(transparent)]
    Encoder(#[from] EncoderWorkerError<SegmentedRecordingError>),
    #[error(transparent)]
    Pipeline(#[from] PipelineError),
    #[error("采集线程与编码线程的最终时长不一致")]
    DurationMismatch,
    #[error("采集线程与 pipeline 的背压计数不一致")]
    BackpressureMismatch,
    #[error("录屏会话资源已经被消费")]
    AlreadySettled,
}

#[derive(Debug)]
pub(super) struct DiagnosticRecordingReport {
    pub segment_paths: Vec<PathBuf>,
    pub final_output_path: Option<PathBuf>,
    pub duration_ns: u64,
    pub captured_frames: u64,
    pub accepted_frames: u64,
    pub encoder_input_frames: u64,
    pub encoded_frames: u64,
    pub dropped_by_backpressure: u64,
}

pub(super) struct DiagnosticRecordingSession {
    pipeline: Arc<RecordingPipeline>,
    capture: Option<CaptureWorker>,
    encoder: Option<EncoderWorker<SegmentedRecordingWriter>>,
    session_directory: PathBuf,
    settled: bool,
}

impl DiagnosticRecordingSession {
    pub fn start<S>(
        app_data_dir: &Path,
        config: DiagnosticRecordingConfig,
        source: S,
    ) -> Result<Self, DiagnosticRecordingError>
    where
        S: RecordingFrameSource,
    {
        if !(1..=120).contains(&config.frames_per_second) || !config.encoder.is_valid() {
            return Err(DiagnosticRecordingError::InvalidConfiguration);
        }
        let (encoder_name, container) = config.encoder.manifest_descriptor();
        let journal = RecordingJournal::create(
            app_data_dir,
            RecordingJournalConfig {
                session_id: config.session_id.clone(),
                source_id: config.source_id.clone(),
                physical_x: config.physical_x,
                physical_y: config.physical_y,
                width: config.width,
                height: config.height,
                target_fps_numerator: config.frames_per_second,
                target_fps_denominator: 1,
                encoder: encoder_name.to_string(),
                container: container.to_string(),
                include_cursor: config.include_cursor,
            },
        )
        .map_err(DiagnosticRecordingError::Journal)?;
        let session_directory = journal.session_directory().to_path_buf();
        let pipeline = Arc::new(RecordingPipeline::default());
        let segmented = SegmentedRecordingWriter::new(
            journal,
            Arc::clone(&pipeline),
            config.encoder,
            config.width,
            config.height,
            config.frames_per_second,
            config.segment_duration_ns,
        )?;
        let encoder = match EncoderWorker::spawn(segmented, Arc::clone(&pipeline)) {
            Ok(encoder) => encoder,
            Err(error) => {
                return Err(error.into());
            }
        };
        let capture =
            match CaptureWorker::spawn(source, Arc::clone(&pipeline), config.frames_per_second) {
                Ok(capture) => capture,
                Err(error) => {
                    drop(encoder);
                    return Err(error.into());
                }
            };
        Ok(Self {
            pipeline,
            capture: Some(capture),
            encoder: Some(encoder),
            session_directory,
            settled: false,
        })
    }

    pub fn pause(&self) -> Result<(), DiagnosticRecordingError> {
        self.capture
            .as_ref()
            .ok_or(DiagnosticRecordingError::AlreadySettled)?
            .pause()?;
        Ok(())
    }

    pub fn resume(&self) -> Result<(), DiagnosticRecordingError> {
        self.capture
            .as_ref()
            .ok_or(DiagnosticRecordingError::AlreadySettled)?
            .resume()?;
        Ok(())
    }

    pub fn stop(mut self) -> Result<DiagnosticRecordingReport, DiagnosticRecordingError> {
        let capture = self
            .capture
            .take()
            .ok_or(DiagnosticRecordingError::AlreadySettled)?;
        let encoder = self
            .encoder
            .take()
            .ok_or(DiagnosticRecordingError::AlreadySettled)?;
        // 两条线程都要 join 后再选根因。编码失败会把 capture 推入 Pipeline::Aborted；采集失败则会
        // 把 encoder 推入同名终态，不能让这个联动错误遮住最先发生的具体错误。
        let capture_result = capture.stop();
        let encoder_result = encoder.wait().map_err(DiagnosticRecordingError::Encoder);
        let (capture_report, encoder_report) = match (capture_result, encoder_result) {
            (Ok(capture_report), Ok(encoder_report)) => (capture_report, encoder_report),
            (Err(CaptureWorkerError::Pipeline(PipelineError::Aborted)), Err(encoder_error)) => {
                return self.fail(encoder_error);
            }
            (Err(capture_error), _) => return self.fail(capture_error.into()),
            (Ok(_), Err(encoder_error)) => return self.fail(encoder_error),
        };
        let result = self.commit_reports(capture_report, encoder_report);
        match result {
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

    fn commit_reports(
        &mut self,
        capture: CaptureWorkerReport,
        encoder: EncoderReport<PendingRecordingCompletion>,
    ) -> Result<DiagnosticRecordingReport, DiagnosticRecordingError> {
        let duration_ns = capture
            .duration_ns
            .ok_or(DiagnosticRecordingError::DurationMismatch)?;
        if encoder.duration_ns != duration_ns {
            return Err(DiagnosticRecordingError::DurationMismatch);
        }
        let stats = self.pipeline.stats()?;
        if capture.dropped_by_backpressure != stats.dropped_by_backpressure {
            return Err(DiagnosticRecordingError::BackpressureMismatch);
        }
        let outputs = encoder
            .writer
            .complete()
            .map_err(DiagnosticRecordingError::Journal)?;
        Ok(DiagnosticRecordingReport {
            segment_paths: outputs.segment_paths,
            final_output_path: outputs.final_output_path,
            duration_ns,
            captured_frames: capture.captured_frames,
            accepted_frames: stats.accepted_frames,
            encoder_input_frames: encoder.input_frames,
            encoded_frames: encoder.encoded_frames,
            dropped_by_backpressure: stats.dropped_by_backpressure,
        })
    }

    fn fail<T>(&mut self, error: DiagnosticRecordingError) -> Result<T, DiagnosticRecordingError> {
        let _ = self.pipeline.abort();
        drop(self.capture.take());
        drop(self.encoder.take());
        self.settled = true;
        Err(error)
    }
}

impl Drop for DiagnosticRecordingSession {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let _ = self.pipeline.abort();
        drop(self.capture.take());
        drop(self.encoder.take());
        self.settled = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::frame::CapturedFrame;
    use crate::recording::mux::avi_mjpeg::AviMjpegError;
    use crate::recording::segmenting::DEFAULT_SEGMENT_DURATION_NS;
    use std::convert::Infallible;
    use std::fmt;
    use std::process::Command;

    struct FixtureSource {
        sequence: u64,
        timestamp_ns: u64,
    }

    impl RecordingFrameSource for FixtureSource {
        type Error = Infallible;

        fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
            let sequence = self.sequence;
            let captured_at_ns = self.timestamp_ns;
            self.sequence += 1;
            self.timestamp_ns += 100_000_000;
            Ok(CapturedFrame {
                sequence,
                captured_at_ns,
                width: 2,
                height: 2,
                stride: 8,
                rgba: vec![sequence as u8; 16].into_boxed_slice(),
            })
        }

        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            let timestamp_ns = self.timestamp_ns;
            self.timestamp_ns += 1;
            Ok(timestamp_ns)
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct FixtureSourceError;

    impl fmt::Display for FixtureSourceError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("fixture capture failed")
        }
    }

    impl std::error::Error for FixtureSourceError {}

    struct FailingSource;

    impl RecordingFrameSource for FailingSource {
        type Error = FixtureSourceError;

        fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
            Err(FixtureSourceError)
        }

        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            Err(FixtureSourceError)
        }
    }

    struct MismatchedSource {
        sequence: u64,
        timestamp_ns: u64,
    }

    #[cfg(feature = "recording-vp9-prototype")]
    struct Vp9FixtureSource {
        sequence: u64,
        timestamp_ns: u64,
    }

    #[cfg(feature = "recording-vp9-prototype")]
    impl RecordingFrameSource for Vp9FixtureSource {
        type Error = Infallible;

        fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
            let sequence = self.sequence;
            let captured_at_ns = self.timestamp_ns;
            self.sequence += 1;
            self.timestamp_ns += 100_000_000;
            let marker = sequence as u8;
            let rgba = (0..64 * 48)
                .flat_map(|pixel| {
                    let value = marker.wrapping_add(pixel as u8);
                    [
                        value,
                        value.wrapping_mul(3),
                        255_u8.wrapping_sub(value),
                        255,
                    ]
                })
                .collect::<Vec<_>>();
            Ok(CapturedFrame {
                sequence,
                captured_at_ns,
                width: 64,
                height: 48,
                stride: 64 * 4,
                rgba: rgba.into_boxed_slice(),
            })
        }

        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            let timestamp_ns = self.timestamp_ns;
            self.timestamp_ns += 1;
            Ok(timestamp_ns)
        }
    }

    impl RecordingFrameSource for MismatchedSource {
        type Error = Infallible;

        fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
            let sequence = self.sequence;
            let captured_at_ns = self.timestamp_ns;
            self.sequence += 1;
            self.timestamp_ns += 100_000_000;
            Ok(CapturedFrame {
                sequence,
                captured_at_ns,
                width: 1,
                height: 1,
                stride: 4,
                rgba: vec![0; 4].into_boxed_slice(),
            })
        }

        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            let timestamp_ns = self.timestamp_ns;
            self.timestamp_ns += 1;
            Ok(timestamp_ns)
        }
    }

    fn config(session_id: &str) -> DiagnosticRecordingConfig {
        DiagnosticRecordingConfig {
            session_id: session_id.to_string(),
            source_id: "fixture-monitor".to_string(),
            physical_x: 0,
            physical_y: 0,
            width: 2,
            height: 2,
            frames_per_second: 10,
            include_cursor: true,
            encoder: RecordingEncoder::MjpegDiagnostic { jpeg_quality: 85 },
            segment_duration_ns: DEFAULT_SEGMENT_DURATION_NS,
        }
    }

    fn manifest_state(directory: &Path) -> String {
        let value = manifest_value(directory);
        value["state"].as_str().unwrap().to_string()
    }

    fn manifest_value(directory: &Path) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(directory.join("manifest.json")).unwrap()).unwrap()
    }

    #[cfg(feature = "recording-vp9-prototype")]
    fn assert_vp9_probe_when_available(path: &Path, frame_count: u64, duration_ns: u64) {
        let available = Command::new("ffprobe")
            .arg("-version")
            .output()
            .is_ok_and(|output| output.status.success());
        if !available {
            return;
        }
        let output = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-count_frames",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,nb_read_frames:format=duration",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(payload["streams"][0]["codec_name"], "vp9");
        assert_eq!(
            payload["streams"][0]["nb_read_frames"],
            frame_count.to_string()
        );
        let actual_duration = payload["format"]["duration"]
            .as_str()
            .unwrap()
            .parse::<f64>()
            .unwrap();
        let expected_duration = duration_ns as f64 / 1_000_000_000.0;
        assert!((actual_duration - expected_duration).abs() <= 0.002);
    }

    #[test]
    fn normal_stop_commits_segment_and_complete_manifest() {
        let temporary = tempfile::tempdir().unwrap();
        let session = DiagnosticRecordingSession::start(
            temporary.path(),
            config("session-complete"),
            FixtureSource {
                sequence: 0,
                timestamp_ns: 100,
            },
        )
        .unwrap();
        let directory = session.session_directory().to_path_buf();
        let report = session.stop().unwrap();

        assert_eq!(manifest_state(&directory), "complete");
        assert_eq!(report.duration_ns, 100_000_000);
        assert!(report.captured_frames >= 1);
        assert!(report.accepted_frames >= 1);
        assert!(report.encoder_input_frames >= 1);
        assert!(report.encoded_frames >= 1);
        assert_eq!(report.dropped_by_backpressure, 0);
        assert_eq!(report.segment_paths.len(), 1);
        assert!(report.final_output_path.is_none());
        assert_eq!(
            &std::fs::read(&report.segment_paths[0]).unwrap()[0..4],
            b"RIFF"
        );
        assert!(!directory.join(".segment-000000.avi.partial").exists());
    }

    #[test]
    fn periodic_segment_is_committed_before_stop_and_preserved_in_final_manifest() {
        let temporary = tempfile::tempdir().unwrap();
        let mut configuration = config("session-periodic");
        configuration.segment_duration_ns = 200_000_000;
        let session = DiagnosticRecordingSession::start(
            temporary.path(),
            configuration,
            FixtureSource {
                sequence: 0,
                timestamp_ns: 100,
            },
        )
        .unwrap();
        let directory = session.session_directory().to_path_buf();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let manifest = manifest_value(&directory);
            if !manifest["segments"].as_array().unwrap().is_empty() {
                assert_eq!(manifest["state"], "recording");
                assert!(directory.join("segment-000000.avi").exists());
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "周期分段未在停止前提交"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let report = session.stop().unwrap();
        let manifest = manifest_value(&directory);
        let segments = manifest["segments"].as_array().unwrap();
        assert_eq!(manifest["state"], "complete");
        assert!(report.segment_paths.len() >= 2);
        assert_eq!(segments.len(), report.segment_paths.len());
        assert_eq!(
            segments
                .iter()
                .map(|segment| segment["durationNs"].as_u64().unwrap())
                .sum::<u64>(),
            report.duration_ns
        );
        assert_eq!(
            segments
                .iter()
                .map(|segment| segment["frameCount"].as_u64().unwrap())
                .sum::<u64>(),
            report.encoded_frames
        );
        for path in report.segment_paths {
            assert_eq!(&std::fs::read(path).unwrap()[0..4], b"RIFF");
        }
    }

    #[test]
    fn strong_kill_recovers_committed_prefix_and_discards_open_tail() {
        const CHILD_ENV: &str = "CLIPPY_RECORDING_CRASH_FIXTURE_CHILD";
        const DIRECTORY_ENV: &str = "CLIPPY_RECORDING_CRASH_FIXTURE_DIRECTORY";
        if std::env::var_os(CHILD_ENV).is_some() {
            let directory = PathBuf::from(std::env::var_os(DIRECTORY_ENV).unwrap());
            let mut configuration = config("session-crash");
            configuration.segment_duration_ns = 200_000_000;
            let session = DiagnosticRecordingSession::start(
                &directory,
                configuration,
                FixtureSource {
                    sequence: 0,
                    timestamp_ns: 100,
                },
            )
            .unwrap();
            let session_directory = session.session_directory().to_path_buf();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while manifest_value(&session_directory)["segments"]
                .as_array()
                .unwrap()
                .is_empty()
            {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(session_directory.join("segment-000000.avi").exists());
            assert!(session_directory
                .join(".segment-000001.avi.partial")
                .exists());
            std::process::exit(91);
        }

        let temporary = tempfile::tempdir().unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "recording::session::tests::strong_kill_recovers_committed_prefix_and_discards_open_tail",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .env(DIRECTORY_ENV, temporary.path())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(91));

        let summary = crate::recording::recover_interrupted_sessions(temporary.path()).unwrap();
        let directory = temporary.path().join("recordings/session-crash");
        let manifest = manifest_value(&directory);
        assert_eq!(summary.interrupted_sessions, 1);
        assert_eq!(summary.recoverable_segments, 1);
        assert_eq!(manifest["state"], "interrupted");
        assert_eq!(manifest["segments"].as_array().unwrap().len(), 1);
        assert!(directory.join("segment-000000.avi").exists());
        assert!(!directory.join(".segment-000001.avi.partial").exists());
        assert_eq!(
            &std::fs::read(directory.join("segment-000000.avi")).unwrap()[0..4],
            b"RIFF"
        );
    }

    #[test]
    fn capture_failure_interrupts_manifest_and_removes_partial() {
        let temporary = tempfile::tempdir().unwrap();
        let session = DiagnosticRecordingSession::start(
            temporary.path(),
            config("session-failed"),
            FailingSource,
        )
        .unwrap();
        let directory = session.session_directory().to_path_buf();
        assert!(matches!(
            session.stop(),
            Err(DiagnosticRecordingError::Capture(
                CaptureWorkerError::Source(_)
            ))
        ));
        assert_eq!(manifest_state(&directory), "interrupted");
        assert!(!directory.join(".segment-000000.avi.partial").exists());
    }

    #[test]
    fn dropping_active_session_interrupts_and_joins_both_workers() {
        let temporary = tempfile::tempdir().unwrap();
        let session = DiagnosticRecordingSession::start(
            temporary.path(),
            config("session-dropped"),
            FixtureSource {
                sequence: 0,
                timestamp_ns: 100,
            },
        )
        .unwrap();
        let directory = session.session_directory().to_path_buf();
        drop(session);
        assert_eq!(manifest_state(&directory), "interrupted");
        assert!(!directory.join(".segment-000000.avi.partial").exists());
    }

    #[test]
    fn encoder_root_cause_outranks_capture_abort_cascade() {
        let temporary = tempfile::tempdir().unwrap();
        let session = DiagnosticRecordingSession::start(
            temporary.path(),
            config("session-encoder-failed"),
            MismatchedSource {
                sequence: 0,
                timestamp_ns: 100,
            },
        )
        .unwrap();
        let directory = session.session_directory().to_path_buf();
        assert!(matches!(
            session.stop(),
            Err(DiagnosticRecordingError::Encoder(EncoderWorkerError::Mux(
                SegmentedRecordingError::Avi(AviMjpegError::InvalidFrame)
            )))
        ));
        assert_eq!(manifest_state(&directory), "interrupted");
        assert!(!directory.join(".segment-000000.avi.partial").exists());
    }

    #[test]
    fn invalid_runtime_config_creates_no_recording_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let mut invalid = config("invalid");
        invalid.frames_per_second = 0;
        assert!(matches!(
            DiagnosticRecordingSession::start(temporary.path(), invalid, FailingSource),
            Err(DiagnosticRecordingError::InvalidConfiguration)
        ));
        assert!(!temporary.path().join("recordings").exists());
    }

    #[cfg(feature = "recording-vp9-prototype")]
    #[test]
    fn vp9_session_commits_webm_and_matching_manifest_descriptor() {
        let temporary = tempfile::tempdir().unwrap();
        let mut configuration = config("session-vp9");
        configuration.width = 64;
        configuration.height = 48;
        configuration.encoder = RecordingEncoder::Vp9Prototype;
        configuration.segment_duration_ns = 200_000_000;
        let session = DiagnosticRecordingSession::start(
            temporary.path(),
            configuration,
            Vp9FixtureSource {
                sequence: 0,
                timestamp_ns: 100,
            },
        )
        .unwrap();
        let directory = session.session_directory().to_path_buf();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while session.pipeline.stats().unwrap().accepted_frames < 3
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(session.pipeline.stats().unwrap().accepted_frames >= 3);

        let report = session.stop().unwrap();
        let manifest = manifest_value(&directory);
        assert!(report.segment_paths.len() >= 2);
        for path in &report.segment_paths {
            assert_eq!(
                &std::fs::read(path).unwrap()[0..4],
                [0x1a, 0x45, 0xdf, 0xa3]
            );
            assert_eq!(path.extension().unwrap(), "webm");
            assert!(crate::private_files::is_private(path));
        }
        let final_output = report.final_output_path.as_ref().unwrap();
        assert_eq!(final_output.file_name().unwrap(), "recording.webm");
        assert_eq!(
            &std::fs::read(final_output).unwrap()[0..4],
            [0x1a, 0x45, 0xdf, 0xa3]
        );
        assert!(crate::private_files::is_private(final_output));
        assert_eq!(manifest["state"], "complete");
        assert_eq!(manifest["video"]["encoder"], "vp9-prototype");
        assert_eq!(manifest["video"]["container"], "webm");
        assert_eq!(manifest["segments"][0]["fileName"], "segment-000000.webm");
        assert_eq!(manifest["segments"][1]["fileName"], "segment-000001.webm");
        assert_eq!(manifest["finalOutput"]["fileName"], "recording.webm");
        assert_eq!(manifest["finalOutput"]["durationNs"], report.duration_ns);
        assert_eq!(manifest["finalOutput"]["frameCount"], report.encoded_frames);
        let segments = manifest["segments"].as_array().unwrap();
        let manifest_duration: u64 = segments
            .iter()
            .map(|segment| segment["durationNs"].as_u64().unwrap())
            .sum();
        let manifest_frames: u64 = segments
            .iter()
            .map(|segment| segment["frameCount"].as_u64().unwrap())
            .sum();
        assert_eq!(manifest_duration, report.duration_ns);
        assert_eq!(manifest_frames, report.encoded_frames);
        for (path, segment) in report.segment_paths.iter().zip(segments) {
            assert_vp9_probe_when_available(
                path,
                segment["frameCount"].as_u64().unwrap(),
                segment["durationNs"].as_u64().unwrap(),
            );
        }
        assert_vp9_probe_when_available(final_output, report.encoded_frames, report.duration_ns);
        assert!(!directory.join(".segment-000000.webm.partial").exists());
        assert!(!directory.join(".segment-000001.webm.partial").exists());
    }

    #[cfg(feature = "recording-vp9-prototype")]
    #[test]
    fn vp9_strong_kill_recovers_committed_prefix_and_discards_open_tail() {
        const CHILD_ENV: &str = "CLIPPY_RECORDING_VP9_CRASH_FIXTURE_CHILD";
        const DIRECTORY_ENV: &str = "CLIPPY_RECORDING_VP9_CRASH_FIXTURE_DIRECTORY";
        if std::env::var_os(CHILD_ENV).is_some() {
            let directory = PathBuf::from(std::env::var_os(DIRECTORY_ENV).unwrap());
            let mut configuration = config("session-vp9-crash");
            configuration.width = 64;
            configuration.height = 48;
            configuration.encoder = RecordingEncoder::Vp9Prototype;
            configuration.segment_duration_ns = 200_000_000;
            let session = DiagnosticRecordingSession::start(
                &directory,
                configuration,
                Vp9FixtureSource {
                    sequence: 0,
                    timestamp_ns: 100,
                },
            )
            .unwrap();
            let session_directory = session.session_directory().to_path_buf();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while manifest_value(&session_directory)["segments"]
                .as_array()
                .unwrap()
                .is_empty()
            {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(session_directory.join("segment-000000.webm").exists());
            assert!(session_directory
                .join(".segment-000001.webm.partial")
                .exists());
            assert!(session_directory.join(".recording.webm.partial").exists());
            std::process::exit(91);
        }

        let temporary = tempfile::tempdir().unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "recording::session::tests::vp9_strong_kill_recovers_committed_prefix_and_discards_open_tail",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .env(DIRECTORY_ENV, temporary.path())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(91));

        let summary = crate::recording::recover_interrupted_sessions(temporary.path()).unwrap();
        let directory = temporary.path().join("recordings/session-vp9-crash");
        let manifest = manifest_value(&directory);
        assert_eq!(summary.interrupted_sessions, 1);
        assert_eq!(summary.recoverable_segments, 1);
        assert_eq!(manifest["state"], "interrupted");
        assert_eq!(manifest["segments"].as_array().unwrap().len(), 1);
        assert!(directory.join("segment-000000.webm").exists());
        assert!(!directory.join(".segment-000001.webm.partial").exists());
        assert!(!directory.join(".recording.webm.partial").exists());
        assert!(!directory.join("recording.webm").exists());
        assert!(manifest["finalOutput"].is_null());
        assert_eq!(
            &std::fs::read(directory.join("segment-000000.webm")).unwrap()[0..4],
            [0x1a, 0x45, 0xdf, 0xa3]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "由 ci-local.sh 在隔离 Xvfb 中显式运行"]
    fn x11_source_records_a_complete_private_avi_session() {
        use crate::capture::RecordingCaptureSpec;
        use crate::recording::platform::x11::X11RegionFrameSource;
        use x11rb::connection::Connection;
        use x11rb::protocol::randr::ConnectionExt as _;
        use x11rb::rust_connection::RustConnection;

        let (connection, screen_number) = RustConnection::connect(None).expect("连接测试 X11");
        let root = connection.setup().roots[screen_number].root;
        connection
            .randr_query_version(1, 5)
            .expect("请求 RandR 版本")
            .reply()
            .expect("读取 RandR 版本");
        let monitors = connection
            .randr_get_monitors(root, true)
            .expect("请求 RandR 显示器")
            .reply()
            .expect("读取 RandR 显示器");
        let monitor = monitors
            .monitors
            .iter()
            .find(|monitor| !monitor.outputs.is_empty())
            .expect("Xvfb 至少暴露一个 RandR output");
        let width = u32::from(monitor.width).min(64);
        let height = u32::from(monitor.height).min(64);
        let source = X11RegionFrameSource::connect(RecordingCaptureSpec {
            monitor_id: monitor.outputs[0],
            monitor_pixel_width: u32::from(monitor.width),
            monitor_pixel_height: u32::from(monitor.height),
            crop_left: 0,
            crop_top: 0,
            crop_width: width,
            crop_height: height,
        })
        .expect("连接 X11 录屏帧源");
        let descriptor = source.descriptor().clone();
        assert_eq!(descriptor.physical_x, i32::from(monitor.x));
        assert_eq!(descriptor.physical_y, i32::from(monitor.y));
        assert_eq!((descriptor.width, descriptor.height), (width, height));
        let temporary = tempfile::tempdir().unwrap();
        let session = DiagnosticRecordingSession::start(
            temporary.path(),
            DiagnosticRecordingConfig {
                session_id: "x11-e2e".to_string(),
                source_id: descriptor.source_id,
                physical_x: descriptor.physical_x,
                physical_y: descriptor.physical_y,
                width: descriptor.width,
                height: descriptor.height,
                frames_per_second: 30,
                include_cursor: true,
                encoder: RecordingEncoder::MjpegDiagnostic { jpeg_quality: 85 },
                segment_duration_ns: DEFAULT_SEGMENT_DURATION_NS,
            },
            source,
        )
        .expect("启动 X11 诊断录屏");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while session.pipeline.stats().unwrap().accepted_frames < 2
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(session.pipeline.stats().unwrap().accepted_frames >= 2);
        let report = session.stop().expect("停止并提交 X11 诊断录屏");
        assert!(report.captured_frames >= 2);
        assert!(report.encoded_frames >= 1);
        assert_eq!(report.segment_paths.len(), 1);
        assert_eq!(
            &std::fs::read(&report.segment_paths[0]).unwrap()[0..4],
            b"RIFF"
        );
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(
                report.segment_paths[0]
                    .parent()
                    .unwrap()
                    .join("manifest.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["state"], "complete");
        assert_eq!(manifest["segments"][0]["frameCount"], report.encoded_frames);
        assert!(crate::private_files::is_private(&report.segment_paths[0]));
    }
}
