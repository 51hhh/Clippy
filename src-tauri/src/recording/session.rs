//! 诊断录屏会话的单一资源 owner。
//!
//! 会话同时持有 journal、临时分段、采集线程、三槽 pipeline 和编码线程。正常停止只由这一个 owner
//! 提交分段与 complete；任一错误或 `Drop` 都中止两条线程、清理未提交临时文件并把 journal 标为
//! interrupted，避免各层分别猜测资源是否已经释放。

use super::encoder_worker::{
    DiagnosticEncoderError, DiagnosticEncoderReport, DiagnosticEncoderWorker,
};
use super::manifest::{PendingSegment, RecordingJournal, RecordingJournalConfig};
use super::mux::avi_mjpeg::{AviMjpegError, AviMjpegWriter};
use super::pipeline::{PipelineError, RecordingPipeline};
use super::worker::{CaptureWorker, CaptureWorkerError, CaptureWorkerReport, RecordingFrameSource};
use std::fs::File;
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
    pub jpeg_quality: u8,
}

#[derive(Debug, Error)]
pub(super) enum DiagnosticRecordingError {
    #[error("诊断录屏配置无效")]
    InvalidConfiguration,
    #[error("录屏 journal 失败: {0}")]
    Journal(String),
    #[error(transparent)]
    Avi(#[from] AviMjpegError),
    #[error(transparent)]
    Capture(#[from] CaptureWorkerError),
    #[error(transparent)]
    Encoder(#[from] DiagnosticEncoderError),
    #[error(transparent)]
    Pipeline(#[from] PipelineError),
    #[error("采集线程与编码线程的最终时长不一致")]
    DurationMismatch,
    #[error("采集线程与 pipeline 的背压计数不一致")]
    BackpressureMismatch,
    #[error("录屏会话资源已经被消费")]
    AlreadySettled,
}

pub(super) struct DiagnosticRecordingReport {
    pub segment_path: PathBuf,
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
    encoder: Option<DiagnosticEncoderWorker<File>>,
    journal: RecordingJournal,
    pending_segment: Option<PendingSegment>,
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
        if !(1..=120).contains(&config.frames_per_second)
            || !(1..=100).contains(&config.jpeg_quality)
        {
            return Err(DiagnosticRecordingError::InvalidConfiguration);
        }
        let mut journal = RecordingJournal::create(
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
                encoder: "mjpeg-diagnostic".to_string(),
                container: "avi".to_string(),
                include_cursor: config.include_cursor,
            },
        )
        .map_err(DiagnosticRecordingError::Journal)?;
        let (file, pending_segment) = match journal.begin_segment() {
            Ok(segment) => segment,
            Err(error) => {
                interrupt_after_start_failure(&mut journal);
                return Err(DiagnosticRecordingError::Journal(error));
            }
        };
        let writer = match AviMjpegWriter::new(
            file,
            config.width,
            config.height,
            config.frames_per_second,
            1,
            config.jpeg_quality,
        ) {
            Ok(writer) => writer,
            Err(error) => {
                drop(pending_segment);
                interrupt_after_start_failure(&mut journal);
                return Err(error.into());
            }
        };
        let pipeline = Arc::new(RecordingPipeline::default());
        let encoder = match DiagnosticEncoderWorker::spawn(writer, Arc::clone(&pipeline)) {
            Ok(encoder) => encoder,
            Err(error) => {
                drop(pending_segment);
                interrupt_after_start_failure(&mut journal);
                return Err(error.into());
            }
        };
        let capture =
            match CaptureWorker::spawn(source, Arc::clone(&pipeline), config.frames_per_second) {
                Ok(capture) => capture,
                Err(error) => {
                    drop(encoder);
                    drop(pending_segment);
                    interrupt_after_start_failure(&mut journal);
                    return Err(error.into());
                }
            };
        Ok(Self {
            pipeline,
            capture: Some(capture),
            encoder: Some(encoder),
            journal,
            pending_segment: Some(pending_segment),
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
        let encoder_result = encoder.wait();
        let (capture_report, encoder_report) = match (capture_result, encoder_result) {
            (Ok(capture_report), Ok(encoder_report)) => (capture_report, encoder_report),
            (Err(CaptureWorkerError::Pipeline(PipelineError::Aborted)), Err(encoder_error)) => {
                return self.fail(encoder_error.into());
            }
            (Err(capture_error), _) => return self.fail(capture_error.into()),
            (Ok(_), Err(encoder_error)) => return self.fail(encoder_error.into()),
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
        self.journal.session_directory()
    }

    fn commit_reports(
        &mut self,
        capture: CaptureWorkerReport,
        encoder: DiagnosticEncoderReport<File>,
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
        let pending = self
            .pending_segment
            .take()
            .ok_or(DiagnosticRecordingError::AlreadySettled)?;
        let segment_path = self
            .journal
            .commit_segment(
                pending,
                encoder.writer,
                duration_ns,
                encoder.encoded_frames,
                stats.dropped_by_backpressure,
            )
            .map_err(DiagnosticRecordingError::Journal)?;
        self.journal
            .complete()
            .map_err(DiagnosticRecordingError::Journal)?;
        Ok(DiagnosticRecordingReport {
            segment_path,
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
        drop(self.pending_segment.take());
        if let Err(cleanup_error) = self.journal.interrupt() {
            log::warn!("录屏失败后写入 interrupted 状态失败: {cleanup_error}");
        }
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
        drop(self.pending_segment.take());
        if let Err(error) = self.journal.interrupt() {
            log::warn!("回收未完成录屏会话时写入 interrupted 状态失败: {error}");
        }
        self.settled = true;
    }
}

fn interrupt_after_start_failure(journal: &mut RecordingJournal) {
    if let Err(error) = journal.interrupt() {
        log::warn!("录屏启动失败后写入 interrupted 状态失败: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::frame::CapturedFrame;
    use std::convert::Infallible;
    use std::fmt;

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
            jpeg_quality: 85,
        }
    }

    fn manifest_state(directory: &Path) -> String {
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.join("manifest.json")).unwrap())
                .unwrap();
        value["state"].as_str().unwrap().to_string()
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
        assert_eq!(&std::fs::read(report.segment_path).unwrap()[0..4], b"RIFF");
        assert!(!directory.join(".segment-000000.avi.partial").exists());
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
            Err(DiagnosticRecordingError::Encoder(
                DiagnosticEncoderError::Avi(AviMjpegError::InvalidFrame)
            ))
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
                jpeg_quality: 85,
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
        assert_eq!(&std::fs::read(&report.segment_path).unwrap()[0..4], b"RIFF");
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(report.segment_path.parent().unwrap().join("manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["state"], "complete");
        assert_eq!(manifest["segments"][0]["frameCount"], report.encoded_frames);
        assert!(crate::private_files::is_private(&report.segment_path));
    }
}
