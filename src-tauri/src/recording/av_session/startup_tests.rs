use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::ThreadId;

struct StartupAudioSource {
    inner: SessionAudioSource,
    reads: Arc<AtomicUsize>,
    first_read: Option<mpsc::Sender<()>>,
    dropped: mpsc::Sender<(ThreadId, usize)>,
}

impl RecordingAudioSource for StartupAudioSource {
    type Error = FixtureError;

    fn capture_next_available(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if let Some(first_read) = self.first_read.take() {
            let _ = first_read.send(());
        }
        self.inner.capture_next_available(timeout)
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        self.inner.control_timestamp_ns()
    }
}

impl Drop for StartupAudioSource {
    fn drop(&mut self) {
        let _ = self
            .dropped
            .send((thread::current().id(), self.reads.load(Ordering::SeqCst)));
    }
}

#[derive(Clone, Copy)]
enum VideoInitialization {
    Success,
    Failure,
    Panic,
}

fn assert_audio_is_not_polled_during_video_initialization(outcome: VideoInitialization) {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().to_path_buf();
    let reads = Arc::new(AtomicUsize::new(0));
    let audio_reads = Arc::clone(&reads);
    let (first_read, read_started) = mpsc::channel();
    let (dropped, audio_dropped) = mpsc::channel();
    let (audio_created, creation) = mpsc::channel();
    let (video_entered, initialization) = mpsc::channel();
    let (release_video, video_release) = mpsc::channel();
    let owner = thread::spawn(move || {
        AvRecordingSession::start_with_factories(
            &directory,
            config("av-startup-gate"),
            move |_| {
                video_entered.send(()).unwrap();
                video_release
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|_| "video fixture was not released".to_string())?;
                match outcome {
                    VideoInitialization::Success => Ok(video_source(None, None)),
                    VideoInitialization::Failure => Err("original video initialization error".into()),
                    VideoInitialization::Panic => panic!("controlled video factory panic"),
                }
            },
            move |_| {
                audio_created.send(thread::current().id()).unwrap();
                Ok(StartupAudioSource {
                    inner: audio_source(None, None),
                    reads: audio_reads,
                    first_read: Some(first_read),
                    dropped,
                })
            },
        )
    });
    let audio_thread = creation.recv_timeout(Duration::from_secs(5)).unwrap();
    initialization.recv_timeout(Duration::from_secs(5)).unwrap();
    // 延迟只用于确认受控 factory 等待期间有无采集，不是 WGC 初始化或设备响应时间承诺。
    let early_read = read_started.recv_timeout(Duration::from_millis(200));
    let reads_before_release = reads.load(Ordering::SeqCst);
    release_video.send(()).unwrap();
    let result = owner.join().unwrap();
    match outcome {
        VideoInitialization::Success => {
            let session = result.unwrap();
            if early_read.is_err() {
                read_started.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            drop(session);
        }
        VideoInitialization::Failure => assert!(matches!(
            result,
            Err(AvRecordingSessionError::VideoCapture(CaptureWorkerError::SourceInitialization(ref error)))
                if error == "original video initialization error"
        )),
        VideoInitialization::Panic => assert!(matches!(
            result,
            Err(AvRecordingSessionError::VideoCapture(CaptureWorkerError::SourceInitializationPanicked))
        )),
    }
    let (drop_thread, final_reads) = audio_dropped.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(drop_thread, audio_thread, "source 必须留在创建线程回收");
    assert_eq!(reads_before_release, 0, "视频 factory 未就绪时不能轮询音频");
    assert!(early_read.is_err(), "音频只能在两个 factory 就绪后开始采集");
    if !matches!(outcome, VideoInitialization::Success) {
        assert_eq!(final_reads, 0, "启动失败不能通过采集提交音频前缀");
        assert!(!temporary.path().join("recordings/av-startup-gate").exists());
    }
}

#[test]
fn audio_capture_waits_until_video_factory_is_ready() {
    assert_audio_is_not_polled_during_video_initialization(VideoInitialization::Success);
}

#[test]
fn failed_video_initialization_does_not_poll_audio() {
    assert_audio_is_not_polled_during_video_initialization(VideoInitialization::Failure);
}

#[test]
fn panicking_video_initialization_does_not_poll_audio() {
    assert_audio_is_not_polled_during_video_initialization(VideoInitialization::Panic);
}
