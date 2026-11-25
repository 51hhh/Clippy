struct ActivationSource {
    inner: QueueSource,
    _affinity: Rc<()>,
    activated: mpsc::Sender<std::thread::ThreadId>,
    dropped: mpsc::Sender<std::thread::ThreadId>,
    failed: bool,
}

impl RecordingAudioSource for ActivationSource {
    type Error = FixtureError;

    fn start_capture(&mut self) -> Result<Option<u64>, Self::Error> {
        let _ = self.activated.send(thread::current().id());
        if self.failed {
            Err(FixtureError)
        } else {
            Ok(None)
        }
    }

    fn capture_next_available(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        self.inner.capture_next_available(timeout)
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        self.inner.control_timestamp_ns()
    }
}

impl Drop for ActivationSource {
    fn drop(&mut self) {
        let _ = self.dropped.send(thread::current().id());
    }
}

fn assert_activation(gated: bool, release: bool, failed: bool) {
    let pipeline = Arc::new(AudioPipeline::new(0));
    let (created, creation) = mpsc::channel();
    let (activated, activation) = mpsc::channel();
    let (dropped, destruction) = mpsc::channel();
    let (start, signal) = mpsc::sync_channel(1);
    let factory = move |_| {
        created.send(thread::current().id()).unwrap();
        Ok(ActivationSource {
            inner: QueueSource {
                chunks: VecDeque::new(),
                stop_at_ns: 500_000_000,
                fail_capture: false,
                hooks: None,
            },
            _affinity: Rc::new(()),
            activated,
            dropped,
            failed,
        })
    };
    let worker = if gated {
        AudioCaptureWorker::spawn_with_factory_gated(
            factory,
            RecordingSessionClock::new(),
            Arc::clone(&pipeline),
            signal,
        )
    } else {
        AudioCaptureWorker::spawn_with_factory(
            factory,
            RecordingSessionClock::new(),
            Arc::clone(&pipeline),
        )
    }
    .unwrap();
    let source_thread = creation.recv_timeout(Duration::from_secs(5)).unwrap();
    if gated {
        assert!(activation.recv_timeout(Duration::from_millis(100)).is_err());
        if release {
            start.send(()).unwrap();
        }
    }
    if !gated || release {
        assert_eq!(
            activation.recv_timeout(Duration::from_secs(5)).unwrap(),
            source_thread
        );
    }
    if failed {
        assert!(matches!(
            worker.wait(),
            Err(AudioCaptureWorkerError::Source(_))
        ));
        assert!(matches!(
            pipeline.pop_wait(),
            Err(AudioPipelineError::Aborted)
        ));
    } else if gated && !release {
        drop(worker);
        assert!(activation.recv_timeout(Duration::from_secs(5)).is_err());
        assert!(matches!(
            pipeline.pop_wait(),
            Err(AudioPipelineError::Aborted)
        ));
    } else {
        // 激活信号发出后仍可能尚未清除 pending；等待 worker 进入控制循环再正常 Stop。
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while worker.is_start_pending() {
            assert!(std::time::Instant::now() < deadline);
            thread::yield_now();
        }
        assert!(worker.stop().unwrap().duration_ns.is_some());
    }
    assert_eq!(
        destruction.recv_timeout(Duration::from_secs(5)).unwrap(),
        source_thread
    );
    drop(start);
}

#[test]
fn gated_audio_activates_only_after_release_on_source_thread() {
    assert_activation(true, true, false);
}

#[test]
fn cancelled_audio_never_activates_native_source() {
    assert_activation(true, false, false);
}

#[test]
fn activation_failure_aborts_pipeline_and_reclaims_source() {
    assert_activation(true, true, true);
}

#[test]
fn single_track_audio_activates_immediately() {
    assert_activation(false, true, false);
}
