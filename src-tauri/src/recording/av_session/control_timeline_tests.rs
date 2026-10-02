use crate::recording::audio_worker::RecordingAudioSource;
use crate::recording::av_timeline::{AudioEpochOutcome, AvTimelineCoordinator};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

struct ControlProbe {
    pause: [u64; 2],
    resume: [u64; 2],
    sample: u64,
    round: AtomicUsize,
    allowed: AtomicUsize,
    drops: [AtomicUsize; 2],
    hooks: Mutex<Vec<(usize, &'static str, u64)>>,
}

impl ControlProbe {
    fn new(pause: [u64; 2], resume: [u64; 2], sample: u64) -> Arc<Self> {
        Arc::new(Self {
            pause,
            resume,
            sample,
            round: AtomicUsize::new(0),
            allowed: AtomicUsize::new(1),
            drops: [AtomicUsize::new(0), AtomicUsize::new(0)],
            hooks: Mutex::new(Vec::new()),
        })
    }

    fn stamp(&self, side: usize, action: &'static str) -> u64 {
        let offset = self.round.load(Ordering::Acquire) as u64 * 1_200_000_000;
        let stamp = offset
            + match action {
                "pause" => self.pause[side],
                "resume" => self.resume[side],
                "stop" => 1_000_000_000,
                _ => unreachable!(),
            };
        self.hooks.lock().unwrap().push((side, action, stamp));
        stamp
    }

    fn frame_stamp(&self, sequence: u64) -> u64 {
        if sequence == 0 {
            100_000_000
        } else {
            self.sample + (sequence - 1) * 1_200_000_000
        }
    }
}

#[derive(Debug)]
struct ProbeError;
impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("合成控制时序源")
    }
}
impl std::error::Error for ProbeError {}

struct ProbeVideo {
    probe: Arc<ControlProbe>,
    sequence: u64,
    running: bool,
}
impl RecordingFrameSource for ProbeVideo {
    type Error = ProbeError;
    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
        unreachable!("仅使用可控推送入口")
    }
    fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
        if !self.running || self.sequence as usize >= self.probe.allowed.load(Ordering::Acquire) {
            return Ok(None);
        }
        let sequence = self.sequence;
        self.sequence += 1;
        Ok(Some(CapturedFrame {
            sequence,
            captured_at_ns: self.probe.frame_stamp(sequence),
            width: 64,
            height: 48,
            stride: 256,
            rgba: vec![if sequence == 0 { 0x40 } else { 0xc0 }; 64 * 48 * 4].into_boxed_slice(),
        }))
    }
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(self.probe.stamp(0, "stop"))
    }
    fn pause_capture(&mut self) -> Result<u64, Self::Error> {
        self.running = false;
        Ok(self.probe.stamp(0, "pause"))
    }
    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        self.running = true;
        Ok(self.probe.stamp(0, "resume"))
    }
}
impl Drop for ProbeVideo {
    fn drop(&mut self) {
        self.probe.drops[0].fetch_add(1, Ordering::AcqRel);
    }
}

struct ProbeAudio {
    probe: Arc<ControlProbe>,
    sequence: u64,
    running: bool,
}
impl RecordingAudioSource for ProbeAudio {
    type Error = ProbeError;
    fn capture_next_available(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        if !self.running || self.sequence as usize >= self.probe.allowed.load(Ordering::Acquire) {
            std::thread::sleep(timeout);
            return Ok(None);
        }
        let sequence = self.sequence;
        self.sequence += 1;
        Ok(Some(CapturedAudioChunk {
            sequence,
            captured_at_ns: self.probe.frame_stamp(sequence),
            format: AudioFormat::normalized(2),
            frame_count: 960,
            samples: vec![0.1; 1920].into_boxed_slice(),
        }))
    }
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(self.probe.stamp(1, "stop"))
    }
    fn pause_capture(&mut self) -> Result<u64, Self::Error> {
        self.running = false;
        Ok(self.probe.stamp(1, "pause"))
    }
    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        self.running = true;
        Ok(self.probe.stamp(1, "resume"))
    }
}
impl Drop for ProbeAudio {
    fn drop(&mut self) {
        self.probe.drops[1].fetch_add(1, Ordering::AcqRel);
    }
}

