use sha2::{Digest,Sha256};

struct SkewVideo {mode:SkewMode,delivered:u64,resumed:bool,drops:Arc<AtomicUsize>}
impl RecordingFrameSource for SkewVideo {
    type Error=SkewError;
    fn capture_next(&mut self)->Result<CapturedFrame,Self::Error> {self.capture_next_available()?.ok_or(SkewError)}
    fn capture_next_available(&mut self)->Result<Option<CapturedFrame>,Self::Error> {
        let timestamp=match self.delivered {0=>0,1 if self.resumed=>1_010_000_000,_=>return Ok(None)};
        let sequence=self.delivered;self.delivered+=1;
        Ok(Some(CapturedFrame {sequence,captured_at_ns:timestamp,width:64,height:48,stride:256,rgba:vec![25;64*48*4].into_boxed_slice()}))
    }
    fn control_timestamp_ns(&mut self)->Result<u64,Self::Error> {Ok(if self.mode==SkewMode::StopTail {40_000_000} else if self.resumed&&self.delivered==2 {1_010_000_000} else if self.resumed {1_000_000_000} else {0})}
    fn stop_capture(&mut self)->Result<u64,Self::Error> {Ok(if self.mode==SkewMode::StopTail {40_000_000} else {1_040_000_000})}
    fn pause_capture(&mut self)->Result<u64,Self::Error> {Ok(20_000_000)}
    fn resume_capture(&mut self)->Result<u64,Self::Error> {self.resumed=true;Ok(1_000_000_000)}
}
impl Drop for SkewVideo {fn drop(&mut self) {self.drops.fetch_add(1,Ordering::AcqRel);}}

fn check_skew_av(mode:SkewMode) {
    let temporary=tempfile::tempdir().unwrap(); let vd=Arc::new(AtomicUsize::new(0));
    let video=SkewVideo {mode,delivered:0,resumed:false,drops:Arc::clone(&vd)};let (audio,ad)=skew_source(mode,true);
    let name=if mode==SkewMode::Resume {"av-resume"} else {"av-tail"};
    let mut configuration=config(name);configuration.width=64;configuration.height=48;configuration.frames_per_second=100;configuration.segment_duration_ns=100_000_000;
    let session=AvRecordingSession::start_with_factories(temporary.path(),configuration,move |_|Ok(video),move |_|Ok(audio)).unwrap();
    let directory=session.session_directory().to_path_buf();
    wait_skew(||ad.started.load(Ordering::Acquire)>=2);
    if mode==SkewMode::Resume {
        wait_skew(||session.audio_capture.as_ref().unwrap().is_finished()||(session.audio_pipeline.stats().unwrap().accepted_chunks==1&&session.video_pipeline.stats().unwrap().accepted_frames==1));
        session.pause().unwrap();session.resume().unwrap();
        wait_skew(||session.audio_capture.as_ref().unwrap().is_finished()||(session.audio_pipeline.stats().unwrap().accepted_chunks==3&&session.video_pipeline.stats().unwrap().accepted_frames==2));
    }
    let result=session.stop(); let manifest:Value=serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();let mut files=Vec::new();
    if let Some(root)=std::env::var_os("CLIPPY_MIXED_SKEW_EVIDENCE") {
        let root=PathBuf::from(root).join(name);fs::create_dir_all(&root).unwrap();
        for entry in fs::read_dir(&directory).unwrap() {let entry=entry.unwrap();if !entry.file_type().unwrap().is_file(){continue;}
            let bytes=fs::read(entry.path()).unwrap();fs::write(root.join(entry.file_name()),&bytes).unwrap();
            files.push(serde_json::json!({"name":entry.file_name().to_string_lossy(),"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes))}));
        }
    }
    skew_observation(name,serde_json::json!({"result":format!("{result:?}"),"manifest":manifest,"files":files,"sourceDrops":[vd.load(Ordering::Acquire),ad.load(Ordering::Acquire)]}));
    let report=result.unwrap();let frames=if mode==SkewMode::Resume {2880} else {1920};let duration=if mode==SkewMode::Resume {60_000_000} else {40_000_000};
    assert_eq!(manifest["state"],"complete");assert_eq!(report.duration_ns,duration);assert_eq!(report.audio_encoder_input_frames,frames);assert_eq!(report.audio_pcm_frames,frames);
    assert_eq!(report.video_encoded_frames,if mode==SkewMode::Resume {6} else {4});
    assert_eq!(report.audio_dropped_before_video_frames,0);assert_eq!(report.audio_trimmed_before_video_frames,0);
    assert_eq!(vd.load(Ordering::Acquire),1);assert_eq!(ad.load(Ordering::Acquire),2);assert!(report.final_output_path.unwrap().is_file());
}

#[test]
fn mixed_control_skew_original_av_tail_preserves_later_samples() {check_skew_av(SkewMode::StopTail);}
#[test]
fn mixed_control_skew_original_av_resume_preserves_earlier_source_pcm() {check_skew_av(SkewMode::Resume);}
