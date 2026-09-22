//! 平台音频 source 到有界 PCM pipeline 的采集线程。
//!
//! 原生对象在本线程内创建、控制并销毁。source 负责把原生 PTS 映射到共享会话时钟并按有界等待
//! 返回归一化 PCM；worker 负责暂停、停止、背压失败和线程回收。

use super::audio::{AudioPipeline, AudioPipelineError, AudioPushOutcome, CapturedAudioChunk};
use super::clock::RecordingSessionClock;
use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use thiserror::Error;

const CONTROL_QUEUE_CAPACITY: usize = 8;
const SOURCE_POLL_TIMEOUT: Duration = Duration::from_millis(50);
const PAUSED_STOP_POLL: Duration = Duration::from_millis(250);

/// 平台音频对象只在采集线程内存在，因此不要求 `Send`。
///
/// `capture_next_available` 必须在 `timeout` 内返回；推送型 API 应在内部使用有界桥接队列。块时间戳
/// 必须来自原生 PTS 到传入 `RecordingSessionClock` 的映射，不能使用回调抵达时刻。
pub(super) trait RecordingAudioSource: 'static {
    type Error: Error + Send + Sync + 'static;

    fn capture_next_available(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error>;

    /// 返回与 `CapturedAudioChunk::captured_at_ns` 相同会话时间域的控制时间戳。
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error>;

    /// 停止平台回调并清除迟到块后返回暂停时刻。
    fn pause_capture(&mut self) -> Result<u64, Self::Error> {
        self.control_timestamp_ns()
    }

    /// 恢复平台回调并丢弃暂停前缓存后返回恢复时刻。
    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        self.control_timestamp_ns()
    }

    /// 关闭平台流并等待回调退出，再返回不早于最后 PCM frame 末尾的时刻。
    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        self.control_timestamp_ns()
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(super) enum AudioCaptureWorkerError {
    #[error("录屏音频源失败: {0}")]
    Source(String),
    #[error("录屏音频源初始化失败: {0}")]
    SourceInitialization(String),
    #[error("录屏音频源初始化时发生 panic")]
    SourceInitializationPanicked,
    #[error(transparent)]
    Pipeline(#[from] AudioPipelineError),
    #[error("录屏音频控制通道已经关闭")]
    ControlDisconnected,
    #[error("录屏音频控制队列已满")]
    ControlQueueFull,
    #[error("无法启动录屏音频采集线程: {0}")]
    ThreadSpawn(String),
    #[error("录屏音频采集线程异常退出")]
    ThreadPanicked,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct AudioCaptureWorkerReport {
    pub captured_chunks: u64,
    pub captured_frames: u64,
    pub queued_chunks: u64,
    /// 只有显式 Stop 完成时间线封尾时存在；错误与 Drop 回收会中止 pipeline。
    pub duration_ns: Option<u64>,
}

enum ControlCommand {
    Pause(SyncSender<Result<(), AudioCaptureWorkerError>>),
    Resume(SyncSender<Result<(), AudioCaptureWorkerError>>),
    Stop,
}

pub(super) struct AudioCaptureWorker {
    control: SyncSender<ControlCommand>,
    stop_requested: Arc<AtomicBool>,
    join: Option<JoinHandle<Result<AudioCaptureWorkerReport, AudioCaptureWorkerError>>>,
}

impl AudioCaptureWorker {
    /// factory 在音频线程内取得与视频相同的会话时钟，再创建线程亲和的原生 source。
    pub fn spawn_with_factory<F, S>(
        factory: F,
        clock: RecordingSessionClock,
        pipeline: Arc<AudioPipeline>,
    ) -> Result<Self, AudioCaptureWorkerError>
    where
        F: FnOnce(RecordingSessionClock) -> Result<S, String> + Send + 'static,
        S: RecordingAudioSource,
    {
        let (control, commands) = mpsc::sync_channel(CONTROL_QUEUE_CAPACITY);
        let (ready, initialized) = mpsc::sync_channel(1);
        let stop_requested = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop_requested);
        let worker_pipeline = Arc::clone(&pipeline);
        let join = thread::Builder::new()
            .name("clippy-recording-audio-capture".to_string())
            .spawn(move || {
                let source =
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| factory(clock)))
                    {
                        Ok(Ok(source)) => source,
                        Ok(Err(message)) => {
                            let error = AudioCaptureWorkerError::SourceInitialization(message);
                            let _ = worker_pipeline.abort();
                            let _ = ready.send(Err(error.clone()));
                            return Err(error);
                        }
                        Err(_) => {
                            let error = AudioCaptureWorkerError::SourceInitializationPanicked;
                            let _ = worker_pipeline.abort();
                            let _ = ready.send(Err(error.clone()));
                            return Err(error);
                        }
                    };
                if ready.send(Ok(())).is_err() {
                    let _ = worker_pipeline.abort();
                    return Err(AudioCaptureWorkerError::ControlDisconnected);
                }
                run_loop(source, &worker_pipeline, commands, &worker_stop)
            })
            .map_err(|error| {
                let _ = pipeline.abort();
                AudioCaptureWorkerError::ThreadSpawn(error.to_string())
            })?;
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
                Ok(_) => Err(AudioCaptureWorkerError::ControlDisconnected),
            },
        }
    }

    pub fn pause(&self) -> Result<(), AudioCaptureWorkerError> {
        self.send_control(ControlCommand::Pause)
    }

    pub fn resume(&self) -> Result<(), AudioCaptureWorkerError> {
        self.send_control(ControlCommand::Resume)
    }

    pub fn stop(mut self) -> Result<AudioCaptureWorkerReport, AudioCaptureWorkerError> {
        let _ = self.control.send(ControlCommand::Stop);
        self.join_inner()
    }

    pub fn wait(mut self) -> Result<AudioCaptureWorkerReport, AudioCaptureWorkerError> {
        self.join_inner()
    }

    fn send_control(
        &self,
        build: fn(SyncSender<Result<(), AudioCaptureWorkerError>>) -> ControlCommand,
    ) -> Result<(), AudioCaptureWorkerError> {
        let (reply, result) = mpsc::sync_channel(1);
        self.control
            .try_send(build(reply))
            .map_err(map_control_send_error)?;
        result
            .recv()
            .map_err(|_| AudioCaptureWorkerError::ControlDisconnected)??;
        Ok(())
    }

    fn request_abort(&self) {
        self.stop_requested.store(true, Ordering::Release);
    }

    fn join_inner(&mut self) -> Result<AudioCaptureWorkerReport, AudioCaptureWorkerError> {
        let Some(join) = self.join.take() else {
            return Err(AudioCaptureWorkerError::ThreadPanicked);
        };
        join.join()
            .map_err(|_| AudioCaptureWorkerError::ThreadPanicked)?
    }
}

