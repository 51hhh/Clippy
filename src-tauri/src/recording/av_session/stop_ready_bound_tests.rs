use sha2::{Digest, Sha256};
struct StopBoundVideo {
    delivered: bool,
    drops: Arc<AtomicUsize>,
}
impl RecordingFrameSource for StopBoundVideo {
    type Error = StopBoundError;
    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
        self.capture_next_available()?
            .ok_or(StopBoundError("no frame"))
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
    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        Ok(60_000_000)
    }
}
impl Drop for StopBoundVideo {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
    }
}
fn check_stop_bound_av(fail_second: bool) {
    let temporary = tempfile::tempdir().unwrap();
    let video_drops = Arc::new(AtomicUsize::new(0));
    let video = StopBoundVideo {
        delivered: false,
        drops: Arc::clone(&video_drops),
    };
    let (audio, probe) = stop_bound_batches(fail_second);
    let name = if fail_second {
        "av-late-batch-error"
    } else {
        "av-complete-batches"
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
    wait_stop_bound(|| {
        session.audio_pipeline.stats().unwrap().accepted_chunks == 1
            && session.video_pipeline.stats().unwrap().accepted_frames == 1
    });
    let directory = session.session_directory().to_path_buf();
    let result = session.stop();
    let manifest: Value =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    let mut files = Vec::new();
    if let Some(root) = std::env::var_os("CLIPPY_MIXED_STOP_BOUND_EVIDENCE") {
        let root = PathBuf::from(root).join(name);
        fs::create_dir_all(&root).unwrap();
        for entry in fs::read_dir(&directory).unwrap() {
            let entry = entry.unwrap();
            if !entry.file_type().unwrap().is_file() {
                continue;
            }
            let data = fs::read(entry.path()).unwrap();
            fs::write(root.join(entry.file_name()), &data).unwrap();
            files.push(serde_json::json!({"name":entry.file_name().to_string_lossy(),"bytes":data.len(),"sha256":format!("{:x}",Sha256::digest(&data))}));
        }
    }
    stop_bound_observation(
        name,
        serde_json::json!({"result":format!("{result:?}"),"manifest":manifest,"files":files,"sourceDrops":[video_drops.load(Ordering::Acquire),probe.drops.load(Ordering::Acquire)],"stopAttempts":probe.stop_attempts.load(Ordering::Acquire)}),
    );
    assert_eq!(video_drops.load(Ordering::Acquire), 1);
    assert_eq!(probe.drops.load(Ordering::Acquire), 2);
    assert_eq!(probe.stop_attempts.load(Ordering::Acquire), 2);
    if fail_second {
        assert_eq!(manifest["state"], "interrupted");
        assert!(
            matches!(result,Err(AvRecordingSessionError::AudioCapture(AudioCaptureWorkerError::Source(ref s))) if s.contains("second finite tail batch failed"))
        );
    } else {
        let report = result.unwrap();
        assert_eq!(manifest["state"], "complete");
        assert_eq!(report.duration_ns, 60_000_000);
        assert_eq!(report.audio_encoder_input_frames, 2880);
        assert_eq!(report.audio_pcm_frames, 2880);
        assert_eq!(report.video_encoded_frames, 6);
        assert_eq!(report.audio_dropped_before_video_frames, 0);
        assert_eq!(report.audio_trimmed_before_video_frames, 0);
        assert!(report.final_output_path.unwrap().is_file());
    }
}
#[test]
fn mixed_stop_ready_bound_original_av_keeps_all_finite_pcm_batches() {
    check_stop_bound_av(false);
}
#[test]
fn mixed_stop_ready_bound_original_av_late_tail_failure_is_interrupted() {
    check_stop_bound_av(true);
}
