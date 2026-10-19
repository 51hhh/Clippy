use crate::recording::audio_mixer::MixedAudioSource;
use crate::recording::audio_worker::RecordingAudioSource;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, thiserror::Error)]
#[error("control skew fixture")]
struct SkewError;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SkewMode { Resume, StopTail }

#[derive(Default)]
struct SkewProbe { drops:AtomicUsize, started:AtomicUsize }
impl SkewProbe { fn load(&self,order:Ordering)->usize {self.drops.load(order)} }

fn skew_tone(frame: usize) -> f32 {
    (std::f32::consts::TAU * (frame % 48) as f32 / 48.0).sin()
}

fn skew_packet(sequence: u64, timestamp: u64, gain: f32) -> CapturedAudioChunk {
    CapturedAudioChunk { sequence, captured_at_ns: timestamp, format: AudioFormat::normalized(2),
        frame_count: 960, samples: (0..1920).map(|sample| gain * skew_tone(sample / 2)).collect::<Vec<_>>().into_boxed_slice() }
}

struct SkewInput {
    mode: SkewMode,
    gain: f32,
    resume_at: u64,
    stop_at: u64,
    chunks: VecDeque<CapturedAudioChunk>,
    tails: Vec<CapturedAudioChunk>,
    drops: Arc<SkewProbe>,
}

impl RecordingAudioSource for SkewInput {
    type Error = SkewError;
    fn start_capture(&mut self) -> Result<Option<u64>, Self::Error> { self.drops.started.fetch_add(1,Ordering::AcqRel);Ok(Some(0)) }
    fn capture_next_available(&mut self, timeout: Duration) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        if let Some(chunk) = self.chunks.pop_front() { return Ok(Some(chunk)); }
        std::thread::sleep(timeout.min(Duration::from_millis(1))); Ok(None)
    }
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> { Ok(self.stop_at) }
    fn pause_capture(&mut self) -> Result<u64, Self::Error> { self.chunks.clear(); Ok(20_000_000) }
    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        assert!(self.mode == SkewMode::Resume);
        self.chunks.push_back(skew_packet(1,self.resume_at,self.gain));
        if self.resume_at == 1_000_000_000 { self.chunks.push_back(skew_packet(2,1_020_000_000,self.gain)); }
        self.stop_at = 1_040_000_000; Ok(self.resume_at)
    }
    fn stop_capture(&mut self) -> Result<u64, Self::Error> { Ok(self.stop_at) }
    fn take_stopped_chunks(&mut self) -> Result<Vec<CapturedAudioChunk>, Self::Error> { Ok(std::mem::take(&mut self.tails)) }
}
impl Drop for SkewInput { fn drop(&mut self) { self.drops.drops.fetch_add(1,Ordering::AcqRel); } }

fn skew_source(mode: SkewMode, system_is_earlier: bool) -> (MixedAudioSource<SkewInput,SkewInput>, Arc<SkewProbe>) {
    let drops = Arc::new(SkewProbe::default());
    let input = |gain, is_earlier| {
        let duration = if is_earlier { 20_000_000 } else { 40_000_000 };
        let mut tails = Vec::new();
        if mode == SkewMode::StopTail {
            tails.push(skew_packet(0,0,gain));
            if !is_earlier { tails.push(skew_packet(1,20_000_000,gain)); }
        }
        SkewInput { mode,gain,resume_at:if is_earlier {1_000_000_000} else {1_020_000_000},
            stop_at:if mode==SkewMode::Resume {20_000_000} else {duration},
            chunks:if mode==SkewMode::Resume {VecDeque::from([skew_packet(0,0,gain)])} else {VecDeque::new()},
            tails,drops:Arc::clone(&drops) }
    };
    (MixedAudioSource::new(input(0.8,system_is_earlier),input(0.2,!system_is_earlier)),drops)
}

fn skew_observation(name: &str, value: serde_json::Value) {
    if let Some(root)=std::env::var_os("CLIPPY_MIXED_SKEW_EVIDENCE") {
        let root=std::path::PathBuf::from(root); std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(format!("{name}.json")),serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
}

fn wait_skew(ready: impl Fn()->bool) {
    let deadline=std::time::Instant::now()+Duration::from_secs(5);
    while !ready() { assert!(std::time::Instant::now()<deadline,"双源时刻差异合同必须在预算内完成"); std::thread::sleep(Duration::from_millis(2)); }
}