fn wait_probe(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(Instant::now() < deadline, "新夹具等待采样入队超时");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn record_probe(name: &str, value: &Value) {
    eprintln!("AV control timeline {name}: {value}");
    if let Some(path) = std::env::var_os("CLIPPY_AV_CONTROL_EVIDENCE") {
        let path = std::path::PathBuf::from(path);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join(format!("{name}.json")),
            serde_json::to_vec_pretty(value).unwrap(),
        )
        .unwrap();
    }
}

fn manual_owner(probe: &Arc<ControlProbe>, directory: &Path) -> AvRecordingSession {
    let video_pipeline = Arc::new(RecordingPipeline::default());
    let audio_pipeline = Arc::new(AudioPipeline::new(0));
    let video = CaptureWorker::spawn(
        ProbeVideo {
            probe: Arc::clone(probe),
            sequence: 0,
            running: true,
        },
        Arc::clone(&video_pipeline),
        100,
    )
    .unwrap();
    let audio_probe = Arc::clone(probe);
    let audio = AudioCaptureWorker::spawn_with_factory(
        move |_| {
            Ok(ProbeAudio {
                probe: audio_probe,
                sequence: 0,
                running: true,
            })
        },
        RecordingSessionClock::new(),
        Arc::clone(&audio_pipeline),
    )
    .unwrap();
    // 不创建 encoder：保留原 worker 的真实队列，以检查原 owner 控制后的媒体映射。
    // 完整 start/codec/stop/文件合同由下方另一用例运行，不能把此处冒充该层。
    let session = AvRecordingSession {
        video_pipeline,
        audio_pipeline,
        video_capture: Some(video),
        audio_capture: Some(audio),
        encoder: None,
        session_directory: directory.to_path_buf(),
        settled: false,
    };
    wait_probe(|| {
        session
            .video_capture
            .as_ref()
            .unwrap()
            .is_first_frame_ready()
            && session.video_pipeline.stats().unwrap().accepted_frames == 1
            && session.audio_pipeline.stats().unwrap().accepted_chunks == 1
    });
    session
}

fn owner_mapping(
    name: &str,
    pause: [u64; 2],
    resume: [u64; 2],
    sample: u64,
    rounds: usize,
) -> Vec<(u64, u64, u64)> {
    let temporary = tempfile::tempdir().unwrap();
    let probe = ControlProbe::new(pause, resume, sample);
    let session = manual_owner(&probe, temporary.path());
    let mut coordinator = AvTimelineCoordinator::new(0);
    coordinator
        .establish_video_epoch(&session.video_pipeline.pop().unwrap().unwrap())
        .unwrap();
    coordinator
        .align_audio(session.audio_pipeline.pop().unwrap().unwrap())
        .unwrap();
    let mut observed = Vec::new();
    for round in 0..rounds {
        probe.round.store(round, Ordering::Release);
        session.pause().unwrap();
        session.resume().unwrap();
        probe.allowed.store(round + 2, Ordering::Release);
        wait_probe(|| {
            session.video_pipeline.stats().unwrap().accepted_frames == round as u64 + 2
                && session.audio_pipeline.stats().unwrap().accepted_chunks == round as u64 + 2
        });
        let video = session.video_pipeline.pop().unwrap().unwrap();
        let AudioEpochOutcome::Aligned { chunk, .. } = coordinator
            .align_audio(session.audio_pipeline.pop().unwrap().unwrap())
            .unwrap()
        else {
            panic!("恢复 PCM 应保留");
        };
        observed.push((
            video.presentation_at_ns,
            chunk.presentation_at_ns,
            chunk.gap_before_ns,
        ));
    }
    drop(session);
    let hooks = probe.hooks.lock().unwrap();
    let controls: Vec<_> = hooks
        .iter()
        .filter(|(_, action, _)| *action != "stop")
        .collect();
    record_probe(
        name,
        &serde_json::json!({"observed":observed,"controls":controls,"drops":[probe.drops[0].load(Ordering::Acquire),probe.drops[1].load(Ordering::Acquire)]}),
    );
    assert_eq!(controls.len(), rounds * 4, "每源每次控制只能调用一次");
    assert_eq!(probe.drops[0].load(Ordering::Acquire), 1);
    assert_eq!(probe.drops[1].load(Ordering::Acquire), 1);
    observed
}