impl Drop for AudioCaptureWorker {
    fn drop(&mut self) {
        if let Some(join) = self.join.take() {
            self.request_abort();
            let _ = join.join();
        }
    }
}

fn map_control_send_error(error: TrySendError<ControlCommand>) -> AudioCaptureWorkerError {
    match error {
        TrySendError::Full(_) => AudioCaptureWorkerError::ControlQueueFull,
        TrySendError::Disconnected(_) => AudioCaptureWorkerError::ControlDisconnected,
    }
}

fn run_loop<S>(
    mut source: S,
    pipeline: &AudioPipeline,
    commands: Receiver<ControlCommand>,
    stop_requested: &AtomicBool,
) -> Result<AudioCaptureWorkerReport, AudioCaptureWorkerError>
where
    S: RecordingAudioSource,
{
    let mut abort_guard = AudioPipelineAbortGuard::new(pipeline);
    let result = run_loop_inner(&mut source, pipeline, commands, stop_requested);
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
    pipeline: &AudioPipeline,
    commands: Receiver<ControlCommand>,
    stop_requested: &AtomicBool,
) -> Result<AudioCaptureWorkerReport, AudioCaptureWorkerError>
where
    S: RecordingAudioSource,
{
    let mut report = AudioCaptureWorkerReport::default();
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
                    return Err(AudioCaptureWorkerError::ControlDisconnected);
                }
            }
            continue;
        }

        if let Some(duration_ns) = try_handle_command(&commands, source, pipeline, &mut paused)? {
            report.duration_ns = Some(duration_ns);
            return Ok(report);
        }
        if paused {
            continue;
        }

        let chunk = source
            .capture_next_available(SOURCE_POLL_TIMEOUT)
            .map_err(|error| AudioCaptureWorkerError::Source(error.to_string()))?;
        if let Some(chunk) = chunk {
            report.captured_chunks = report.captured_chunks.saturating_add(1);
            report.captured_frames = report
                .captured_frames
                .saturating_add(u64::from(chunk.frame_count));
            match pipeline.push(chunk)? {
                AudioPushOutcome::Queued { .. } => {
                    report.queued_chunks = report.queued_chunks.saturating_add(1);
                }
                AudioPushOutcome::IgnoredWhilePaused => {
                    return Err(AudioCaptureWorkerError::Pipeline(
                        AudioPipelineError::AlreadyPaused,
                    ));
                }
            }
        }

        if let Some(duration_ns) = try_handle_command(&commands, source, pipeline, &mut paused)? {
            report.duration_ns = Some(duration_ns);
            return Ok(report);
        }
    }
    Ok(report)
}

