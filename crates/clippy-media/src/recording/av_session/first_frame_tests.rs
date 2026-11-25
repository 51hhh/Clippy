use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::ThreadId;

struct DelayedVideo {
    released: Arc<AtomicBool>,
    entered: Option<mpsc::Sender<()>>,
    inner: SessionVideoSource,
    failure: Option<&'static str>,
}

impl RecordingFrameSource for DelayedVideo {
    type Error = FixtureError;

    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
        self.inner.capture_next()
    }

    fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
        if let Some(entered) = self.entered.take() {
            let _ = entered.send(());
        }
        if !self.released.load(Ordering::Acquire) {
            return Ok(None);
        }
        if let Some(error) = self.failure.take() {
            return Err(FixtureError(error));
        }
        self.inner.capture_next_available()
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        self.inner.control_timestamp_ns()
    }
}

struct PacedAudio {
    reads: Arc<AtomicUsize>,
    next: u64,
    finished: Option<mpsc::Sender<()>>,
    dropped: mpsc::Sender<(ThreadId, usize)>,
}

impl RecordingAudioSource for PacedAudio {
    type Error = FixtureError;

    fn capture_next_available(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.next < 20 {
            // 受控加速交付 100 ms PCM；严格序号/PTS，真实队列仍只有一秒，非设备时延测量。
            thread::sleep(Duration::from_millis(10));
            let result = chunk(self.next, 2_000_000_000 + self.next * 100_000_000);
            self.next += 1;
            return Ok(Some(result));
        }
        if let Some(finished) = self.finished.take() {
            let _ = finished.send(());
        }
        thread::sleep(timeout);
        Ok(None)
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(4_000_000_000)
    }
}

impl Drop for PacedAudio {
    fn drop(&mut self) {
        let _ = self
            .dropped
            .send((thread::current().id(), self.reads.load(Ordering::SeqCst)));
    }
}

type DelayedFixture = (
    AvRecordingSession,
    Arc<AtomicBool>,
    Arc<AtomicUsize>,
    mpsc::Receiver<()>,
    mpsc::Receiver<(ThreadId, usize)>,
    ThreadId,
    mpsc::Receiver<()>,
);

fn setup_delayed(directory: &Path, id: &str, failure: Option<&'static str>) -> DelayedFixture {
    let released = Arc::new(AtomicBool::new(false));
    let video_release = Arc::clone(&released);
    let reads = Arc::new(AtomicUsize::new(0));
    let audio_reads = Arc::clone(&reads);
    let (entered, entry) = mpsc::channel();
    let (finished, audio_finished) = mpsc::channel();
    let (dropped, audio_dropped) = mpsc::channel();
    let (created, creation) = mpsc::channel();
    let (video_progress, video_events) = mpsc::channel();
    let mut settings = config(id);
    settings.frames_per_second = 100;
    let session = AvRecordingSession::start_with_factories(
        directory,
        settings,
        move |_| {
            Ok(DelayedVideo {
                released: video_release,
                entered: Some(entered),
                inner: SessionVideoSource {
                    frames: (0..20)
                        .map(|sequence| frame(sequence, 2_000_000_000 + sequence * 100_000_000))
                        .collect(),
                    stop_at_ns: 4_000_000_000,
                    hooks: None,
                    progress: Some(video_progress),
                },
                failure,
            })
        },
        move |_| {
            let _ = created.send(thread::current().id());
            Ok(PacedAudio {
                reads: audio_reads,
                next: 0,
                finished: Some(finished),
                dropped,
            })
        },
    )
    .unwrap();
    entry.recv_timeout(Duration::from_secs(5)).unwrap();
    let audio_thread = creation.recv_timeout(Duration::from_secs(5)).unwrap();
    (
        session,
        released,
        reads,
        audio_finished,
        audio_dropped,
        audio_thread,
        video_events,
    )
}

#[test]
fn first_frame_wait_does_not_fill_audio_queue_and_can_complete() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut session, released, reads, finished, dropped, created, video_events) =
        setup_delayed(temporary.path(), "first-frame-pressure", None);
    thread::sleep(Duration::from_millis(300));
    let early_reads = reads.load(Ordering::SeqCst);
    let prematurely_terminated = session.has_terminated_worker();
    // 旧实现退出后直接读取 audio worker 的原错误，避免随后视频 Stop 的无首帧错误遮蔽背压。
    let early_audio_result = if prematurely_terminated {
        Some(session.audio_capture.take().unwrap().wait())
    } else {
        None
    };
    released.store(true, Ordering::Release);
    let finished_result = finished.recv_timeout(Duration::from_secs(5));
    if !prematurely_terminated {
        for _ in 0..20 {
            video_events.recv_timeout(Duration::from_secs(5)).unwrap();
        }
    }
    let result = session.stop();
    let (drop_thread, _) = dropped.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(drop_thread, created);
    assert_eq!(early_reads, 0, "首帧未到不应轮询 PCM；提前结束={prematurely_terminated}, audio={early_audio_result:?}, stop={result:?}");
    assert!(!prematurely_terminated);
    finished_result.unwrap();
    let report = result.unwrap();
    assert_eq!(report.video_captured_frames, 20);
    assert_eq!(report.audio_accepted_chunks, 20);
    assert_eq!(report.audio_encoder_input_chunks, 20);
    assert_eq!(report.audio_dropped_before_video_frames, 0);
    let manifest: Value = serde_json::from_slice(
        &fs::read(
            temporary
                .path()
                .join("recordings/first-frame-pressure/manifest.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["state"], "complete");
}

#[test]
fn first_frame_source_error_cancels_audio_without_polling() {
    let temporary = tempfile::tempdir().unwrap();
    let (session, released, reads, _, dropped, created, _) = setup_delayed(
        temporary.path(),
        "first-frame-error",
        Some("original first-frame failure"),
    );
    thread::sleep(Duration::from_millis(60));
    let early_reads = reads.load(Ordering::SeqCst);
    released.store(true, Ordering::Release);
    let (drop_thread, final_reads) = dropped.recv_timeout(Duration::from_secs(5)).unwrap();
    let result = session.stop();
    assert_eq!(drop_thread, created);
    assert_eq!(early_reads, 0);
    assert_eq!(final_reads, 0);
    assert!(
        matches!(result, Err(AvRecordingSessionError::VideoCapture(CaptureWorkerError::Source(ref error))) if error == "original first-frame failure")
    );
}

#[test]
fn first_frame_wait_drop_cancels_audio_without_polling() {
    let temporary = tempfile::tempdir().unwrap();
    let (session, _, reads, _, dropped, created, _) =
        setup_delayed(temporary.path(), "first-frame-drop", None);
    thread::sleep(Duration::from_millis(60));
    let early_reads = reads.load(Ordering::SeqCst);
    drop(session);
    let (drop_thread, final_reads) = dropped.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(drop_thread, created);
    assert_eq!(early_reads, 0);
    assert_eq!(final_reads, 0);
}
