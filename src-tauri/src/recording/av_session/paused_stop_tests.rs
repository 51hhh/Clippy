use sha2::{Digest, Sha256};

struct PausedStopVideo {
    delivered: bool,
    drops: Arc<AtomicUsize>,
}

impl RecordingFrameSource for PausedStopVideo {
    type Error = PausedStopError;

    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
        self.capture_next_available()?
            .ok_or(PausedStopError("no frame"))
    }

    fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
        if self.delivered {
            return Ok(None);
        }
        self.delivered = true;
        Ok(Some(CapturedFrame {
            sequence: 0,
            captured_at_ns: 0,
            width: 64,
            height: 48,
            stride: 256,
            rgba: vec![25; 64 * 48 * 4].into_boxed_slice(),
        }))
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(if self.delivered { 20_000_000 } else { 0 })
    }

    fn pause_capture(&mut self) -> Result<u64, Self::Error> {
        Ok(20_000_000)
    }
    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        Ok(2_040_000_000)
    }
}

impl Drop for PausedStopVideo {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
    }
}

fn check_paused_stop_av(fail_stop: bool) {
    let temporary = tempfile::tempdir().unwrap();
    let video_drops = Arc::new(AtomicUsize::new(0));
    let video = PausedStopVideo {
        delivered: false,
        drops: Arc::clone(&video_drops),
    };
    let (audio, probe) = paused_stop_source_with_failure(true, true, fail_stop);
    let name = if fail_stop {
        "av-real-stop-error"
    } else {
        "av-paused-stop"
    };
    let mut configuration = config(name);
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
    wait_paused_stop(|| {
        session.audio_pipeline.stats().unwrap().accepted_chunks == 1
            && session.video_pipeline.stats().unwrap().accepted_frames == 1
    });
    let directory = session.session_directory().to_path_buf();
    session.pause().unwrap();
    let result = session.stop();
    let manifest: Value =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    let mut files = Vec::new();
    if let Some(root) = std::env::var_os("CLIPPY_MIXED_PAUSED_STOP_EVIDENCE") {
        let root = PathBuf::from(root).join(name);
        fs::create_dir_all(&root).unwrap();
        for entry in fs::read_dir(&directory).unwrap() {
            let entry = entry.unwrap();
            if !entry.file_type().unwrap().is_file() {
                continue;
            }
            let bytes = fs::read(entry.path()).unwrap();
            fs::write(root.join(entry.file_name()), &bytes).unwrap();
            files.push(serde_json::json!({"name":entry.file_name().to_string_lossy(),"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes))}));
        }
    }
    paused_stop_observation(
        name,
        serde_json::json!({"result":format!("{result:?}"),"manifest":manifest,"files":files,"sourceDrops":[video_drops.load(Ordering::Acquire),probe.drops.load(Ordering::Acquire)],"stopAttempts":probe.stop_attempts.load(Ordering::Acquire)}),
    );
    assert_eq!(video_drops.load(Ordering::Acquire), 1);
    assert_eq!(probe.drops.load(Ordering::Acquire), 2);
    assert_eq!(probe.stop_attempts.load(Ordering::Acquire), 2);
    if fail_stop {
        assert_eq!(manifest["state"], "interrupted");
        assert!(
            matches!(result, Err(AvRecordingSessionError::AudioCapture(AudioCaptureWorkerError::Source(ref message))) if message.contains("microphone stop failed"))
        );
    } else {
        let report = result.unwrap();
        assert_eq!(manifest["state"], "complete");
        assert_eq!(report.duration_ns, 20_000_000);
        assert_eq!(report.audio_encoder_input_frames, 960);
        assert_eq!(report.audio_pcm_frames, 960);
        assert_eq!(report.video_encoded_frames, 2);
        assert_eq!(report.audio_dropped_before_video_frames, 0);
        assert_eq!(report.audio_trimmed_before_video_frames, 0);
        assert!(report.final_output_path.unwrap().is_file());
    }
}

#[test]
fn mixed_paused_stop_original_complete_av_keeps_only_active_media() {
    check_paused_stop_av(false);
}

#[test]
fn mixed_paused_stop_original_av_real_source_failure_stays_interrupted() {
    check_paused_stop_av(true);
}
