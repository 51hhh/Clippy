//! 录屏帧源到有界 pipeline 的持续采集 worker。
//!
//! worker 只负责按上限帧率取帧和入队；编码、文件与 UI 不得进入该线程。停止信号通过条件变量唤醒，
//! 不依赖轮询；`Drop` 会请求停止并回收线程，避免帧源在会话结束后继续读取桌面。

use super::frame::CapturedFrame;
use super::pipeline::{PipelineError, PushOutcome, RecordingPipeline};
use std::error::Error;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use thiserror::Error;

const MIN_CAPTURE_FPS: u32 = 1;
const MAX_CAPTURE_FPS: u32 = 120;

pub(super) trait RecordingFrameSource: Send + 'static {
    type Error: Error + Send + Sync + 'static;

    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error>;
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(super) enum CaptureWorkerError {
    #[error("录屏采集帧率必须在 1 到 120 之间")]
    InvalidFps,
    #[error("录屏采集源失败: {0}")]
    Source(String),
    #[error(transparent)]
    Pipeline(#[from] PipelineError),
    #[error("录屏采集控制锁已损坏")]
    ControlPoisoned,
    #[error("无法启动录屏采集线程: {0}")]
    ThreadSpawn(String),
    #[error("录屏采集线程异常退出")]
    ThreadPanicked,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct CaptureWorkerReport {
    pub captured_frames: u64,
    pub queued_frames: u64,
    pub ignored_while_paused: u64,
    pub dropped_by_backpressure: u64,
}

#[derive(Debug, Default)]
struct StopState {
    stopped: Mutex<bool>,
    wake: Condvar,
}

impl StopState {
    fn request(&self) -> Result<(), CaptureWorkerError> {
        *self
            .stopped
            .lock()
            .map_err(|_| CaptureWorkerError::ControlPoisoned)? = true;
        self.wake.notify_all();
        Ok(())
    }

    fn is_requested(&self) -> Result<bool, CaptureWorkerError> {
        self.stopped
            .lock()
            .map(|stopped| *stopped)
            .map_err(|_| CaptureWorkerError::ControlPoisoned)
    }

    fn wait(&self, duration: Duration) -> Result<(), CaptureWorkerError> {
        let stopped = self
            .stopped
            .lock()
            .map_err(|_| CaptureWorkerError::ControlPoisoned)?;
        if *stopped || duration.is_zero() {
            return Ok(());
        }
        let (_stopped, _timeout) = self
            .wake
            .wait_timeout_while(stopped, duration, |stopped| !*stopped)
            .map_err(|_| CaptureWorkerError::ControlPoisoned)?;
        Ok(())
    }
}

pub(super) struct CaptureWorker {
    stop: Arc<StopState>,
    join: Option<JoinHandle<Result<CaptureWorkerReport, CaptureWorkerError>>>,
}

impl CaptureWorker {
    pub fn spawn<S>(
        source: S,
        pipeline: Arc<RecordingPipeline>,
        frames_per_second: u32,
    ) -> Result<Self, CaptureWorkerError>
    where
        S: RecordingFrameSource,
    {
        let interval = capture_interval(frames_per_second)?;
        let stop = Arc::new(StopState::default());
        let worker_stop = Arc::clone(&stop);
        let join = thread::Builder::new()
            .name("clippy-recording-capture".to_string())
            .spawn(move || run_loop(source, &pipeline, &worker_stop, interval))
            .map_err(|error| CaptureWorkerError::ThreadSpawn(error.to_string()))?;
        Ok(Self {
            stop,
            join: Some(join),
        })
    }

    pub fn stop(mut self) -> Result<CaptureWorkerReport, CaptureWorkerError> {
        self.stop.request()?;
        self.join_inner()
    }

    pub fn wait(mut self) -> Result<CaptureWorkerReport, CaptureWorkerError> {
        self.join_inner()
    }

    fn join_inner(&mut self) -> Result<CaptureWorkerReport, CaptureWorkerError> {
        let Some(join) = self.join.take() else {
            return Err(CaptureWorkerError::ThreadPanicked);
        };
        join.join()
            .map_err(|_| CaptureWorkerError::ThreadPanicked)?
    }
}

impl Drop for CaptureWorker {
    fn drop(&mut self) {
        if let Some(join) = self.join.take() {
            let _ = self.stop.request();
            let _ = join.join();
        }
    }
}

fn capture_interval(frames_per_second: u32) -> Result<Duration, CaptureWorkerError> {
    if !(MIN_CAPTURE_FPS..=MAX_CAPTURE_FPS).contains(&frames_per_second) {
        return Err(CaptureWorkerError::InvalidFps);
    }
    Ok(Duration::from_nanos(
        1_000_000_000_u64 / u64::from(frames_per_second),
    ))
}

fn run_loop<S>(
    mut source: S,
    pipeline: &RecordingPipeline,
    stop: &StopState,
    interval: Duration,
) -> Result<CaptureWorkerReport, CaptureWorkerError>
where
    S: RecordingFrameSource,
{
    let mut report = CaptureWorkerReport::default();
    while !stop.is_requested()? {
        let started = Instant::now();
        let frame = source
            .capture_next()
            .map_err(|error| CaptureWorkerError::Source(error.to_string()))?;
        report.captured_frames = report.captured_frames.saturating_add(1);
        match pipeline.push(frame)? {
            PushOutcome::Queued { .. } => {
                report.queued_frames = report.queued_frames.saturating_add(1);
            }
            PushOutcome::QueuedAfterDropping { .. } => {
                report.queued_frames = report.queued_frames.saturating_add(1);
                report.dropped_by_backpressure = report.dropped_by_backpressure.saturating_add(1);
            }
            PushOutcome::IgnoredWhilePaused => {
                report.ignored_while_paused = report.ignored_while_paused.saturating_add(1);
            }
        }
        stop.wait(interval.saturating_sub(started.elapsed()))?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;

    #[derive(Debug, Clone, Copy)]
    struct FakeSourceError;

    impl fmt::Display for FakeSourceError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("fixture source failed")
        }
    }

    impl Error for FakeSourceError {}

    struct FakeSource {
        next_sequence: u64,
        fail_at: Option<u64>,
        stop_after: Option<(u64, Arc<StopState>)>,
    }

    impl RecordingFrameSource for FakeSource {
        type Error = FakeSourceError;

        fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
            if self.fail_at == Some(self.next_sequence) {
                return Err(FakeSourceError);
            }
            let sequence = self.next_sequence;
            self.next_sequence += 1;
            if let Some((last_sequence, stop)) = &self.stop_after {
                if sequence == *last_sequence {
                    stop.request().unwrap();
                }
            }
            Ok(CapturedFrame {
                sequence,
                captured_at_ns: 100 + sequence * 10,
                width: 2,
                height: 2,
                stride: 8,
                rgba: vec![sequence as u8; 16].into_boxed_slice(),
            })
        }
    }

    fn source() -> FakeSource {
        FakeSource {
            next_sequence: 0,
            fail_at: None,
            stop_after: None,
        }
    }

    #[test]
    fn fps_contract_rejects_zero_and_values_above_limit() {
        assert_eq!(capture_interval(0), Err(CaptureWorkerError::InvalidFps));
        assert_eq!(capture_interval(121), Err(CaptureWorkerError::InvalidFps));
        assert_eq!(capture_interval(120).unwrap().as_nanos(), 8_333_333);
    }

    #[test]
    fn loop_stops_without_capturing_when_already_cancelled() {
        let stop = StopState::default();
        stop.request().unwrap();
        let pipeline = RecordingPipeline::default();
        assert_eq!(
            run_loop(source(), &pipeline, &stop, Duration::ZERO).unwrap(),
            CaptureWorkerReport::default()
        );
    }

    #[test]
    fn loop_reports_backpressure_and_keeps_pipeline_bounded() {
        let stop = Arc::new(StopState::default());
        let mut source = source();
        source.stop_after = Some((4, Arc::clone(&stop)));
        let pipeline = RecordingPipeline::default();
        let report = run_loop(source, &pipeline, &stop, Duration::ZERO).unwrap();
        assert_eq!(
            report,
            CaptureWorkerReport {
                captured_frames: 5,
                queued_frames: 5,
                ignored_while_paused: 0,
                dropped_by_backpressure: 2,
            }
        );
        let stats = pipeline.stats().unwrap();
        assert_eq!(stats.queued_frames, 3);
        assert_eq!(stats.queued_bytes, 48);
        assert_eq!(stats.dropped_by_backpressure, 2);
    }

    #[test]
    fn source_failure_preserves_queued_prefix_and_returns_exact_error() {
        let stop = StopState::default();
        let pipeline = RecordingPipeline::default();
        let mut source = source();
        source.fail_at = Some(2);
        assert_eq!(
            run_loop(source, &pipeline, &stop, Duration::ZERO),
            Err(CaptureWorkerError::Source(
                "fixture source failed".to_string()
            ))
        );
        assert_eq!(pipeline.stats().unwrap().queued_frames, 2);
    }

    #[test]
    fn spawned_worker_stop_joins_and_releases_source() {
        let pipeline = Arc::new(RecordingPipeline::default());
        let worker = CaptureWorker::spawn(source(), Arc::clone(&pipeline), 120).unwrap();
        std::thread::sleep(Duration::from_millis(25));
        let report = worker.stop().unwrap();
        assert!(report.captured_frames >= 1);
        assert!(report.captured_frames <= 10);
        assert_eq!(report.captured_frames, report.queued_frames);
    }
}
