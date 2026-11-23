use crate::recording::audio::AudioPipelineError;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

pub(super) struct QpcVideo {
    pub(super) delivered: usize,
    pub(super) audio_progress: Arc<AtomicUsize>,
    pub(super) stopping: Arc<AtomicBool>,
    pub(super) drops: Arc<AtomicUsize>,
}
impl RecordingFrameSource for QpcVideo {
    type Error = FixtureError;
    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> { self.capture_next_available()?.ok_or(FixtureError("pending")) }
    fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
        let timestamp = match self.delivered { 0 => 100_000_000, 1 if self.audio_progress.load(Ordering::Acquire) == 3 => 130_000_000, _ => return Ok(None) };
        let sequence = self.delivered as u64; self.delivered += 1;
        Ok(Some(CapturedFrame { sequence, captured_at_ns: timestamp, width:64, height:48, stride:256, rgba:vec![sequence as u8; 64*48*4].into_boxed_slice() }))
    }
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(if self.stopping.load(Ordering::Acquire) { 140_000_000 } else if self.delivered == 2 { 130_000_000 } else { 100_000_000 })
    }
}
impl Drop for QpcVideo { fn drop(&mut self) { self.drops.fetch_add(1, Ordering::AcqRel); } }
pub(super) struct QpcAudio {
    pub(super) delivered: usize,
    pub(super) progress: Arc<AtomicUsize>,
    pub(super) drops: Arc<AtomicUsize>,
}
impl RecordingAudioSource for QpcAudio {
    type Error = FixtureError;
    fn capture_next_available(&mut self, _: Duration) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        let Some(&timestamp) = [100_000_000, 113_333_300, 126_666_600].get(self.delivered) else { thread::sleep(Duration::from_millis(2)); return Ok(None); };
        let sequence = self.delivered as u64; self.delivered += 1; self.progress.store(self.delivered, Ordering::Release);
        Ok(Some(CapturedAudioChunk { sequence, captured_at_ns:timestamp, format:AudioFormat::normalized(2), frame_count:640, samples:vec![0.25;1280].into_boxed_slice() }))
    }
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> { Ok(140_000_000) }
}
impl Drop for QpcAudio { fn drop(&mut self) { self.drops.fetch_add(1, Ordering::AcqRel); } }
pub(super) fn wait_qpc(ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() { assert!(Instant::now() < deadline, "QPC 合同必须在预算内完成"); thread::sleep(Duration::from_millis(2)); }
}
pub(super) fn save_qpc_session(name: &str, directory: &std::path::Path, result: &impl std::fmt::Debug, drops: [usize; 2]) -> Value {
    let manifest: Value = serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    if let Some(root) = std::env::var_os("CLIPPY_WASAPI_QPC_EVIDENCE") {
        let root = PathBuf::from(root).join(name); fs::create_dir_all(&root).unwrap(); let mut files = Vec::new();
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap(); if !entry.file_type().unwrap().is_file() { continue; }
            let bytes = fs::read(entry.path()).unwrap(); fs::write(root.join(entry.file_name()), &bytes).unwrap();
            files.push(serde_json::json!({"name":entry.file_name().to_string_lossy(),"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes))}));
        }
        fs::write(root.join("observation.json"),serde_json::to_vec_pretty(&serde_json::json!({"result":format!("{result:?}"),"manifest":manifest,"files":files,"sourceDrops":drops})).unwrap()).unwrap();
    }
    manifest
}

#[test]
fn wasapi_qpc_original_av_session_preserves_overlap_error_and_interrupted_files() {
    let temporary = tempfile::tempdir().unwrap();
    let progress = Arc::new(AtomicUsize::new(0)); let stopping = Arc::new(AtomicBool::new(false));
    let vd = Arc::new(AtomicUsize::new(0)); let ad = Arc::new(AtomicUsize::new(0));
    let video = QpcVideo { delivered:0, audio_progress:Arc::clone(&progress), stopping:Arc::clone(&stopping), drops:Arc::clone(&vd) };
    let audio = QpcAudio { delivered:0, progress, drops:Arc::clone(&ad) };
    let mut configuration = config("qpc-original"); configuration.width=64; configuration.height=48; configuration.frames_per_second=100; configuration.segment_duration_ns=100_000_000;
    let session = AvRecordingSession::start_with_factories(temporary.path(),configuration,move |_| Ok(video),move |_| Ok(audio)).unwrap();
    let directory = session.session_directory().to_path_buf();
    wait_qpc(|| session.audio_capture.as_ref().unwrap().is_finished());
    stopping.store(true,Ordering::Release); let result=session.stop();
    let drops=[vd.load(Ordering::Acquire),ad.load(Ordering::Acquire)];
    let manifest=save_qpc_session("original-av-session",&directory,&result,drops);
    assert!(matches!(result,Err(AvRecordingSessionError::AudioCapture(AudioCaptureWorkerError::Pipeline(AudioPipelineError::PresentationOverlap)))));
    assert_eq!(manifest["state"],"interrupted"); assert_eq!(drops,[1,1]);
}