fn try_handle_command<S>(
    commands: &Receiver<ControlCommand>,
    source: &mut S,
    pipeline: &AudioPipeline,
    paused: &mut bool,
) -> Result<Option<u64>, AudioCaptureWorkerError>
where
    S: RecordingAudioSource,
{
    match commands.try_recv() {
        Ok(command) => handle_command(command, source, pipeline, paused),
        Err(TryRecvError::Empty) => Ok(None),
        Err(TryRecvError::Disconnected) => Err(AudioCaptureWorkerError::ControlDisconnected),
    }
}

fn handle_command<S>(
    command: ControlCommand,
    source: &mut S,
    pipeline: &AudioPipeline,
    paused: &mut bool,
) -> Result<Option<u64>, AudioCaptureWorkerError>
where
    S: RecordingAudioSource,
{
    match command {
        ControlCommand::Pause(reply) => {
            let timestamp = match source.pause_capture() {
                Ok(timestamp) => timestamp,
                Err(error) => {
                    let error = AudioCaptureWorkerError::Source(error.to_string());
                    let _ = reply.send(Err(error.clone()));
                    return Err(error);
                }
            };
            match pipeline
                .pause(timestamp)
                .map_err(AudioCaptureWorkerError::from)
            {
                Ok(()) => {
                    *paused = true;
                    let _ = reply.send(Ok(()));
                    Ok(None)
                }
                Err(error) => {
                    let _ = reply.send(Err(error.clone()));
                    Err(error)
                }
            }
        }
        ControlCommand::Resume(reply) => {
            let timestamp = match source.resume_capture() {
                Ok(timestamp) => timestamp,
                Err(error) => {
                    let error = AudioCaptureWorkerError::Source(error.to_string());
                    let _ = reply.send(Err(error.clone()));
                    return Err(error);
                }
            };
            match pipeline
                .resume(timestamp)
                .map_err(AudioCaptureWorkerError::from)
            {
                Ok(()) => {
                    *paused = false;
                    let _ = reply.send(Ok(()));
                    Ok(None)
                }
                Err(error) => {
                    let _ = reply.send(Err(error.clone()));
                    Err(error)
                }
            }
        }
        ControlCommand::Stop => {
            let timestamp = source
                .stop_capture()
                .map_err(|error| AudioCaptureWorkerError::Source(error.to_string()))?;
            Ok(Some(pipeline.finish(timestamp)?))
        }
    }
}

