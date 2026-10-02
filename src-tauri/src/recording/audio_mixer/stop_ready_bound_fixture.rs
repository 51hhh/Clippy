use crate::recording::audio::{AudioFormat, CapturedAudioChunk};
use crate::recording::audio_mixer::MixedAudioSource;
use crate::recording::audio_worker::RecordingAudioSource;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct StopBoundError(&'static str);

#[derive(Default)]
struct StopBoundProbe {
    drops: AtomicUsize,
    stop_attempts: AtomicUsize,
}

fn stop_bound_tone(frame: usize) -> f32 {
    (std::f32::consts::TAU * (frame % 48) as f32 / 48.0).sin()
}

fn stop_bound_packet(sequence: u64, at: u64, gain: f32) -> CapturedAudioChunk {
    CapturedAudioChunk {
        sequence,
        captured_at_ns: at,
        format: AudioFormat::normalized(2),
        frame_count: 960,
        samples: (0..1920)
            .map(|n| gain * stop_bound_tone(n / 2))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    }
}

struct StopBoundInput {
    initial: Option<CapturedAudioChunk>,
    tails: Vec<CapturedAudioChunk>,
    stop_at: u64,
    fail_stop: bool,
    probe: Arc<StopBoundProbe>,
}

impl RecordingAudioSource for StopBoundInput {
    type Error = StopBoundError;
    fn start_capture(&mut self) -> Result<Option<u64>, Self::Error> {
        Ok(Some(0))
    }
    fn capture_next_available(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        if self.initial.is_some() {
            return Ok(self.initial.take());
        }
        std::thread::sleep(timeout.min(Duration::from_millis(1)));
        Ok(None)
    }
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(20_000_000)
    }
    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        self.probe.stop_attempts.fetch_add(1, Ordering::AcqRel);
        if self.fail_stop {
            Err(StopBoundError("microphone stop failed"))
        } else {
            Ok(self.stop_at)
        }
    }
    fn take_stopped_chunks(&mut self) -> Result<Vec<CapturedAudioChunk>, Self::Error> {
        Ok(std::mem::take(&mut self.tails))
    }
}
impl Drop for StopBoundInput {
    fn drop(&mut self) {
        self.probe.drops.fetch_add(1, Ordering::AcqRel);
    }
}

fn stop_bound_source(
    initial: bool,
    system_earlier: bool,
    gap: u64,
) -> (
    MixedAudioSource<StopBoundInput, StopBoundInput>,
    Arc<StopBoundProbe>,
) {
    let probe = Arc::new(StopBoundProbe::default());
    let input = |earlier, gain| StopBoundInput {
        initial: initial.then(|| stop_bound_packet(0, 0, gain)),
        tails: vec![stop_bound_packet(
            u64::from(initial),
            gap + if earlier { 0 } else { 20_000_000 },
            gain,
        )],
        stop_at: gap + if earlier { 20_000_000 } else { 40_000_000 },
        fail_stop: false,
        probe: Arc::clone(&probe),
    };
    (
        MixedAudioSource::new(input(system_earlier, 0.8), input(!system_earlier, 0.2)),
        probe,
    )
}

// 已有take_stopped_chunks API的有限批夹具：原worker只读一次会漏掉第二批或其真实错误。
struct StopBoundBatches {
    inner: MixedAudioSource<StopBoundInput, StopBoundInput>,
    pending: VecDeque<CapturedAudioChunk>,
    loaded: bool,
    reads: usize,
    fail_second: bool,
}
impl RecordingAudioSource for StopBoundBatches {
    type Error = StopBoundError;
    fn start_capture(&mut self) -> Result<Option<u64>, Self::Error> {
        self.inner
            .start_capture()
            .map_err(|_| StopBoundError("start"))
    }
    fn capture_next_available(
        &mut self,
        t: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        self.inner
            .capture_next_available(t)
            .map_err(|_| StopBoundError("capture"))
    }
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        self.inner
            .control_timestamp_ns()
            .map_err(|_| StopBoundError("control"))
    }
    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        self.inner
            .stop_capture()
            .map_err(|_| StopBoundError("stop"))
    }
    fn take_stopped_chunks(&mut self) -> Result<Vec<CapturedAudioChunk>, Self::Error> {
        self.reads += 1;
        if self.reads == 2 && self.fail_second {
            return Err(StopBoundError("second finite tail batch failed"));
        }
        if !self.loaded {
            self.pending.extend(
                self.inner
                    .take_stopped_chunks()
                    .map_err(|_| StopBoundError("tail"))?,
            );
            self.loaded = true;
        }
        Ok(self.pending.pop_front().into_iter().collect())
    }
}
fn stop_bound_batches(fail_second: bool) -> (StopBoundBatches, Arc<StopBoundProbe>) {
    let (inner, probe) = stop_bound_source(true, true, 20_000_000);
    (
        StopBoundBatches {
            inner,
            pending: VecDeque::new(),
            loaded: false,
            reads: 0,
            fail_second,
        },
        probe,
    )
}
fn wait_stop_bound(ready: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(std::time::Instant::now() < deadline, "停止输出合同超过预算");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn stop_bound_observation(name: &str, value: serde_json::Value) {
    if let Some(root) = std::env::var_os("CLIPPY_MIXED_STOP_BOUND_EVIDENCE") {
        let root = std::path::PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
}