#[test]
fn av_control_timeline_pause_cost_does_not_shift_tracks() {
    let observed = owner_mapping(
        "pause-cost",
        [180_000_000, 200_000_000],
        [500_000_000; 2],
        700_000_000,
        1,
    );
    assert_eq!(observed[0].0, observed[0].1);
    assert_eq!(observed[0].0, 300_000_000);
}

#[test]
fn av_control_timeline_resume_cost_does_not_shift_tracks() {
    let observed = owner_mapping(
        "resume-cost",
        [200_000_000; 2],
        [520_000_000, 560_000_000],
        700_000_000,
        1,
    );
    assert_eq!(observed[0].0, observed[0].1);
    assert_eq!(observed[0].0, 280_000_000);
}

#[test]
fn av_control_timeline_repeated_controls_do_not_accumulate_offset() {
    let observed = owner_mapping(
        "five-rounds",
        [180_000_000, 200_000_000],
        [520_000_000, 560_000_000],
        700_000_000,
        5,
    );
    for (round, (video, audio, _)) in observed.iter().enumerate() {
        assert_eq!(video, audio);
        assert_eq!(*video, 280_000_000 + round as u64 * 880_000_000);
    }
}

#[test]
fn av_control_timeline_equal_boundaries_keep_original_mapping() {
    let observed = owner_mapping(
        "equal-boundaries",
        [200_000_000; 2],
        [520_000_000; 2],
        700_000_000,
        5,
    );
    for (round, (video, audio, _)) in observed.iter().enumerate() {
        assert_eq!(video, audio);
        assert_eq!(*video, 280_000_000 + round as u64 * 880_000_000);
    }
}

#[test]
fn av_control_timeline_late_audio_resume_keeps_real_gap() {
    let observed = owner_mapping(
        "resume-gap",
        [200_000_000; 2],
        [520_000_000, 560_000_000],
        580_000_000,
        1,
    );
    assert_eq!(observed[0], (160_000_000, 160_000_000, 140_000_000));
}

#[test]
fn av_control_timeline_stop_while_paused_uses_common_boundary() {
    let temporary = tempfile::tempdir().unwrap();
    let probe = ControlProbe::new(
        [180_000_000, 200_000_000],
        [520_000_000, 560_000_000],
        700_000_000,
    );
    let mut session = manual_owner(&probe, temporary.path());
    session.pause().unwrap();
    let video = session
        .video_capture
        .take()
        .unwrap()
        .stop()
        .unwrap()
        .duration_ns
        .unwrap();
    let audio = session
        .audio_capture
        .take()
        .unwrap()
        .stop()
        .unwrap()
        .duration_ns
        .unwrap()
        - 100_000_000;
    drop(session);
    record_probe(
        "paused-stop",
        &serde_json::json!({"video":video,"audio":audio}),
    );
    assert_eq!(video, audio);
    assert_eq!(video, 100_000_000);
}

