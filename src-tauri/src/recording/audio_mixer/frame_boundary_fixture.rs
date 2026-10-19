use crate::recording::audio_mixer::MixedAudioSource;
use crate::recording::audio_worker::RecordingAudioSource;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[derive(Debug, thiserror::Error)]
#[error("boundary fixture")]
struct BoundaryError;

struct BoundaryInput {
    origin: u64,
    packet_at: u64,
    frames: u32,
    stop_at: u64,
    pause_at: u64,
    resume_at: u64,
    control: Arc<AtomicU64>,
    drops: Arc<AtomicUsize>,
    delivered: bool,
    resumed: bool,
    tail_only: bool,
    sequence: u64,
}

impl BoundaryInput {
    fn new(origin: u64, packet_at: u64, frames: u32, stop_at: u64, drops: Arc<AtomicUsize>) -> Self {
        Self { origin, packet_at, frames, stop_at, pause_at: stop_at, resume_at: 1_000_005_000,
            control: Arc::new(AtomicU64::new(stop_at)), drops, delivered: false, resumed: false,
            tail_only: false, sequence: 0 }
    }
    fn packet(&mut self) -> CapturedAudioChunk {
        let packet = CapturedAudioChunk { sequence: self.sequence, captured_at_ns: self.packet_at,
            format: AudioFormat::normalized(2), frame_count: self.frames,
            samples: vec![0.5; self.frames as usize * 2].into_boxed_slice() };
        self.sequence += 1;
        self.delivered = true;
        packet
    }
}
impl RecordingAudioSource for BoundaryInput {
    type Error = BoundaryError;
    fn start_capture(&mut self) -> Result<Option<u64>, Self::Error> { Ok(Some(self.origin)) }
    fn capture_next_available(&mut self, timeout: Duration) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        if !self.delivered && !self.tail_only { return Ok(Some(self.packet())); }
        std::thread::sleep(timeout.min(Duration::from_millis(1)));
        Ok(None)
    }
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> { Ok(self.control.load(Ordering::Acquire)) }
    fn pause_capture(&mut self) -> Result<u64, Self::Error> { Ok(self.pause_at) }
    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        self.resumed = true; self.delivered = false; self.packet_at = self.resume_at;
        self.stop_at = self.resume_at + u64::from(self.frames) * 1_000_000_000 / 48_000;
        self.control.store(self.stop_at, Ordering::Release);
        Ok(self.resume_at)
    }
    fn stop_capture(&mut self) -> Result<u64, Self::Error> { Ok(self.stop_at) }
    fn take_stopped_chunks(&mut self) -> Result<Vec<CapturedAudioChunk>, Self::Error> {
        Ok(if self.tail_only && !self.delivered { vec![self.packet()] } else { Vec::new() })
    }
}
impl Drop for BoundaryInput { fn drop(&mut self) { self.drops.fetch_add(1, Ordering::AcqRel); } }

fn boundary_source(origin: u64, packet_at: u64, frames: u32, stop_at: u64, tail_only: bool)
    -> (MixedAudioSource<BoundaryInput, BoundaryInput>, Arc<AtomicUsize>) {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut system = BoundaryInput::new(origin, packet_at, frames, stop_at, Arc::clone(&drops));
    let mut microphone = BoundaryInput::new(origin, packet_at, frames, stop_at, Arc::clone(&drops));
    system.tail_only = tail_only; microphone.tail_only = tail_only;
    (MixedAudioSource::new(system, microphone), drops)
}

fn boundary_observation(name: &str, value: serde_json::Value) {
    if let Some(root) = std::env::var_os("CLIPPY_MIXED_FRAME_EVIDENCE") {
        let root = PathBuf::from(root); std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(format!("{name}.json")), serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
}
