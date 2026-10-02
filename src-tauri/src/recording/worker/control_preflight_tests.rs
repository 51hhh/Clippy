use crate::recording::segmenting::RecordingEncoder;
use crate::recording::session::{DiagnosticRecordingConfig, DiagnosticRecordingSession};
use crate::recording::timeline::TimelineError;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, AtomicUsize};

#[derive(Default)]
struct ControlState {
    release_frame: AtomicBool,
    running: AtomicBool,
    clock: AtomicU64,
    hooks: Mutex<Vec<&'static str>>,
    drops: AtomicUsize,
}

impl ControlState {
    fn ready() -> Arc<Self> {
        Arc::new(Self {
            running: AtomicBool::new(true),
            clock: AtomicU64::new(100_000_000),
            ..Self::default()
        })
    }

    fn tick(&self) -> u64 {
        self.clock.fetch_add(100_000_000, Ordering::SeqCst)
    }

    fn hooks(&self) -> Vec<&'static str> {
        self.hooks.lock().unwrap().clone()
    }
}

struct ControlledPushSource {
    state: Arc<ControlState>,
    emitted: bool,
    fail_pause: bool,
}

impl RecordingFrameSource for ControlledPushSource {
    type Error = FakeSourceError;

    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
        unreachable!("原 worker 应调用推送源短轮询入口")
    }

    fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
        if self.emitted
            || !self.state.running.load(Ordering::Acquire)
            || !self.state.release_frame.load(Ordering::Acquire)
        {
            return Ok(None);
        }
        self.emitted = true;
        let mut rgba = vec![80; 64 * 48 * 4];
        for alpha in rgba.iter_mut().skip(3).step_by(4) {
            *alpha = 255;
        }
        Ok(Some(CapturedFrame {
            sequence: 0,
            captured_at_ns: self.state.tick(),
            width: 64,
            height: 48,
            stride: 256,
            rgba: rgba.into_boxed_slice(),
        }))
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(self.state.tick())
    }

    fn pause_capture(&mut self) -> Result<u64, Self::Error> {
        self.state.hooks.lock().unwrap().push("pause");
        if self.fail_pause {
            return Err(FakeSourceError);
        }
        self.state.running.store(false, Ordering::Release);
        self.control_timestamp_ns()
    }

    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        self.state.hooks.lock().unwrap().push("resume");
        self.state.running.store(true, Ordering::Release);
        self.control_timestamp_ns()
    }

    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        self.state.hooks.lock().unwrap().push("stop");
        self.state.running.store(false, Ordering::Release);
        self.control_timestamp_ns()
    }
}