struct AudioPipelineAbortGuard<'a> {
    pipeline: &'a AudioPipeline,
    armed: bool,
}

impl<'a> AudioPipelineAbortGuard<'a> {
    fn new(pipeline: &'a AudioPipeline) -> Self {
        Self {
            pipeline,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for AudioPipelineAbortGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.pipeline.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::audio::{AudioFormat, AudioPipelineStats};
    use std::collections::VecDeque;
    use std::fmt;
    use std::rc::Rc;
    use std::sync::atomic::AtomicU64;
    use std::sync::Mutex;
    use std::time::Instant;

    const CHUNK_FRAMES: u32 = 960;
    const CHUNK_NS: u64 = 20_000_000;

    #[derive(Debug, Clone, Copy)]
    struct FixtureError;

    impl fmt::Display for FixtureError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("fixture audio source failed")
        }
    }

    impl Error for FixtureError {}

    fn chunk(sequence: u64, captured_at_ns: u64, frame_count: u32) -> CapturedAudioChunk {
        CapturedAudioChunk {
            sequence,
            captured_at_ns,
            format: AudioFormat::normalized(2),
            frame_count,
            samples: vec![0.25; frame_count as usize * 2].into_boxed_slice(),
        }
    }

    struct QueueSource {
        chunks: VecDeque<CapturedAudioChunk>,
        stop_at_ns: u64,
        fail_capture: bool,
        hooks: Option<Arc<Mutex<Vec<&'static str>>>>,
    }

    impl RecordingAudioSource for QueueSource {
        type Error = FixtureError;

        fn capture_next_available(
            &mut self,
            timeout: Duration,
        ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
            if self.fail_capture {
                return Err(FixtureError);
            }
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
            Ok(self.stop_at_ns)
        }

        fn resume_capture(&mut self) -> Result<u64, Self::Error> {
            if let Some(hooks) = &self.hooks {
                hooks.lock().unwrap().push("resume");
            }
            self.stop_at_ns += 1_000_000_000;
            Ok(self.stop_at_ns)
        }

        fn stop_capture(&mut self) -> Result<u64, Self::Error> {
            if let Some(hooks) = &self.hooks {
                hooks.lock().unwrap().push("stop");
            }
            Ok(self.stop_at_ns + 1_000_000_000)
        }
    }

    fn wait_for_chunks(pipeline: &AudioPipeline, expected: u64) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if pipeline.stats().unwrap().accepted_chunks >= expected {
                return;
            }
            assert!(Instant::now() < deadline, "音频块未在期限内进入 pipeline");
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn queues_pcm_and_finishes_with_source_clock() {
        let pipeline = Arc::new(AudioPipeline::new(0));
        let worker = AudioCaptureWorker::spawn_with_factory(
            |_| {
                Ok(QueueSource {
                    chunks: VecDeque::from([
                        chunk(0, 100_000_000, CHUNK_FRAMES),
                        chunk(1, 120_000_000, CHUNK_FRAMES),
                    ]),
                    stop_at_ns: 140_000_000,
                    fail_capture: false,
                    hooks: None,
                })
            },
            RecordingSessionClock::new(),
            Arc::clone(&pipeline),
        )
        .unwrap();
        wait_for_chunks(&pipeline, 2);

        let report = worker.stop().unwrap();
        assert_eq!(report.captured_chunks, 2);
        assert_eq!(report.captured_frames, u64::from(CHUNK_FRAMES) * 2);
        assert_eq!(report.queued_chunks, 2);
        assert_eq!(report.duration_ns, Some(1_140_000_000));
        assert_eq!(pipeline.pop().unwrap().unwrap().chunk.sequence, 0);
        assert_eq!(pipeline.pop().unwrap().unwrap().chunk.sequence, 1);
        assert!(pipeline.pop().unwrap().is_none());
    }

