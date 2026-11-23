use crate::recording::segmenting::RecordingEncoder;
use crate::recording::session::{
    DiagnosticRecordingConfig, DiagnosticRecordingError, DiagnosticRecordingSession,
};
use crate::recording::timeline::TimelineError;
use sha2::{Digest, Sha256};
use std::sync::atomic::AtomicUsize;

#[derive(Clone, Copy)]
enum Fault {
    PauseTimestamp,
    ResumeTimestamp,
    None,
}

#[derive(Default)]
struct State {
    ready: AtomicBool,
    running: AtomicBool,
    hooks: Mutex<Vec<&'static str>>,
    drops: AtomicUsize,
}

impl State {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            running: AtomicBool::new(true),
            ..Self::default()
        })
    }
}

struct FaultSource {
    state: Arc<State>,
    emitted: bool,
    fault: Fault,
}

impl RecordingFrameSource for FaultSource {
    type Error = FakeSourceError;

    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
        unreachable!("合成推送源只使用短轮询入口")
    }

    fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
        if self.emitted || !self.state.running.load(Ordering::Acquire) {
            return Ok(None);
        }
        self.emitted = true;
        Ok(Some(CapturedFrame {
            sequence: 0,
            captured_at_ns: 100_000_000,
            width: 64,
            height: 48,
            stride: 256,
            rgba: vec![0x80; 64 * 48 * 4].into_boxed_slice(),
        }))
    }

    fn capture_lower_bound_ns(&mut self) -> Result<Option<u64>, Self::Error> {
        // 原 worker 在 pipeline 接受首帧后才调用这里，不用“source 已返回”代替入队证据。
        if self.emitted {
            self.state.ready.store(true, Ordering::Release);
        }
        Ok(None)
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(400_000_000)
    }

    fn pause_capture(&mut self) -> Result<u64, Self::Error> {
        self.state.hooks.lock().unwrap().push("pause");
        self.state.running.store(false, Ordering::Release);
        Ok(if matches!(self.fault, Fault::PauseTimestamp) {
            99_000_000
        } else {
            200_000_000
        })
    }

    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        self.state.hooks.lock().unwrap().push("resume");
        self.state.running.store(true, Ordering::Release);
        Ok(if matches!(self.fault, Fault::ResumeTimestamp) {
            200_000_000
        } else {
            300_000_000
        })
    }

    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        self.state.hooks.lock().unwrap().push("stop");
        self.state.running.store(false, Ordering::Release);
        self.control_timestamp_ns()
    }
}

impl Drop for FaultSource {
    fn drop(&mut self) {
        self.state.drops.fetch_add(1, Ordering::AcqRel);
    }
}

fn observe(mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(1);
    while !condition() {
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(2));
    }
    true
}

fn timeline_error() -> CaptureWorkerError {
    CaptureWorkerError::Pipeline(PipelineError::Timeline(
        TimelineError::SourceTimestampNotIncreasing,
    ))
}

fn assert_worker_failure(fault: Fault, resume: bool, sink_error: Option<PipelineError>) {
    let state = State::new();
    let pipeline = Arc::new(RecordingPipeline::default());
    let worker = CaptureWorker::spawn(
        FaultSource {
            state: Arc::clone(&state),
            emitted: false,
            fault,
        },
        Arc::clone(&pipeline),
        120,
    )
    .unwrap();
    assert!(observe(|| state.ready.load(Ordering::Acquire)));
    if resume {
        worker.pause().unwrap();
    }
    match &sink_error {
        Some(PipelineError::Aborted) => pipeline.abort().unwrap(),
        Some(PipelineError::Closed) => {
            pipeline.finish(400_000_000).unwrap();
        }
        None => {}
        _ => unreachable!("夹具仅注入关闭或中止"),
    }
    let control = if resume {
        worker.resume()
    } else {
        worker.pause()
    };
    let hooks = state.hooks.lock().unwrap().clone();
    let source_running = state.running.load(Ordering::Acquire);
    let terminated = observe(|| worker.is_finished());
    // 原实现会一直等待；先主动 Drop/join 再失败，不能把不通过的对照留成挂住的线程。
    let joined = if terminated {
        Some(worker.wait())
    } else {
        drop(worker);
        None
    };
    let expected = sink_error
        .clone()
        .map(CaptureWorkerError::Pipeline)
        .unwrap_or_else(timeline_error);
    eprintln!("video control failure: resume={resume}, sink={sink_error:?}, hooks={hooks:?}, source_running={source_running}, terminated={terminated}, control={control:?}, join={joined:?}");
    assert_eq!(control, Err(expected.clone()));
    assert_eq!(
        hooks,
        if resume {
            vec!["pause", "resume"]
        } else {
            vec!["pause"]
        }
    );
    assert_eq!(source_running, resume);
    assert!(
        terminated,
        "源已改变而 worker 仍继续，根错误不能留到后续 Stop"
    );
    assert_eq!(joined.unwrap(), Err(expected));
    assert_eq!(state.drops.load(Ordering::Acquire), 1);
    assert!(!pipeline.is_open().unwrap());
    assert_eq!(pipeline.pop().unwrap().unwrap().frame.sequence, 0);
    assert!(pipeline.pop().unwrap().is_none());
}

