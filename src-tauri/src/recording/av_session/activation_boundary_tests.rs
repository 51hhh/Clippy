use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

struct ActivationVideo {
    probe: Arc<AtomicUsize>,
    delivered: usize,
    drops: Arc<AtomicUsize>,
}
impl RecordingFrameSource for ActivationVideo {
    type Error = FixtureError;
    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
        self.capture_next_available()?
            .ok_or(FixtureError("fixture pending"))
    }
    fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
        let timestamp = match self.delivered {
            0 => 100_000_000,
            1 if self.probe.load(Ordering::Acquire) == 2 => 560_000_000,
            _ => return Ok(None),
        };
        let sequence = self.delivered as u64;
        self.delivered += 1;
        Ok(Some(CapturedFrame {
            sequence,
            captured_at_ns: timestamp,
            width: 64,
            height: 48,
            stride: 256,
            rgba: vec![sequence as u8; 64 * 48 * 4].into_boxed_slice(),
        }))
    }
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(700_000_000)
    }
    fn pause_capture(&mut self) -> Result<u64, Self::Error> {
        Ok(200_000_000)
    }
    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        Ok(520_000_000)
    }
}
impl Drop for ActivationVideo {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
    }
}
struct ActivationAudio {
    probe: Arc<AtomicUsize>,
    delivered: usize,
    drops: Arc<AtomicUsize>,
}
impl RecordingAudioSource for ActivationAudio {
    type Error = FixtureError;
    fn capture_next_available(
        &mut self,
        _: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        let timestamp = match self.delivered {
            0 => 100_000_000,
            1 if self.probe.load(Ordering::Acquire) == 2 => 560_000_000,
            _ => {
                thread::sleep(Duration::from_millis(2));
                return Ok(None);
            }
        };
        let sequence = self.delivered as u64;
        self.delivered += 1;
        Ok(Some(chunk(sequence, timestamp)))
    }
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(700_000_000)
    }
    fn pause_capture(&mut self) -> Result<u64, Self::Error> {
        Ok(200_000_000)
    }
    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        Ok(560_000_000)
    }
}
impl Drop for ActivationAudio {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
    }
}
fn wait_activation_session(ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < deadline, "双轨边界合同必须在预算内完成");
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn audio_activation_original_av_session_commits_exact_resume_pcm() {
    let temporary = tempfile::tempdir().unwrap();
    let probe = Arc::new(AtomicUsize::new(0));
    let video_drops = Arc::new(AtomicUsize::new(0));
    let audio_drops = Arc::new(AtomicUsize::new(0));
    let video = ActivationVideo {
        probe: Arc::clone(&probe),
        delivered: 0,
        drops: Arc::clone(&video_drops),
    };
    let audio = ActivationAudio {
        probe: Arc::clone(&probe),
        delivered: 0,
        drops: Arc::clone(&audio_drops),
    };
    let mut configuration = config("activation-exact-resume");
    configuration.width = 64;
    configuration.height = 48;
    configuration.frames_per_second = 100;
    configuration.segment_duration_ns = 100_000_000;
    let session = AvRecordingSession::start_with_factories(
        temporary.path(),
        configuration,
        move |_| Ok(video),
        move |_| Ok(audio),
    )
    .unwrap();
    let directory = session.session_directory().to_path_buf();
    wait_activation_session(|| {
        session.video_pipeline.stats().unwrap().accepted_frames == 1
            && session.audio_pipeline.stats().unwrap().accepted_chunks == 1
    });
    session.pause().unwrap();
    session.resume().unwrap();
    probe.store(2, Ordering::Release);
    wait_activation_session(|| {
        session.audio_capture.as_ref().unwrap().is_finished()
            || (session.video_pipeline.stats().unwrap().accepted_frames == 2
                && session.audio_pipeline.stats().unwrap().accepted_chunks == 2)
    });
    let result = session.stop();
    let manifest: Value =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    if let Some(root) = std::env::var_os("CLIPPY_AUDIO_ACTIVATION_EVIDENCE") {
        let root = PathBuf::from(root).join("original-av-session");
        fs::create_dir_all(&root).unwrap();
        let mut files = Vec::new();
        for entry in fs::read_dir(&directory).unwrap() {
            let entry = entry.unwrap();
            if !entry.file_type().unwrap().is_file() {
                continue;
            }
            let bytes = fs::read(entry.path()).unwrap();
            fs::write(root.join(entry.file_name()), &bytes).unwrap();
            files.push(serde_json::json!({"name":entry.file_name().to_string_lossy(),"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes))}));
        }
        fs::write(root.join("observation.json"),serde_json::to_vec_pretty(&serde_json::json!({"result":format!("{result:?}"),"manifest":manifest,"files":files,"sourceDrops":[video_drops.load(Ordering::Acquire),audio_drops.load(Ordering::Acquire)]})).unwrap()).unwrap();
    }
    let report = result.unwrap();
    assert_eq!(manifest["state"], "complete");
    assert_eq!(report.duration_ns, 280_000_000);
    assert_eq!(report.video_encoded_frames, 28);
    assert_eq!(report.audio_pcm_frames, 13_440);
    assert_eq!(report.audio_accepted_chunks, 2);
    assert_eq!(video_drops.load(Ordering::Acquire), 1);
    assert_eq!(audio_drops.load(Ordering::Acquire), 1);
    assert!(report.final_output_path.unwrap().is_file());
}