#[test]
fn av_control_timeline_invalid_source_resume_still_fails() {
    let temporary = tempfile::tempdir().unwrap();
    let probe = ControlProbe::new([200_000_000; 2], [520_000_000, 200_000_000], 700_000_000);
    let session = manual_owner(&probe, temporary.path());
    session.pause().unwrap();
    let error = session.resume().unwrap_err();
    record_probe(
        "bad-raw-resume",
        &serde_json::json!({"error":error.to_string()}),
    );
    assert!(
        matches!(error, AvRecordingSessionError::ControlDiverged(ref message) if message.contains(&AudioCaptureWorkerError::Pipeline(AudioPipelineError::InvalidResumeTimestamp).to_string()))
    );
    drop(session);
    assert_eq!(probe.drops[0].load(Ordering::Acquire), 1);
    assert_eq!(probe.drops[1].load(Ordering::Acquire), 1);
}

#[test]
fn av_control_timeline_original_session_commits_common_duration() {
    let temporary = tempfile::tempdir().unwrap();
    let probe = ControlProbe::new(
        [180_000_000, 200_000_000],
        [520_000_000, 560_000_000],
        700_000_000,
    );
    let video_probe = Arc::clone(&probe);
    let audio_probe = Arc::clone(&probe);
    let session = AvRecordingSession::start_with_factories(
        temporary.path(),
        AvRecordingConfig {
            session_id: "control-timeline".into(),
            source_id: "synthetic-control".into(),
            physical_x: 0,
            physical_y: 0,
            width: 64,
            height: 48,
            frames_per_second: 100,
            include_cursor: false,
            audio_channels: 2,
            segment_duration_ns: 1_000_000_000,
        },
        move |_| {
            Ok(ProbeVideo {
                probe: video_probe,
                sequence: 0,
                running: true,
            })
        },
        move |_| {
            Ok(ProbeAudio {
                probe: audio_probe,
                sequence: 0,
                running: true,
            })
        },
    )
    .unwrap();
    wait_probe(|| {
        session
            .video_capture
            .as_ref()
            .unwrap()
            .is_first_frame_ready()
            && session.video_pipeline.stats().unwrap().accepted_frames == 1
            && session.audio_pipeline.stats().unwrap().accepted_chunks == 1
    });
    session.pause().unwrap();
    session.resume().unwrap();
    probe.allowed.store(2, Ordering::Release);
    wait_probe(|| {
        session.video_pipeline.stats().unwrap().accepted_frames == 2
            && session.audio_pipeline.stats().unwrap().accepted_chunks == 2
    });
    let directory = session.session_directory().to_path_buf();
    let report = session.stop().unwrap();
    let manifest: Value =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    if let Some(path) = std::env::var_os("CLIPPY_AV_CONTROL_EVIDENCE") {
        let path = std::path::PathBuf::from(path).join("session");
        fs::create_dir_all(&path).unwrap();
        let mut files = Vec::new();
        for item in fs::read_dir(&directory).unwrap() {
            let item = item.unwrap();
            if item.path().is_file() {
                let bytes = fs::read(item.path()).unwrap();
                fs::write(path.join(item.file_name()), &bytes).unwrap();
                files.push(serde_json::json!({"file":item.file_name().to_string_lossy(),"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes))}));
            }
        }
        fs::write(
            path.join("FILES.json"),
            serde_json::to_vec_pretty(&files).unwrap(),
        )
        .unwrap();
    }
    record_probe(
        "original-session",
        &serde_json::json!({"duration":report.duration_ns,"videoFrames":report.video_encoded_frames,"pcmFrames":report.audio_pcm_frames,"capturedVideo":report.video_captured_frames,"capturedAudio":report.audio_captured_frames,"manifest":manifest}),
    );
    assert_eq!(report.duration_ns, 580_000_000);
    assert_eq!(report.video_encoded_frames, 58);
    assert_eq!(report.audio_pcm_frames, 27_840);
    assert_eq!(manifest["state"], "complete");
    assert_eq!(probe.drops[0].load(Ordering::Acquire), 1);
    assert_eq!(probe.drops[1].load(Ordering::Acquire), 1);
}