#[test]
fn video_control_failure_pause_bad_timestamp_keeps_root_and_exits() {
    assert_worker_failure(Fault::PauseTimestamp, false, None);
}

#[test]
fn video_control_failure_resume_bad_timestamp_keeps_root_and_exits() {
    assert_worker_failure(Fault::ResumeTimestamp, true, None);
}

#[test]
fn video_control_failure_pause_aborted_sink_exits() {
    assert_worker_failure(Fault::None, false, Some(PipelineError::Aborted));
}

#[test]
fn video_control_failure_resume_aborted_sink_exits() {
    assert_worker_failure(Fault::None, true, Some(PipelineError::Aborted));
}

#[test]
fn video_control_failure_pause_closed_sink_exits() {
    assert_worker_failure(Fault::None, false, Some(PipelineError::Closed));
}

#[test]
fn video_control_failure_resume_closed_sink_exits() {
    assert_worker_failure(Fault::None, true, Some(PipelineError::Closed));
}

fn assert_interrupted(directory: &std::path::Path) {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["state"], "interrupted");
    assert!(manifest["segments"].as_array().unwrap().is_empty());
    for entry in std::fs::read_dir(directory).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(!name.ends_with(".partial"), "未提交产物残留: {name}");
        assert!(
            !name.ends_with(".avi") && !name.ends_with(".webm"),
            "失败会话被提交: {name}"
        );
    }
}

