use crate::recording::audio::{AudioFormat, CapturedAudioChunk};
use crate::recording::audio_mixer::MixedAudioSource;
use crate::recording::audio_worker::RecordingAudioSource;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct PausedStopError(&'static str);

#[derive(Default)]
struct PausedStopProbe {
    started: AtomicUsize,
    drops: AtomicUsize,
    stop_attempts: AtomicUsize,
}

fn paused_stop_tone(frame: usize) -> f32 {
    (std::f32::consts::TAU * (frame % 48) as f32 / 48.0).sin()
}

fn paused_stop_packet(sequence: u64, timestamp: u64, gain: f32) -> CapturedAudioChunk {
    CapturedAudioChunk {
        sequence,
        captured_at_ns: timestamp,
        format: AudioFormat::normalized(2),
        frame_count: 960,
        samples: (0..1920)
            .map(|sample| gain * paused_stop_tone(sample / 2))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    }
}

struct PausedStopInput {
    system: bool,
    stop_at: u64,
    chunks: VecDeque<CapturedAudioChunk>,
    tails: Vec<CapturedAudioChunk>,
    fail_stop: bool,
    unexpected_tail: bool,
    invalid_tail: bool,
    probe: Arc<PausedStopProbe>,
}

impl RecordingAudioSource for PausedStopInput {
    type Error = PausedStopError;

    fn start_capture(&mut self) -> Result<Option<u64>, Self::Error> {
        self.probe.started.fetch_add(1, Ordering::AcqRel);
        Ok(Some(0))
    }

    fn capture_next_available(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        if let Some(chunk) = self.chunks.pop_front() {
            return Ok(Some(chunk));
        }
        std::thread::sleep(timeout.min(Duration::from_millis(1)));
        Ok(None)
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(20_000_000)
    }

    fn pause_capture(&mut self) -> Result<u64, Self::Error> {
        // 模拟原WASAPI成功暂停：Stop/Reset后丢弃尚未提交的packet。
        self.chunks.clear();
        self.tails.clear();
        Ok(20_000_000)
    }

    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        self.probe.stop_attempts.fetch_add(1, Ordering::AcqRel);
        if self.fail_stop {
            return Err(PausedStopError(if self.system {
                "system stop failed"
            } else {
                "microphone stop failed"
            }));
        }
        if self.unexpected_tail || self.invalid_tail {
            let mut tail = paused_stop_packet(1, 40_000_000, 0.6);
            if self.invalid_tail {
                tail.format.sample_rate_hz = 44_100;
            }
            self.tails.push(tail);
        }
        Ok(self.stop_at)
    }

    fn take_stopped_chunks(&mut self) -> Result<Vec<CapturedAudioChunk>, Self::Error> {
        Ok(std::mem::take(&mut self.tails))
    }
}

impl Drop for PausedStopInput {
    fn drop(&mut self) {
        self.probe.drops.fetch_add(1, Ordering::AcqRel);
    }
}

fn paused_stop_source(
    initial: bool,
    system_earlier: bool,
) -> (
    MixedAudioSource<PausedStopInput, PausedStopInput>,
    Arc<PausedStopProbe>,
) {
    paused_stop_source_with_failure(initial, system_earlier, false)
}

fn paused_stop_source_with_failure(
    initial: bool,
    system_earlier: bool,
    microphone_failure: bool,
) -> (
    MixedAudioSource<PausedStopInput, PausedStopInput>,
    Arc<PausedStopProbe>,
) {
    let probe = Arc::new(PausedStopProbe::default());
    let input = |system, earlier, gain| PausedStopInput {
        system,
        stop_at: if earlier {
            2_000_000_000
        } else {
            2_040_000_000
        },
        chunks: if initial {
            VecDeque::from([paused_stop_packet(0, 0, gain)])
        } else {
            VecDeque::new()
        },
        tails: Vec::new(),
        fail_stop: !system && microphone_failure,
        unexpected_tail: false,
        invalid_tail: false,
        probe: Arc::clone(&probe),
    };
    (
        MixedAudioSource::new(
            input(true, system_earlier, 0.8),
            input(false, !system_earlier, 0.2),
        ),
        probe,
    )
}

fn wait_paused_stop(ready: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(
            std::time::Instant::now() < deadline,
            "暂停停止合同必须在预算内完成"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn paused_stop_observation(name: &str, value: serde_json::Value) {
    if let Some(root) = std::env::var_os("CLIPPY_MIXED_PAUSED_STOP_EVIDENCE") {
        let root = std::path::PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
}