impl Drop for ControlledPushSource {
    fn drop(&mut self) {
        self.state.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn controlled_worker(
    state: &Arc<ControlState>,
    pipeline: &Arc<RecordingPipeline>,
    fail_pause: bool,
) -> CaptureWorker {
    CaptureWorker::spawn(
        ControlledPushSource {
            state: Arc::clone(state),
            emitted: false,
            fail_pause,
        },
        Arc::clone(pipeline),
        120,
    )
    .unwrap()
}

fn await_frame(pipeline: &RecordingPipeline) -> bool {
    let end = Instant::now() + Duration::from_millis(300);
    while pipeline.stats().unwrap().accepted_frames == 0 {
        if Instant::now() >= end {
            return false;
        }
        thread::sleep(Duration::from_millis(2));
    }
    true
}

#[test]
fn video_control_preflight_pause_before_first_frame_keeps_source_running() {
    let state = ControlState::ready();
    let pipeline = Arc::new(RecordingPipeline::default());
    let worker = controlled_worker(&state, &pipeline, false);
    let rejected = worker.pause();
    let hooks = state.hooks();
    let running = state.running.load(Ordering::Acquire);
    state.release_frame.store(true, Ordering::Release);
    let accepted = await_frame(&pipeline);
    let stopped = worker.stop();
    eprintln!(
        "prefirst pause: hooks={hooks:?}, running={running}, frame={accepted}, stop={stopped:?}"
    );
    assert!(hooks.is_empty(), "失败请求改变了平台源: {hooks:?}");
    assert_eq!(
        rejected,
        Err(CaptureWorkerError::Pipeline(PipelineError::Timeline(
            TimelineError::PauseBeforeFirstFrame
        )))
    );
    assert!(running && accepted);
    assert_eq!(stopped.unwrap().captured_frames, 1);
    assert_eq!(state.drops.load(Ordering::Acquire), 1);
}

#[test]
fn video_control_preflight_resume_before_first_frame_has_no_native_call() {
    let state = ControlState::ready();
    let pipeline = Arc::new(RecordingPipeline::default());
    let worker = controlled_worker(&state, &pipeline, false);
    let rejected = worker.resume();
    let hooks = state.hooks();
    state.release_frame.store(true, Ordering::Release);
    let accepted = await_frame(&pipeline);
    let stopped = worker.stop();
    assert!(hooks.is_empty(), "失败请求调用了平台恢复: {hooks:?}");
    assert_eq!(
        rejected,
        Err(CaptureWorkerError::Pipeline(PipelineError::Timeline(
            TimelineError::NotPaused
        )))
    );
    assert!(accepted);
    assert_eq!(stopped.unwrap().captured_frames, 1);
}

#[test]
fn video_control_preflight_duplicate_pause_does_not_repeat_native_control() {
    let state = ControlState::ready();
    state.release_frame.store(true, Ordering::Release);
    let pipeline = Arc::new(RecordingPipeline::default());
    let worker = controlled_worker(&state, &pipeline, false);
    assert!(await_frame(&pipeline));
    worker.pause().unwrap();
    let rejected = worker.pause();
    let hooks = state.hooks();
    worker.resume().unwrap();
    let stopped = worker.stop();
    assert_eq!(hooks, ["pause"]);
    assert_eq!(
        rejected,
        Err(CaptureWorkerError::Pipeline(PipelineError::Timeline(
            TimelineError::AlreadyPaused
        )))
    );
    assert_eq!(stopped.unwrap().captured_frames, 1);
}

#[test]
fn video_control_preflight_active_resume_does_not_touch_source() {
    let state = ControlState::ready();
    state.release_frame.store(true, Ordering::Release);
    let pipeline = Arc::new(RecordingPipeline::default());
    let worker = controlled_worker(&state, &pipeline, false);
    assert!(await_frame(&pipeline));
    let rejected = worker.resume();
    let hooks = state.hooks();
    let stopped = worker.stop();
    assert!(hooks.is_empty(), "未暂停的源被恢复: {hooks:?}");
    assert_eq!(
        rejected,
        Err(CaptureWorkerError::Pipeline(PipelineError::Timeline(
            TimelineError::NotPaused
        )))
    );
    assert_eq!(stopped.unwrap().captured_frames, 1);
}

#[test]
fn video_control_preflight_normal_roundtrip_preserves_native_calls() {
    let state = ControlState::ready();
    state.release_frame.store(true, Ordering::Release);
    let pipeline = Arc::new(RecordingPipeline::default());
    let worker = controlled_worker(&state, &pipeline, false);
    assert!(await_frame(&pipeline));
    worker.pause().unwrap();
    worker.resume().unwrap();
    let stopped = worker.stop().unwrap();
    assert_eq!(state.hooks(), ["pause", "resume", "stop"]);
    assert_eq!(stopped.captured_frames, 1);
    assert!(stopped.duration_ns.is_some());
    assert_eq!(state.drops.load(Ordering::Acquire), 1);
}

#[test]
fn video_control_preflight_source_failure_keeps_original_root_error() {
    let state = ControlState::ready();
    state.release_frame.store(true, Ordering::Release);
    let pipeline = Arc::new(RecordingPipeline::default());
    let worker = controlled_worker(&state, &pipeline, true);
    assert!(await_frame(&pipeline));
    let rejected = worker.pause();
    let result = worker.wait();
    let expected = Err(CaptureWorkerError::Source(
        "fixture source failed".to_string(),
    ));
    assert_eq!(rejected, expected);
    assert_eq!(result, expected.map(|()| CaptureWorkerReport::default()));
    assert!(!pipeline.is_open().unwrap());
    assert_eq!(state.drops.load(Ordering::Acquire), 1);
}

fn assert_video_session_prefirst_pause(encoder: RecordingEncoder) {
    let temp = tempfile::tempdir().unwrap();
    let state = ControlState::ready();
    let session = DiagnosticRecordingSession::start(
        temp.path(),
        DiagnosticRecordingConfig {
            session_id: "control-preflight".to_string(),
            source_id: "synthetic-push".to_string(),
            physical_x: 0,
            physical_y: 0,
            width: 64,
            height: 48,
            frames_per_second: 10,
            include_cursor: false,
            encoder,
            segment_duration_ns: 1_000_000_000,
        },
        ControlledPushSource {
            state: Arc::clone(&state),
            emitted: false,
            fail_pause: false,
        },
    )
    .unwrap();
    let rejected = session.pause();
    let directory = session.session_directory().to_path_buf();
    let hooks = state.hooks();
    state.release_frame.store(true, Ordering::Release);
    // 这里只观察合成源的交付；失败时仍主动 stop/join 再报告断言。
    let end = Instant::now() + Duration::from_millis(300);
    while state.clock.load(Ordering::Acquire) == 100_000_000 && Instant::now() < end {
        thread::sleep(Duration::from_millis(2));
    }
    let stopped = session.stop();
    eprintln!("video session prefirst: hooks={hooks:?}, rejected={rejected:?}, stop={stopped:?}");
    assert!(
        hooks.is_empty(),
        "会话失败的暂停请求调用了平台源: {hooks:?}"
    );
    assert!(matches!(
        rejected,
        Err(
            crate::recording::session::DiagnosticRecordingError::Capture(
                CaptureWorkerError::Pipeline(PipelineError::Timeline(
                    TimelineError::PauseBeforeFirstFrame
                ))
            )
        )
    ));
    let report = stopped.unwrap();
    assert_eq!(report.captured_frames, 1);
    assert_eq!(report.encoded_frames, 1);
    assert_eq!(report.segment_paths.len(), 1);
    assert!(std::fs::metadata(&report.segment_paths[0]).unwrap().len() > 0);
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["state"], "complete");
    let segment = &manifest["segments"][0];
    let bytes = std::fs::read(&report.segment_paths[0]).unwrap();
    assert_eq!(segment["byteLength"].as_u64().unwrap(), bytes.len() as u64);
    assert_eq!(
        segment["sha256"].as_str().unwrap(),
        format!("{:x}", Sha256::digest(&bytes))
    );
    assert_eq!(segment["frameCount"], 1);
    assert!(crate::private_files::is_private(&report.segment_paths[0]));
    if matches!(encoder, RecordingEncoder::MjpegDiagnostic { .. }) {
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"AVI ");
        assert!(bytes.windows(4).any(|value| value == b"idx1"));
    }
    if let Some(path) = report.final_output_path {
        let output = &manifest["finalOutput"];
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(output["byteLength"].as_u64().unwrap(), bytes.len() as u64);
        assert_eq!(
            output["sha256"].as_str().unwrap(),
            format!("{:x}", Sha256::digest(&bytes))
        );
        assert!(crate::private_files::is_private(&path));
        #[cfg(feature = "recording-vp9-prototype")]
        {
            use crate::recording::mux::webm_remux::{
                remux_vp9_segments, WebmRemuxSource, WebmRemuxSpec,
            };
            let parsed = remux_vp9_segments(
                &[WebmRemuxSource {
                    path,
                    byte_length: bytes.len() as u64,
                    sha256: output["sha256"].as_str().unwrap().to_string(),
                    started_at_ns: 0,
                    duration_ns: report.duration_ns,
                    frame_count: report.encoded_frames,
                }],
                std::io::Cursor::new(Vec::new()),
                WebmRemuxSpec {
                    width: 64,
                    height: 48,
                    fps_numerator: 10,
                    fps_denominator: 1,
                },
            )
            .unwrap();
            assert_eq!(parsed.frame_count, 1);
            assert_eq!(parsed.duration_ns, report.duration_ns);
        }
    }
    assert_eq!(state.drops.load(Ordering::Acquire), 1);
}

#[test]
fn video_control_preflight_mjpeg_session_commits_after_early_pause_rejection() {
    assert_video_session_prefirst_pause(RecordingEncoder::MjpegDiagnostic { jpeg_quality: 85 });
}

#[cfg(feature = "recording-vp9-prototype")]
#[test]
fn video_control_preflight_vp9_session_commits_after_early_pause_rejection() {
    assert_video_session_prefirst_pause(RecordingEncoder::Vp9Prototype);
}