fn retain_session(directory: &std::path::Path) {
    if let Some(evidence) = std::env::var_os("CLIPPY_CONTROL_FAILURE_EVIDENCE") {
        let evidence = std::path::PathBuf::from(evidence).join(directory.file_name().unwrap());
        std::fs::create_dir_all(&evidence).unwrap();
        let mut files = Vec::new();
        // 两个 owner 已同步 stop/join；保存断言前的实际清单和媒体，避免 TempDir 清理丢证据。
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                let bytes = std::fs::read(entry.path()).unwrap();
                std::fs::write(evidence.join(entry.file_name()), &bytes).unwrap();
                files.push(serde_json::json!({"file": entry.file_name().to_string_lossy(), "bytes": bytes.len(), "sha256": format!("{:x}", Sha256::digest(&bytes))}));
            }
        }
        std::fs::write(
            evidence.join("FILES.json"),
            serde_json::to_vec_pretty(&files).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn video_control_failure_mjpeg_owner_does_not_commit_after_rejection() {
    let temporary = tempfile::tempdir().unwrap();
    let state = State::new();
    let session = DiagnosticRecordingSession::start(
        temporary.path(),
        DiagnosticRecordingConfig {
            session_id: "control-failure-video".to_string(),
            source_id: "synthetic-control".to_string(),
            physical_x: 0,
            physical_y: 0,
            width: 64,
            height: 48,
            frames_per_second: 10,
            include_cursor: false,
            encoder: RecordingEncoder::MjpegDiagnostic { jpeg_quality: 80 },
            segment_duration_ns: 1_000_000_000,
        },
        FaultSource {
            state: Arc::clone(&state),
            emitted: false,
            fault: Fault::PauseTimestamp,
        },
    )
    .unwrap();
    assert!(observe(|| state.ready.load(Ordering::Acquire)));
    let directory = session.session_directory().to_path_buf();
    let control = session.pause();
    let terminated = observe(|| session.has_terminated_worker());
    let stopped = session.stop();
    retain_session(&directory);
    eprintln!("video owner control failure: terminated={terminated}, control={control:?}, stop={stopped:?}");
    assert!(
        matches!(control, Err(DiagnosticRecordingError::Capture(error)) if error == timeline_error())
    );
    assert!(
        matches!(stopped, Err(DiagnosticRecordingError::Capture(error)) if error == timeline_error())
    );
    assert!(terminated);
    assert_eq!(state.drops.load(Ordering::Acquire), 1);
    assert_interrupted(&directory);
}

#[cfg(feature = "recording-opus-webm")]
#[test]
fn video_control_failure_av_owner_reclaims_both_sources_and_keeps_root() {
    use crate::recording::audio::{AudioFormat, CapturedAudioChunk};
    use crate::recording::audio_worker::RecordingAudioSource;
    use crate::recording::av_session::{
        AvRecordingConfig, AvRecordingSession, AvRecordingSessionError,
    };
    struct AudioSource {
        state: Arc<State>,
        emitted: bool,
    }
    impl RecordingAudioSource for AudioSource {
        type Error = FakeSourceError;
        fn capture_next_available(
            &mut self,
            timeout: Duration,
        ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
            if self.emitted {
                thread::sleep(timeout);
                return Ok(None);
            }
            self.emitted = true;
            self.state.ready.store(true, Ordering::Release);
            Ok(Some(CapturedAudioChunk {
                sequence: 0,
                captured_at_ns: 100_000_000,
                format: AudioFormat::normalized(2),
                frame_count: 960,
                samples: vec![0.0; 1920].into_boxed_slice(),
            }))
        }
        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            Ok(400_000_000)
        }
        fn pause_capture(&mut self) -> Result<u64, Self::Error> {
            self.state.hooks.lock().unwrap().push("pause");
            Ok(200_000_000)
        }
        fn resume_capture(&mut self) -> Result<u64, Self::Error> {
            self.state.hooks.lock().unwrap().push("resume");
            Ok(300_000_000)
        }
    }
    impl Drop for AudioSource {
        fn drop(&mut self) {
            self.state.drops.fetch_add(1, Ordering::AcqRel);
        }
    }
    let temporary = tempfile::tempdir().unwrap();
    let video = State::new();
    let audio = State::new();
    let factory_video = Arc::clone(&video);
    let factory_audio = Arc::clone(&audio);
    let session = AvRecordingSession::start_with_factories(
        temporary.path(),
        AvRecordingConfig {
            session_id: "control-failure-av".to_string(),
            source_id: "synthetic-control".to_string(),
            physical_x: 0,
            physical_y: 0,
            width: 64,
            height: 48,
            frames_per_second: 10,
            include_cursor: false,
            audio_channels: 2,
            segment_duration_ns: 1_000_000_000,
        },
        move |_| {
            Ok(FaultSource {
                state: factory_video,
                emitted: false,
                fault: Fault::ResumeTimestamp,
            })
        },
        move |_| {
            Ok(AudioSource {
                state: factory_audio,
                emitted: false,
            })
        },
    )
    .unwrap();
    assert!(observe(
        || video.ready.load(Ordering::Acquire) && audio.ready.load(Ordering::Acquire)
    ));
    session.pause().unwrap();
    let directory = session.session_directory().to_path_buf();
    let control = session.resume();
    let terminated = observe(|| session.has_terminated_worker());
    let stopped = session.stop();
    retain_session(&directory);
    eprintln!(
        "av owner control failure: terminated={terminated}, control={control:?}, stop={stopped:?}"
    );
    assert!(
        matches!(control, Err(AvRecordingSessionError::VideoCapture(error)) if error == timeline_error())
    );
    assert!(
        matches!(stopped, Err(AvRecordingSessionError::VideoCapture(error)) if error == timeline_error())
    );
    assert!(terminated);
    assert_eq!(video.drops.load(Ordering::Acquire), 1);
    assert_eq!(audio.drops.load(Ordering::Acquire), 1);
    assert_eq!(*audio.hooks.lock().unwrap(), ["pause"]);
    assert_interrupted(&directory);
}