    #[test]
    fn pause_resume_and_stop_run_platform_hooks_without_polling_while_paused() {
        let hooks = Arc::new(Mutex::new(Vec::new()));
        let polls = Arc::new(AtomicU64::new(0));
        struct HookSource {
            hooks: Arc<Mutex<Vec<&'static str>>>,
            polls: Arc<AtomicU64>,
            next_sequence: u64,
            next_timestamp_ns: u64,
        }
        impl RecordingAudioSource for HookSource {
            type Error = FixtureError;

            fn capture_next_available(
                &mut self,
                _timeout: Duration,
            ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
                self.polls.fetch_add(1, Ordering::AcqRel);
                let chunk = chunk(self.next_sequence, self.next_timestamp_ns, CHUNK_FRAMES);
                self.next_sequence += 1;
                self.next_timestamp_ns += CHUNK_NS;
                thread::sleep(Duration::from_millis(2));
                Ok(Some(chunk))
            }

            fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
                Ok(self.next_timestamp_ns)
            }

            fn pause_capture(&mut self) -> Result<u64, Self::Error> {
                self.hooks.lock().unwrap().push("pause");
                Ok(self.next_timestamp_ns)
            }

            fn resume_capture(&mut self) -> Result<u64, Self::Error> {
                self.hooks.lock().unwrap().push("resume");
                self.next_timestamp_ns += 1_000_000_000;
                let resumed_at = self.next_timestamp_ns;
                self.next_timestamp_ns += CHUNK_NS;
                Ok(resumed_at)
            }

            fn stop_capture(&mut self) -> Result<u64, Self::Error> {
                self.hooks.lock().unwrap().push("stop");
                Ok(self.next_timestamp_ns)
            }
        }

