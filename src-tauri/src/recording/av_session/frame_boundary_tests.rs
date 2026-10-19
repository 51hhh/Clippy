use sha2::{Digest, Sha256};

struct BoundaryVideo { delivered: bool, drops: Arc<AtomicUsize> }
impl RecordingFrameSource for BoundaryVideo {
    type Error = BoundaryError;
    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> { self.capture_next_available()?.ok_or(BoundaryError) }
    fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
        if self.delivered { return Ok(None); } self.delivered = true;
        Ok(Some(CapturedFrame { sequence:0,captured_at_ns:1_000_000_000,width:64,height:48,stride:256,rgba:vec![25;64*48*4].into_boxed_slice() }))
    }
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> { Ok(1_020_000_000) }
}
impl Drop for BoundaryVideo { fn drop(&mut self) { self.drops.fetch_add(1, Ordering::AcqRel); } }

#[test]
fn mixed_frame_boundary_original_av_owner_commits_finite_tail_with_all_inputs() {
    let temporary = tempfile::tempdir().unwrap(); let video_drops = Arc::new(AtomicUsize::new(0));
    let video = BoundaryVideo { delivered:false, drops:Arc::clone(&video_drops) };
    let (audio,audio_drops) = boundary_source(1_000_000_000,1_000_020_000,640,1_013_353_333,true);
    let mut configuration = config("mixed-frame-boundary"); configuration.width=64; configuration.height=48;
    configuration.frames_per_second=100; configuration.segment_duration_ns=100_000_000;
    let session = AvRecordingSession::start_with_factories(temporary.path(),configuration,move |_| Ok(video),move |_| Ok(audio)).unwrap();
    let directory = session.session_directory().to_path_buf(); let result = session.stop();
    let manifest: Value = serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    let mut files = Vec::new();
    if let Some(root) = std::env::var_os("CLIPPY_MIXED_FRAME_EVIDENCE") {
        let root = PathBuf::from(root).join("av-session"); fs::create_dir_all(&root).unwrap();
        for entry in fs::read_dir(&directory).unwrap() {
            let entry = entry.unwrap(); if !entry.file_type().unwrap().is_file() { continue; }
            let bytes = fs::read(entry.path()).unwrap(); fs::write(root.join(entry.file_name()), &bytes).unwrap();
            files.push(serde_json::json!({"name":entry.file_name().to_string_lossy(),"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes))}));
        }
    }
    boundary_observation("av-session",serde_json::json!({"result":format!("{result:?}"),"manifest":manifest,"files":files,"sourceDrops":[video_drops.load(Ordering::Acquire),audio_drops.load(Ordering::Acquire)]}));
    let report = result.unwrap(); assert_eq!(manifest["state"],"complete"); assert_eq!(report.duration_ns,20_000_000);
    assert_eq!(report.video_encoded_frames,2); assert_eq!(report.audio_encoder_input_frames,641);
    assert_eq!(report.audio_accepted_chunks,1); assert_eq!(report.audio_pcm_frames,960);
    assert_eq!(report.audio_dropped_before_video_frames,0); assert_eq!(report.audio_trimmed_before_video_frames,0);
    assert_eq!(video_drops.load(Ordering::Acquire),1); assert_eq!(audio_drops.load(Ordering::Acquire),2);
    assert!(report.final_output_path.unwrap().is_file());
}
