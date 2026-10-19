struct FirstFrameSource {
    inner: FakeSource,
    released: Arc<AtomicBool>,
    invalid: bool,
    failed: bool,
    _affinity: Rc<()>,
}

impl RecordingFrameSource for FirstFrameSource {
    type Error = FakeSourceError;

    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
        self.inner.capture_next()
    }

    fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
        if !self.released.load(Ordering::Acquire) {
            return Ok(None);
        }
        if self.failed {
            return Err(FakeSourceError);
        }
        let mut frame = self.inner.capture_next()?;
        if self.invalid {
            frame.width = 0;
        }
        Ok(Some(frame))
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        self.inner.control_timestamp_ns()
    }
}

fn assert_first_frame_release(delayed: bool, invalid: bool, failed: bool) {
    let pipeline = Arc::new(RecordingPipeline::default());
    let released = Arc::new(AtomicBool::new(!delayed));
    let source_release = Arc::clone(&released);
    let (start, signal) = mpsc::sync_channel(1);
    let (audio_start, audio_signal) = mpsc::sync_channel(1);
    let (polled, first_poll) = mpsc::sync_channel(1);
    let worker = CaptureWorker::spawn_with_first_frame_release(
        move || {
            Ok(FirstFrameSource {
                inner: source(),
                released: source_release,
                invalid,
                failed,
                _affinity: Rc::new(()),
            })
        },
        Arc::clone(&pipeline),
        120,
        signal,
        audio_start,
        polled,
    )
    .unwrap();
    start.send(()).unwrap();
    if invalid || failed {
        assert!(first_poll.recv_timeout(Duration::from_secs(5)).is_err());
        assert!(audio_signal.recv_timeout(Duration::from_secs(5)).is_err());
        assert!(!worker.is_first_frame_ready());
        let result = worker.wait();
        if failed {
            assert!(matches!(result, Err(CaptureWorkerError::Source(_))));
        } else {
            assert!(matches!(result, Err(CaptureWorkerError::Pipeline(_))));
        }
        assert_eq!(pipeline.stats().unwrap().accepted_frames, 0);
        assert!(matches!(pipeline.pop_wait(), Err(PipelineError::Aborted)));
    } else {
        first_poll.recv_timeout(Duration::from_secs(5)).unwrap();
        if delayed {
            assert!(!worker.is_first_frame_ready());
            assert!(audio_signal
                .recv_timeout(Duration::from_millis(100))
                .is_err());
            assert_eq!(pipeline.stats().unwrap().accepted_frames, 0);
            released.store(true, Ordering::Release);
        }
        audio_signal.recv_timeout(Duration::from_secs(5)).unwrap();
        // 信号发送与 accepted 发布可能是相邻的两条指令；按 worker 合同读取最终发布结果。
        let deadline = Instant::now() + Duration::from_secs(5);
        while !worker.is_first_frame_ready() {
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
        assert!(pipeline.stats().unwrap().accepted_frames > 0);
        assert!(worker.stop().unwrap().duration_ns.is_some());
        assert!(
            audio_signal.recv_timeout(Duration::from_secs(5)).is_err(),
            "只释放一次"
        );
    }
}

#[test]
fn first_poll_none_does_not_release_audio() {
    assert_first_frame_release(true, false, false);
}

#[test]
fn accepted_first_frame_releases_audio_once() {
    assert_first_frame_release(false, false, false);
}

#[test]
fn invalid_first_frame_cancels_without_releasing_audio() {
    assert_first_frame_release(false, true, false);
}

#[test]
fn first_poll_source_error_cancels_without_releasing_audio() {
    assert_first_frame_release(false, false, true);
}
