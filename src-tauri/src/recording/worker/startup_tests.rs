use std::sync::atomic::AtomicUsize;
use std::thread::ThreadId;

struct StartupSource {
    inner: FakeSource,
    _affinity: Rc<()>,
    reads: Arc<AtomicUsize>,
    first_read: Option<mpsc::Sender<()>>,
    dropped: mpsc::Sender<ThreadId>,
}

impl RecordingFrameSource for StartupSource {
    type Error = FakeSourceError;

    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if let Some(first_read) = self.first_read.take() {
            let _ = first_read.send(());
        }
        self.inner.capture_next()
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        self.inner.control_timestamp_ns()
    }
}

impl Drop for StartupSource {
    fn drop(&mut self) {
        let _ = self.dropped.send(thread::current().id());
    }
}

#[derive(Clone, Copy)]
enum Settlement {
    Release,
    Disconnect,
    Drop,
    Stop,
}

fn assert_gated_worker_settlement(settlement: Settlement) {
    let pipeline = Arc::new(RecordingPipeline::default());
    let (start, release) = mpsc::sync_channel(1);
    let (created, creation) = mpsc::channel();
    let (dropped, destruction) = mpsc::channel();
    let (first_read, first_poll) = mpsc::channel();
    let reads = Arc::new(AtomicUsize::new(0));
    let source_reads = Arc::clone(&reads);
    let worker = CaptureWorker::spawn_with_factory_gated(
        move || {
            created.send(thread::current().id()).unwrap();
            Ok(StartupSource {
                inner: source(),
                _affinity: Rc::new(()),
                reads: source_reads,
                first_read: Some(first_read),
                dropped,
            })
        },
        Arc::clone(&pipeline),
        120,
        release,
    )
    .unwrap();
    let created_thread = creation.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(first_poll.recv_timeout(Duration::from_millis(100)).is_err());
    assert_eq!(pipeline.stats().unwrap().accepted_frames, 0);
    if matches!(settlement, Settlement::Release) {
        start.send(()).unwrap();
        first_poll.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    let start = if matches!(settlement, Settlement::Disconnect) {
        drop(start);
        None
    } else {
        Some(start)
    };
    // 保留 sender，确保 Drop/Stop 的取消来自 worker，而不是测试提前关闭释放通道。
    let finisher = thread::spawn(move || match settlement {
        Settlement::Release | Settlement::Stop => Some(worker.stop()),
        Settlement::Disconnect => Some(worker.wait()),
        Settlement::Drop => {
            drop(worker);
            None
        }
    });
    assert_eq!(
        destruction.recv_timeout(Duration::from_secs(5)).unwrap(),
        created_thread
    );
    let result = finisher.join().unwrap();
    if matches!(settlement, Settlement::Release) {
        assert!(result.unwrap().unwrap().duration_ns.is_some());
        assert!(reads.load(Ordering::SeqCst) > 0);
    } else {
        if let Some(result) = result {
            assert!(matches!(result, Err(CaptureWorkerError::ControlDisconnected)));
        }
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        assert_eq!(pipeline.stats().unwrap().accepted_frames, 0);
        assert!(matches!(pipeline.pop_wait(), Err(PipelineError::Aborted)));
    }
    drop(start);
}

#[test]
fn gated_video_starts_only_after_owner_release() {
    assert_gated_worker_settlement(Settlement::Release);
}

#[test]
fn gated_video_disconnect_reclaims_unpolled_source() {
    assert_gated_worker_settlement(Settlement::Disconnect);
}

#[test]
fn gated_video_drop_cancels_wait_with_sender_alive() {
    assert_gated_worker_settlement(Settlement::Drop);
}

#[test]
fn gated_video_stop_cancels_wait_with_sender_alive() {
    assert_gated_worker_settlement(Settlement::Stop);
}