        let pipeline = Arc::new(AudioPipeline::new(0));
        let worker_hooks = Arc::clone(&hooks);
        let worker_polls = Arc::clone(&polls);
        let worker = AudioCaptureWorker::spawn_with_factory(
            move |_| {
                Ok(HookSource {
                    hooks: worker_hooks,
                    polls: worker_polls,
                    next_sequence: 0,
                    next_timestamp_ns: 20_000_000,
                })
            },
            RecordingSessionClock::new(),
            Arc::clone(&pipeline),
        )
        .unwrap();
        wait_for_chunks(&pipeline, 2);
        worker.pause().unwrap();
        let paused_polls = polls.load(Ordering::Acquire);
        thread::sleep(Duration::from_millis(60));
        assert_eq!(polls.load(Ordering::Acquire), paused_polls);
        worker.resume().unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while polls.load(Ordering::Acquire) == paused_polls {
            assert!(Instant::now() < deadline, "恢复后没有重新读取音频");
            thread::sleep(Duration::from_millis(2));
        }
        let report = worker.stop().unwrap();
        assert!(report.duration_ns.is_some());
        assert_eq!(*hooks.lock().unwrap(), ["pause", "resume", "stop"]);
    }

    #[test]
    fn backpressure_aborts_without_discarding_queued_prefix() {
        let pipeline = Arc::new(AudioPipeline::new(0));
        let chunks = (0..11)
            .map(|sequence| chunk(sequence, sequence * 100_000_000, 4_800))
            .collect::<VecDeque<_>>();
        let worker = AudioCaptureWorker::spawn_with_factory(
            |_| {
                Ok(QueueSource {
                    chunks,
                    stop_at_ns: 1_100_000_000,
                    fail_capture: false,
                    hooks: None,
                })
            },
            RecordingSessionClock::new(),
            Arc::clone(&pipeline),
        )
        .unwrap();

        assert_eq!(
            worker.wait(),
            Err(AudioCaptureWorkerError::Pipeline(
                AudioPipelineError::Backpressure
            ))
        );
        assert_eq!(
            pipeline.stats().unwrap(),
            AudioPipelineStats {
                queued_chunks: 10,
                queued_frames: 48_000,
                queued_bytes: 48_000 * 2 * std::mem::size_of::<f32>(),
                accepted_chunks: 10,
                ignored_while_paused: 0,
            }
        );
        for sequence in 0..10 {
            assert_eq!(pipeline.pop().unwrap().unwrap().chunk.sequence, sequence);
        }
        assert_eq!(
            pipeline.push(chunk(11, 1_100_000_000, CHUNK_FRAMES)),
            Err(AudioPipelineError::Aborted)
        );
    }

    #[test]
    fn source_failure_aborts_and_keeps_the_existing_prefix() {
        let pipeline = Arc::new(AudioPipeline::new(0));
        pipeline
            .push(chunk(0, 0, CHUNK_FRAMES))
            .expect("准备已提交前缀");
        let worker = AudioCaptureWorker::spawn_with_factory(
            |_| {
                Ok(QueueSource {
                    chunks: VecDeque::new(),
                    stop_at_ns: CHUNK_NS,
                    fail_capture: true,
                    hooks: None,
                })
            },
            RecordingSessionClock::new(),
            Arc::clone(&pipeline),
        )
        .unwrap();

        assert_eq!(
            worker.wait(),
            Err(AudioCaptureWorkerError::Source(
                "fixture audio source failed".to_string()
            ))
        );
        assert_eq!(pipeline.pop().unwrap().unwrap().chunk.sequence, 0);
        assert!(pipeline.pop().unwrap().is_none());
    }

    #[test]
    fn source_control_failure_is_reported_and_aborts_pipeline() {
        struct FailingPauseSource;
        impl RecordingAudioSource for FailingPauseSource {
            type Error = FixtureError;

            fn capture_next_available(
                &mut self,
                timeout: Duration,
            ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
                thread::sleep(timeout);
                Ok(None)
            }

            fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
                Ok(0)
            }

            fn pause_capture(&mut self) -> Result<u64, Self::Error> {
                Err(FixtureError)
            }
        }

        let pipeline = Arc::new(AudioPipeline::new(0));
        let worker = AudioCaptureWorker::spawn_with_factory(
            |_| Ok(FailingPauseSource),
            RecordingSessionClock::new(),
            Arc::clone(&pipeline),
        )
        .unwrap();
        let expected = AudioCaptureWorkerError::Source("fixture audio source failed".to_string());
        assert_eq!(worker.pause(), Err(expected.clone()));
        assert_eq!(worker.wait(), Err(expected));
        assert_eq!(
            pipeline.push(chunk(0, 0, CHUNK_FRAMES)),
            Err(AudioPipelineError::Aborted)
        );
    }

    #[test]
    fn invalid_pipeline_control_aborts_instead_of_diverging_from_source() {
        let pipeline = Arc::new(AudioPipeline::new(0));
        let worker = AudioCaptureWorker::spawn_with_factory(
            |_| {
                Ok(QueueSource {
                    chunks: VecDeque::new(),
                    stop_at_ns: 0,
                    fail_capture: false,
                    hooks: None,
                })
            },
            RecordingSessionClock::new(),
            Arc::clone(&pipeline),
        )
        .unwrap();
        worker.pause().unwrap();
        let expected = AudioCaptureWorkerError::Pipeline(AudioPipelineError::AlreadyPaused);
        assert_eq!(worker.pause(), Err(expected.clone()));
        assert_eq!(worker.wait(), Err(expected));
        assert_eq!(
            pipeline.push(chunk(0, 0, CHUNK_FRAMES)),
            Err(AudioPipelineError::Aborted)
        );
    }

    #[test]
    fn factory_failure_and_panic_are_synchronous_and_abort_pipeline() {
        let failed = Arc::new(AudioPipeline::new(0));
        let failure = AudioCaptureWorker::spawn_with_factory(
            |_| Err::<QueueSource, _>("device unavailable".to_string()),
            RecordingSessionClock::new(),
            Arc::clone(&failed),
        );
        assert!(matches!(
            failure,
            Err(AudioCaptureWorkerError::SourceInitialization(message))
                if message == "device unavailable"
        ));
        assert_eq!(
            failed.push(chunk(0, 0, CHUNK_FRAMES)),
            Err(AudioPipelineError::Aborted)
        );

        let panicked = Arc::new(AudioPipeline::new(0));
        let panic = AudioCaptureWorker::spawn_with_factory(
            |_| -> Result<QueueSource, String> { panic!("fixture panic") },
            RecordingSessionClock::new(),
            Arc::clone(&panicked),
        );
        assert!(matches!(
            panic,
            Err(AudioCaptureWorkerError::SourceInitializationPanicked)
        ));
        assert_eq!(
            panicked.push(chunk(0, 0, CHUNK_FRAMES)),
            Err(AudioPipelineError::Aborted)
        );
    }

    #[test]
    fn non_send_source_is_created_and_dropped_inside_audio_thread_with_shared_clock() {
        struct ThreadBoundSource {
            _not_send: Rc<()>,
            dropped_on: SyncSender<thread::ThreadId>,
            stop_at_ns: u64,
        }
        impl RecordingAudioSource for ThreadBoundSource {
            type Error = FixtureError;

            fn capture_next_available(
                &mut self,
                timeout: Duration,
            ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
                thread::sleep(timeout);
                Ok(None)
            }

            fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
                Ok(self.stop_at_ns)
            }
        }
        impl Drop for ThreadBoundSource {
            fn drop(&mut self) {
                let _ = self.dropped_on.send(thread::current().id());
            }
        }

        let pipeline = Arc::new(AudioPipeline::new(0));
        let (created_tx, created_rx) = mpsc::sync_channel(1);
        let (dropped_tx, dropped_rx) = mpsc::sync_channel(1);
        let clock = RecordingSessionClock::new();
        thread::sleep(Duration::from_millis(5));
        let before_factory = clock.now_ns();
        let worker = AudioCaptureWorker::spawn_with_factory(
            move |clock| {
                let created_on = thread::current().id();
                created_tx.send((created_on, clock.now_ns())).unwrap();
                Ok(ThreadBoundSource {
                    _not_send: Rc::new(()),
                    dropped_on: dropped_tx,
                    stop_at_ns: 100_000_000,
                })
            },
            clock.clone(),
            pipeline,
        )
        .unwrap();
        let (created_on, observed_timestamp) = created_rx.recv().unwrap();
        assert!(observed_timestamp >= before_factory);
        assert!(observed_timestamp <= clock.now_ns());
        let report = worker.stop().unwrap();
        assert_eq!(report.duration_ns, Some(100_000_000));
        assert_eq!(dropped_rx.recv().unwrap(), created_on);
    }

    #[test]
    fn dropping_worker_requests_abort_joins_and_aborts_pipeline() {
        let pipeline = Arc::new(AudioPipeline::new(0));
        let (dropped_tx, dropped_rx) = mpsc::sync_channel(1);
        struct DropSource(SyncSender<()>);
        impl RecordingAudioSource for DropSource {
            type Error = FixtureError;

            fn capture_next_available(
                &mut self,
                timeout: Duration,
            ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
                thread::sleep(timeout);
                Ok(None)
            }

            fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
                Ok(0)
            }
        }
        impl Drop for DropSource {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let worker = AudioCaptureWorker::spawn_with_factory(
            |_| Ok(DropSource(dropped_tx)),
            RecordingSessionClock::new(),
            Arc::clone(&pipeline),
        )
        .unwrap();
        drop(worker);
        dropped_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(
            pipeline.push(chunk(0, 0, CHUNK_FRAMES)),
            Err(AudioPipelineError::Aborted)
        );
    }
}
