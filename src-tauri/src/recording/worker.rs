//! 录屏帧源到有界 pipeline 的持续采集 worker。
//!
//! worker 只负责按上限帧率取帧和入队；编码、文件与 UI 不得进入该线程。暂停/继续由采集线程使用
//! 帧源自己的单调时钟执行，暂停期间不读取桌面。`Drop` 会请求停止并回收线程，避免会话结束后留下
//! 活动帧源。

use super::frame::CapturedFrame;
use super::pipeline::{PipelineError, PushOutcome, RecordingPipeline};
use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use thiserror::Error;

const MIN_CAPTURE_FPS: u32 = 1;
const MAX_CAPTURE_FPS: u32 = 120;
const CONTROL_QUEUE_CAPACITY: usize = 8;
const PAUSED_STOP_POLL: Duration = Duration::from_millis(250);

/// 帧源只在采集线程内使用。部分系统对象（例如 macOS 的 Objective-C capture session）具有线程
/// 亲和性，因此帧源本身不要求 `Send`；跨线程移动的是可在线程内创建它的 factory。
pub(super) trait RecordingFrameSource: 'static {
    type Error: Error + Send + Sync + 'static;

    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error>;

    /// 推送型源可短轮询并在暂时没有新画面时返回 `None`，让控制命令保持低延迟。
    /// 拉取型源默认每次都产生一帧。
    fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
        self.capture_next().map(Some)
    }

    /// 返回与 `CapturedFrame::captured_at_ns` 相同时间基的下一单调时间戳。
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error>;

    /// 推送型平台源可在暂停时真正停止原生流；拉取型源沿用同一时钟即可。
    fn pause_capture(&mut self) -> Result<u64, Self::Error> {
        self.control_timestamp_ns()
    }

    /// 恢复成功后的时间戳是新时间线起点；平台源不得再交付该时刻之前缓存的旧帧。
    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        self.control_timestamp_ns()
    }

    /// 正常停止先关闭平台流，再用同一时钟域封尾。
    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        self.control_timestamp_ns()
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(super) enum CaptureWorkerError {
    #[error("录屏采集帧率必须在 1 到 120 之间")]
    InvalidFps,
    #[error("录屏采集源失败: {0}")]
    Source(String),
    #[error("录屏采集源初始化失败: {0}")]
    SourceInitialization(String),
    #[error("录屏采集源初始化时发生 panic")]
    SourceInitializationPanicked,
    #[error(transparent)]
    Pipeline(#[from] PipelineError),
    #[error("录屏采集控制通道已经关闭")]
    ControlDisconnected,
    #[error("录屏采集控制队列已满")]
    ControlQueueFull,
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
    /// 只有正常 Stop 完成 pipeline 封尾后才存在；异常退出与 Drop 回收会中止 pipeline。
    pub duration_ns: Option<u64>,
}

enum ControlCommand {
    Pause(SyncSender<Result<(), CaptureWorkerError>>),
    Resume(SyncSender<Result<(), CaptureWorkerError>>),
    Stop,
}

pub(super) struct CaptureWorker {
    control: SyncSender<ControlCommand>,
    stop_requested: Arc<AtomicBool>,
    join: Option<JoinHandle<Result<CaptureWorkerReport, CaptureWorkerError>>>,
}

impl CaptureWorker {
    pub fn spawn<S>(
        source: S,
        pipeline: Arc<RecordingPipeline>,
        frames_per_second: u32,
    ) -> Result<Self, CaptureWorkerError>
    where
        S: RecordingFrameSource + Send,
    {
        Self::spawn_with_factory(move || Ok(source), pipeline, frames_per_second)
    }

    /// 在采集线程内创建帧源，并在初始化结果明确后才把 worker 交给会话 owner。
    ///
    /// factory 是唯一跨线程的原生准备结果；返回的帧源可为 `!Send`，但创建、使用与析构必须全部
    /// 留在同一条采集线程。初始化失败和 panic 会先中止 pipeline，再同步返回给启动生命周期。
    pub fn spawn_with_factory<F, S>(
        factory: F,
        pipeline: Arc<RecordingPipeline>,
        frames_per_second: u32,
    ) -> Result<Self, CaptureWorkerError>
    where
        F: FnOnce() -> Result<S, String> + Send + 'static,
        S: RecordingFrameSource,
    {
        let interval = capture_interval(frames_per_second)?;
        let (control, commands) = mpsc::sync_channel(CONTROL_QUEUE_CAPACITY);
        let (ready, initialized) = mpsc::sync_channel(1);
        let stop_requested = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop_requested);
        let worker_pipeline = Arc::clone(&pipeline);
        let join = thread::Builder::new()
            .name("clippy-recording-capture".to_string())
            .spawn(move || {
                let source = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(factory)) {
                    Ok(Ok(source)) => source,
                    Ok(Err(message)) => {
                        let error = CaptureWorkerError::SourceInitialization(message);
                        let _ = worker_pipeline.abort();
                        let _ = ready.send(Err(error.clone()));
                        return Err(error);
                    }
                    Err(_) => {
                        let error = CaptureWorkerError::SourceInitializationPanicked;
                        let _ = worker_pipeline.abort();
                        let _ = ready.send(Err(error.clone()));
                        return Err(error);
                    }
                };
                if ready.send(Ok(())).is_err() {
                    let _ = worker_pipeline.abort();
                    return Err(CaptureWorkerError::ControlDisconnected);
                }
                run_loop(source, &worker_pipeline, commands, &worker_stop, interval)
            })
            .map_err(|error| CaptureWorkerError::ThreadSpawn(error.to_string()))?;
        let mut worker = Self {
            control,
            stop_requested,
            join: Some(join),
        };
        match initialized.recv() {
            Ok(Ok(())) => Ok(worker),
            Ok(Err(error)) => {
                let _ = worker.join_inner();
                Err(error)
            }
            Err(_) => match worker.join_inner() {
                Err(error) => Err(error),
                Ok(_) => Err(CaptureWorkerError::ControlDisconnected),
            },
        }
    }

    pub fn pause(&self) -> Result<(), CaptureWorkerError> {
        self.send_control(ControlCommand::Pause)
    }

    pub fn resume(&self) -> Result<(), CaptureWorkerError> {
        self.send_control(ControlCommand::Resume)
    }

    pub fn stop(mut self) -> Result<CaptureWorkerReport, CaptureWorkerError> {
        // 正常停止必须由采集线程从帧源读取同一时钟域的终点，不能由 owner 的本地时钟代替。
        // 通道已经断开时仍然 join，以返回采集线程的原始错误。
        let _ = self.control.send(ControlCommand::Stop);
        self.join_inner()
    }

    pub fn wait(mut self) -> Result<CaptureWorkerReport, CaptureWorkerError> {
        self.join_inner()
    }

    fn send_control(
        &self,
        build: fn(SyncSender<Result<(), CaptureWorkerError>>) -> ControlCommand,
    ) -> Result<(), CaptureWorkerError> {
        let (reply, result) = mpsc::sync_channel(1);
        self.control
            .try_send(build(reply))
            .map_err(map_control_send_error)?;
        result
            .recv()
            .map_err(|_| CaptureWorkerError::ControlDisconnected)??;
        Ok(())
    }

    fn request_stop(&self) {
        self.stop_requested.store(true, Ordering::Release);
        match self.control.try_send(ControlCommand::Stop) {
            Ok(()) | Err(TrySendError::Disconnected(_)) | Err(TrySendError::Full(_)) => {}
        }
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
            self.request_stop();
            let _ = join.join();
        }
    }
}

