use super::*;
use std::sync::atomic::AtomicU64;

struct FrontierSource {
    inner: FakeSource,
    clock: RecordingSessionClockForTest,
    polls: Arc<AtomicU64>,
    fail: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
    frames: mpsc::Sender<Instant>,
    first_only: bool,
}

type RecordingSessionClockForTest = crate::recording::clock::RecordingSessionClock;

impl RecordingFrameSource for FrontierSource {
    type Error = FakeSourceError;
    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
        self.inner.capture_next()
    }
    fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
        if self.first_only && self.inner.next_sequence > 0 {
            return Ok(None);
        }
        let mut value = self.inner.capture_next()?;
        value.captured_at_ns = self.clock.now_ns();
        self.frames.send(Instant::now()).unwrap();
        Ok(Some(value))
    }
    fn capture_lower_bound_ns(&mut self) -> Result<Option<u64>, Self::Error> {
        self.polls.fetch_add(1, Ordering::AcqRel);
        if self.fail.load(Ordering::Acquire) {
            return Err(FakeSourceError);
        }
        Ok(Some(self.clock.now_ns()))
    }
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(self.clock.now_ns())
    }
}

impl Drop for FrontierSource {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::Release);
    }
}

fn frontier_source(first_only: bool) -> (FrontierSource, mpsc::Receiver<Instant>) {
    let (frames, received) = mpsc::channel();
    (
        FrontierSource {
            inner: source(),
            clock: RecordingSessionClockForTest::new(),
            polls: Arc::new(AtomicU64::new(0)),
            fail: Arc::new(AtomicBool::new(false)),
            dropped: Arc::new(AtomicBool::new(false)),
            frames,
            first_only,
        },
        received,
    )
}

fn wait_polls(polls: &AtomicU64, minimum: u64) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while polls.load(Ordering::Acquire) < minimum {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn one_fps_transfers_metadata_before_next_native_frame_without_raising_fps() {
    let (source, frames) = frontier_source(false);
    let polls = Arc::clone(&source.polls);
    let pipeline = Arc::new(RecordingPipeline::default());
    let worker = CaptureWorker::spawn(source, Arc::clone(&pipeline), 1).unwrap();
    let first = frames.recv_timeout(Duration::from_secs(5)).unwrap();
    wait_polls(&polls, 4);
    assert!(frames.try_recv().is_err(), "轮询下界不能再采真实帧");
    let second = frames.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(second.duration_since(first) >= Duration::from_millis(950));
    let report = worker.stop().unwrap();
    assert_eq!(report.captured_frames, 2);
    assert_eq!(pipeline.stats().unwrap().accepted_frames, 2);
}

#[test]
fn pause_stops_metadata_and_resume_reuses_source_clock() {
    let (source, frames) = frontier_source(true);
    let polls = Arc::clone(&source.polls);
    let worker = CaptureWorker::spawn(source, Arc::new(RecordingPipeline::default()), 10).unwrap();
    frames.recv_timeout(Duration::from_secs(5)).unwrap();
    wait_polls(&polls, 2);
    worker.pause().unwrap();
    let paused_polls = polls.load(Ordering::Acquire);
    thread::sleep(Duration::from_millis(150));
    assert_eq!(polls.load(Ordering::Acquire), paused_polls);
    worker.resume().unwrap();
    wait_polls(&polls, paused_polls + 2);
    assert_eq!(worker.stop().unwrap().captured_frames, 1);
}

#[test]
fn metadata_failure_returns_original_source_error_and_joins_source() {
    let (source, frames) = frontier_source(true);
    let fail = Arc::clone(&source.fail);
    let dropped = Arc::clone(&source.dropped);
    let pipeline = Arc::new(RecordingPipeline::default());
    let worker = CaptureWorker::spawn(source, Arc::clone(&pipeline), 1).unwrap();
    frames.recv_timeout(Duration::from_secs(5)).unwrap();
    fail.store(true, Ordering::Release);
    assert!(matches!(worker.wait(), Err(CaptureWorkerError::Source(_))));
    assert!(dropped.load(Ordering::Acquire));
    pipeline.pop().unwrap();
    assert!(matches!(pipeline.pop_wait(), Err(PipelineError::Aborted)));
}

#[test]
fn drop_while_metadata_waiting_cancels_and_joins() {
    let (source, frames) = frontier_source(true);
    let dropped = Arc::clone(&source.dropped);
    let polls = Arc::clone(&source.polls);
    let pipeline = Arc::new(RecordingPipeline::default());
    let worker = CaptureWorker::spawn(source, Arc::clone(&pipeline), 1).unwrap();
    frames.recv_timeout(Duration::from_secs(5)).unwrap();
    wait_polls(&polls, 2);
    drop(worker);
    assert!(dropped.load(Ordering::Acquire));
    assert!(!pipeline.is_open().unwrap());
    pipeline.pop().unwrap();
    // 原 Stop 与取消标志可能由同一等待点收到；两种终态都必须回收源和线程。
    assert!(matches!(
        pipeline.pop_wait(),
        Err(PipelineError::Aborted) | Ok(PipelineDrain::Finished { .. })
    ));
}