fn map_control_send_error(error: TrySendError<ControlCommand>) -> CaptureWorkerError {
    match error {
        TrySendError::Full(_) => CaptureWorkerError::ControlQueueFull,
        TrySendError::Disconnected(_) => CaptureWorkerError::ControlDisconnected,
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
    commands: Receiver<ControlCommand>,
    stop_requested: &AtomicBool,
    interval: Duration,
) -> Result<CaptureWorkerReport, CaptureWorkerError>
where
    S: RecordingFrameSource,
{
    let mut abort_guard = PipelineAbortGuard::new(pipeline);
    let result = run_loop_inner(&mut source, pipeline, commands, stop_requested, interval);
    if result
        .as_ref()
        .is_ok_and(|report| report.duration_ns.is_some())
    {
        abort_guard.disarm();
    }
    result
}

fn run_loop_inner<S>(
    source: &mut S,
    pipeline: &RecordingPipeline,
    commands: Receiver<ControlCommand>,
    stop_requested: &AtomicBool,
    interval: Duration,
) -> Result<CaptureWorkerReport, CaptureWorkerError>
where
    S: RecordingFrameSource,
{
    let mut report = CaptureWorkerReport::default();
    let mut paused = false;
    while !stop_requested.load(Ordering::Acquire) {
        if paused {
            match commands.recv_timeout(PAUSED_STOP_POLL) {
                Ok(command) => {
                    if let Some(duration_ns) =
                        handle_command(command, source, pipeline, &mut paused)?
                    {
                        report.duration_ns = Some(duration_ns);
                        return Ok(report);
                    }
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(CaptureWorkerError::ControlDisconnected);
                }
            }
            continue;
        }

        let started = Instant::now();
        let frame = source
            .capture_next_available()
            .map_err(|error| CaptureWorkerError::Source(error.to_string()))?;
        if let Some(frame) = frame {
            report.captured_frames = report.captured_frames.saturating_add(1);
            match pipeline.push(frame)? {
                PushOutcome::Queued { .. } => {
                    report.queued_frames = report.queued_frames.saturating_add(1);
                }
                PushOutcome::QueuedAfterDropping { .. } => {
                    report.queued_frames = report.queued_frames.saturating_add(1);
                    report.dropped_by_backpressure =
                        report.dropped_by_backpressure.saturating_add(1);
                }
                PushOutcome::IgnoredWhilePaused => {
                    report.ignored_while_paused = report.ignored_while_paused.saturating_add(1);
                }
            }
        }

        let remaining = interval.saturating_sub(started.elapsed());
        match commands.recv_timeout(remaining) {
            Ok(command) => {
                if let Some(duration_ns) = handle_command(command, source, pipeline, &mut paused)? {
                    report.duration_ns = Some(duration_ns);
                    return Ok(report);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(CaptureWorkerError::ControlDisconnected);
            }
        }
    }
    Ok(report)
}

fn handle_command<S>(
    command: ControlCommand,
    source: &mut S,
    pipeline: &RecordingPipeline,
    paused: &mut bool,
) -> Result<Option<u64>, CaptureWorkerError>
where
    S: RecordingFrameSource,
{
    match command {
        ControlCommand::Pause(reply) => {
            let timestamp = match source.pause_capture() {
                Ok(timestamp) => timestamp,
                Err(error) => {
                    let error = CaptureWorkerError::Source(error.to_string());
                    let _ = reply.send(Err(error.clone()));
                    return Err(error);
                }
            };
            let result = pipeline.pause(timestamp).map_err(CaptureWorkerError::from);
            if result.is_ok() {
                *paused = true;
            }
            let _ = reply.send(result);
            Ok(None)
        }
        ControlCommand::Resume(reply) => {
            let timestamp = match source.resume_capture() {
                Ok(timestamp) => timestamp,
                Err(error) => {
                    let error = CaptureWorkerError::Source(error.to_string());
                    let _ = reply.send(Err(error.clone()));
                    return Err(error);
                }
            };
            let result = pipeline.resume(timestamp).map_err(CaptureWorkerError::from);
            if result.is_ok() {
                *paused = false;
            }
            let _ = reply.send(result);
            Ok(None)
        }
        ControlCommand::Stop => {
            let timestamp = source
                .stop_capture()
                .map_err(|error| CaptureWorkerError::Source(error.to_string()))?;
            Ok(Some(pipeline.finish(timestamp)?))
        }
    }
}

struct PipelineAbortGuard<'a> {
    pipeline: &'a RecordingPipeline,
    armed: bool,
}

impl<'a> PipelineAbortGuard<'a> {
    fn new(pipeline: &'a RecordingPipeline) -> Self {
        Self {
            pipeline,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PipelineAbortGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.pipeline.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::pipeline::PipelineDrain;
    use std::fmt;
    use std::rc::Rc;
    use std::sync::Mutex;

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
        next_timestamp_ns: u64,
        fail_at: Option<u64>,
        fail_control: bool,
        stop_after: Option<(u64, Arc<AtomicBool>)>,
    }

    impl RecordingFrameSource for FakeSource {
        type Error = FakeSourceError;

        fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
            if self.fail_at == Some(self.next_sequence) {
                return Err(FakeSourceError);
            }
            let sequence = self.next_sequence;
            let captured_at_ns = self.next_timestamp_ns;
            self.next_sequence += 1;
            self.next_timestamp_ns += 10;
            if let Some((last_sequence, stop)) = &self.stop_after {
                if sequence == *last_sequence {
                    stop.store(true, Ordering::Release);
                }
            }
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
            if self.fail_control {
                return Err(FakeSourceError);
            }
            let timestamp = self.next_timestamp_ns;
            self.next_timestamp_ns += 1;
            Ok(timestamp)
        }
    }

    fn source() -> FakeSource {
        FakeSource {
            next_sequence: 0,
            next_timestamp_ns: 100,
            fail_at: None,
            fail_control: false,
            stop_after: None,
        }
    }

    struct HookSource {
        inner: FakeSource,
        hooks: Arc<Mutex<Vec<&'static str>>>,
    }

    impl RecordingFrameSource for HookSource {
        type Error = FakeSourceError;

        fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
            self.inner.capture_next()
        }

        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            self.inner.control_timestamp_ns()
        }

        fn pause_capture(&mut self) -> Result<u64, Self::Error> {
            self.hooks.lock().unwrap().push("pause");
            self.inner.control_timestamp_ns()
        }

        fn resume_capture(&mut self) -> Result<u64, Self::Error> {
            self.hooks.lock().unwrap().push("resume");
            self.inner.control_timestamp_ns()
        }

        fn stop_capture(&mut self) -> Result<u64, Self::Error> {
            self.hooks.lock().unwrap().push("stop");
            self.inner.control_timestamp_ns()
        }
    }

    struct IdlePushSource {
        inner: FakeSource,
        emitted_first_frame: bool,
        hooks: Arc<Mutex<Vec<&'static str>>>,
    }

    struct ThreadBoundSource {
        inner: FakeSource,
        _thread_affinity: Rc<()>,
    }

    impl RecordingFrameSource for ThreadBoundSource {
        type Error = FakeSourceError;

        fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
            self.inner.capture_next()
        }

        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            self.inner.control_timestamp_ns()
        }
    }

    impl RecordingFrameSource for IdlePushSource {
        type Error = FakeSourceError;

        fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
            unreachable!("推送型源必须使用可空的短轮询入口")
        }

        fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
            if self.emitted_first_frame {
                Ok(None)
            } else {
                self.emitted_first_frame = true;
                self.inner.capture_next().map(Some)
            }
        }

        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            self.inner.control_timestamp_ns()
        }

        fn pause_capture(&mut self) -> Result<u64, Self::Error> {
            self.hooks.lock().unwrap().push("pause");
            self.control_timestamp_ns()
        }

        fn stop_capture(&mut self) -> Result<u64, Self::Error> {
            self.hooks.lock().unwrap().push("stop");
            self.control_timestamp_ns()
        }
    }

    fn controls() -> (
        SyncSender<ControlCommand>,
        Receiver<ControlCommand>,
        Arc<AtomicBool>,
    ) {
        let (control, commands) = mpsc::sync_channel(CONTROL_QUEUE_CAPACITY);
        (control, commands, Arc::new(AtomicBool::new(false)))
    }

    #[test]
    fn fps_contract_rejects_zero_and_values_above_limit() {
        assert_eq!(capture_interval(0), Err(CaptureWorkerError::InvalidFps));
        assert_eq!(capture_interval(121), Err(CaptureWorkerError::InvalidFps));
        assert_eq!(capture_interval(120).unwrap().as_nanos(), 8_333_333);
    }

    #[test]
    fn loop_stops_without_capturing_when_already_cancelled() {
        let (_control, commands, stop) = controls();
        stop.store(true, Ordering::Release);
        let pipeline = RecordingPipeline::default();
        assert_eq!(
            run_loop(source(), &pipeline, commands, &stop, Duration::ZERO).unwrap(),
            CaptureWorkerReport::default()
        );
    }

    #[test]
    fn loop_reports_backpressure_and_keeps_pipeline_bounded() {
        let (_control, commands, stop) = controls();
        let mut source = source();
        source.stop_after = Some((4, Arc::clone(&stop)));
        let pipeline = RecordingPipeline::default();
        let report = run_loop(source, &pipeline, commands, &stop, Duration::ZERO).unwrap();
        assert_eq!(
            report,
            CaptureWorkerReport {
                captured_frames: 5,
                queued_frames: 5,
                ignored_while_paused: 0,
                dropped_by_backpressure: 2,
                duration_ns: None,
            }
        );
        let stats = pipeline.stats().unwrap();
        assert_eq!(stats.queued_frames, 3);
        assert_eq!(stats.queued_bytes, 48);
        assert_eq!(stats.dropped_by_backpressure, 2);
        for _ in 0..3 {
            assert!(matches!(pipeline.pop_wait(), Ok(PipelineDrain::Frame(_))));
        }
        assert!(matches!(pipeline.pop_wait(), Err(PipelineError::Aborted)));
    }

    #[test]
    fn source_failure_preserves_queued_prefix_and_returns_exact_error() {
        let (_control, commands, stop) = controls();
        let pipeline = RecordingPipeline::default();
        let mut source = source();
        source.fail_at = Some(2);
        assert_eq!(
            run_loop(source, &pipeline, commands, &stop, Duration::ZERO),
            Err(CaptureWorkerError::Source(
                "fixture source failed".to_string()
            ))
        );
        assert_eq!(pipeline.stats().unwrap().queued_frames, 2);
        assert!(pipeline.pop().unwrap().is_some());
        assert!(pipeline.pop().unwrap().is_some());
        assert!(matches!(pipeline.pop_wait(), Err(PipelineError::Aborted)));
    }

    #[test]
    fn spawned_worker_pauses_without_capture_and_resumes_same_timeline() {
        let pipeline = Arc::new(RecordingPipeline::default());
        let worker = CaptureWorker::spawn(source(), Arc::clone(&pipeline), 120).unwrap();
        std::thread::sleep(Duration::from_millis(25));
        worker.pause().unwrap();
        let paused_at = pipeline.stats().unwrap().accepted_frames;
        assert!(paused_at >= 1);
        std::thread::sleep(Duration::from_millis(25));
        assert_eq!(pipeline.stats().unwrap().accepted_frames, paused_at);
        worker.resume().unwrap();
        std::thread::sleep(Duration::from_millis(25));
        let report = worker.stop().unwrap();
        assert!(pipeline.stats().unwrap().accepted_frames > paused_at);
        assert_eq!(report.ignored_while_paused, 0);
        let duration_ns = report.duration_ns.expect("正常停止必须返回最终时长");
        loop {
            match pipeline.pop_wait().unwrap() {
                PipelineDrain::Frame(_) => {}
                PipelineDrain::Finished {
                    duration_ns: terminal_duration,
                } => {
                    assert_eq!(terminal_duration, duration_ns);
                    break;
                }
            }
        }
    }

    #[test]
    fn factory_constructs_and_drops_non_send_source_inside_capture_thread() {
        let pipeline = Arc::new(RecordingPipeline::default());
        let worker = CaptureWorker::spawn_with_factory(
            || {
                Ok(ThreadBoundSource {
                    inner: source(),
                    _thread_affinity: Rc::new(()),
                })
            },
            Arc::clone(&pipeline),
            120,
        )
        .unwrap();
        let report = worker.stop().unwrap();
        assert!(report.captured_frames >= 1);
        assert!(report.duration_ns.is_some());
    }

    #[test]
    fn factory_failure_is_returned_synchronously_and_aborts_pipeline() {
        let pipeline = Arc::new(RecordingPipeline::default());
        let error = match CaptureWorker::spawn_with_factory(
            || Err::<FakeSource, _>("native source unavailable".to_string()),
            Arc::clone(&pipeline),
            120,
        ) {
            Ok(worker) => {
                drop(worker);
                panic!("初始化失败不能返回活动 worker");
            }
            Err(error) => error,
        };
        assert_eq!(
            error,
            CaptureWorkerError::SourceInitialization("native source unavailable".to_string())
        );
        assert!(matches!(pipeline.pop_wait(), Err(PipelineError::Aborted)));
    }

    #[test]
    fn factory_panic_is_returned_without_stranding_pipeline() {
        let pipeline = Arc::new(RecordingPipeline::default());
        let error = match CaptureWorker::spawn_with_factory(
            || -> Result<FakeSource, String> { panic!("fixture initialization panic") },
            Arc::clone(&pipeline),
            120,
        ) {
            Ok(worker) => {
                drop(worker);
                panic!("初始化 panic 不能返回活动 worker");
            }
            Err(error) => error,
        };
        assert_eq!(error, CaptureWorkerError::SourceInitializationPanicked);
        assert!(matches!(pipeline.pop_wait(), Err(PipelineError::Aborted)));
    }

    #[test]
    fn worker_routes_pause_resume_and_stop_through_platform_hooks() {
        let hooks = Arc::new(Mutex::new(Vec::new()));
        let platform_source = HookSource {
            inner: source(),
            hooks: Arc::clone(&hooks),
        };
        let pipeline = Arc::new(RecordingPipeline::default());
        let worker = CaptureWorker::spawn(platform_source, pipeline, 120).unwrap();
        std::thread::sleep(Duration::from_millis(15));
        worker.pause().unwrap();
        worker.resume().unwrap();
        worker.stop().unwrap();
        assert_eq!(*hooks.lock().unwrap(), ["pause", "resume", "stop"]);
    }

    #[test]
    fn static_push_source_stays_controllable_without_counting_missing_frames() {
        let hooks = Arc::new(Mutex::new(Vec::new()));
        let source = IdlePushSource {
            inner: source(),
            emitted_first_frame: false,
            hooks: Arc::clone(&hooks),
        };
        let pipeline = Arc::new(RecordingPipeline::default());
        let worker = CaptureWorker::spawn(source, pipeline, 120).unwrap();
        std::thread::sleep(Duration::from_millis(15));
        worker.pause().unwrap();
        let report = worker.stop().unwrap();
        assert_eq!(report.captured_frames, 1);
        assert_eq!(report.queued_frames, 1);
        assert!(report.duration_ns.is_some());
        assert_eq!(*hooks.lock().unwrap(), ["pause", "stop"]);
    }

    #[test]
    fn repeated_pause_and_resume_return_pipeline_state_errors() {
        let pipeline = Arc::new(RecordingPipeline::default());
        let worker = CaptureWorker::spawn(source(), pipeline, 120).unwrap();
        std::thread::sleep(Duration::from_millis(15));
        worker.pause().unwrap();
        assert!(matches!(
            worker.pause(),
            Err(CaptureWorkerError::Pipeline(PipelineError::Timeline(_)))
        ));
        worker.resume().unwrap();
        assert!(matches!(
            worker.resume(),
            Err(CaptureWorkerError::Pipeline(PipelineError::Timeline(_)))
        ));
        worker.stop().unwrap();
    }

    #[test]
    fn control_clock_failure_is_reported_to_caller_and_terminates_worker() {
        let pipeline = Arc::new(RecordingPipeline::default());
        let mut source = source();
        source.fail_control = true;
        let worker = CaptureWorker::spawn(source, Arc::clone(&pipeline), 120).unwrap();
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(
            worker.pause(),
            Err(CaptureWorkerError::Source(
                "fixture source failed".to_string()
            ))
        );
        assert_eq!(
            worker.wait(),
            Err(CaptureWorkerError::Source(
                "fixture source failed".to_string()
            ))
        );
        while pipeline.pop().unwrap().is_some() {}
        assert!(matches!(pipeline.pop_wait(), Err(PipelineError::Aborted)));
    }

    #[test]
    fn stopping_while_paused_finishes_without_resuming_capture() {
        let pipeline = Arc::new(RecordingPipeline::default());
        let worker = CaptureWorker::spawn(source(), Arc::clone(&pipeline), 120).unwrap();
        std::thread::sleep(Duration::from_millis(15));
        worker.pause().unwrap();
        let accepted_at_pause = pipeline.stats().unwrap().accepted_frames;
        std::thread::sleep(Duration::from_millis(20));
        let report = worker.stop().unwrap();
        assert_eq!(pipeline.stats().unwrap().accepted_frames, accepted_at_pause);
        let duration_ns = report.duration_ns.expect("暂停中正常停止仍应封尾");
        loop {
            match pipeline.pop_wait().unwrap() {
                PipelineDrain::Frame(_) => {}
                PipelineDrain::Finished {
                    duration_ns: terminal_duration,
                } => {
                    assert_eq!(terminal_duration, duration_ns);
                    break;
                }
            }
        }
    }

    #[test]
    fn stop_clock_failure_aborts_pipeline_and_returns_source_error() {
        let pipeline = Arc::new(RecordingPipeline::default());
        let mut source = source();
        source.fail_control = true;
        let worker = CaptureWorker::spawn(source, Arc::clone(&pipeline), 120).unwrap();
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(
            worker.stop(),
            Err(CaptureWorkerError::Source(
                "fixture source failed".to_string()
            ))
        );
        while pipeline.pop().unwrap().is_some() {}
        assert!(matches!(pipeline.pop_wait(), Err(PipelineError::Aborted)));
    }
}
