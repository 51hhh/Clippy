use crate::recording::audio::AudioPipeline;
use crate::recording::audio_worker::{AudioCaptureWorker, RecordingAudioSource};
use crate::recording::clock::RecordingSessionClock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn audio_activation_control_clock_orders_packet_end_clamps() {
    let mut clock = WasapiControlTimeline::default();
    assert_eq!(
        clock.next_timestamp(20_000_000, Some(40_000_000)),
        Ok(40_000_000)
    );
    assert_eq!(
        clock.next_timestamp(30_000_000, Some(40_000_000)),
        Ok(40_000_001)
    );
    assert_eq!(
        clock.next_timestamp(25_000_000, Some(40_000_000)),
        Ok(40_000_002)
    );
}

#[test]
fn audio_activation_control_clock_keeps_genuinely_later_time() {
    let mut clock = WasapiControlTimeline::default();
    assert_eq!(clock.next_timestamp(100, None), Ok(100));
    assert_eq!(clock.next_timestamp(200, Some(150)), Ok(200));
    assert_eq!(clock.next_timestamp(250, Some(300)), Ok(300));
}

#[test]
fn audio_activation_control_clock_never_repeats_across_cycles() {
    let mut clock = WasapiControlTimeline::default();
    for sequence in 0..200 {
        assert_eq!(clock.next_timestamp(0, Some(400)), Ok(400 + sequence));
    }
    assert_eq!(clock.next_timestamp(800, Some(600)), Ok(800));
}

#[test]
fn audio_activation_control_clock_overflow_preserves_state() {
    let mut clock = WasapiControlTimeline::default();
    assert_eq!(clock.next_timestamp(u64::MAX, None), Ok(u64::MAX));
    for _ in 0..2 {
        assert_eq!(
            clock.next_timestamp(0, None),
            Err(WindowsAudioContractError::ControlTimestampExhausted)
        );
        assert_eq!(clock.last_control_ns, Some(u64::MAX));
    }
}

struct StatefulClampSource {
    control: WasapiControlTimeline,
    drops: Arc<AtomicUsize>,
}
impl RecordingAudioSource for StatefulClampSource {
    type Error = WindowsAudioContractError;
    fn capture_next_available(
        &mut self,
        _: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        std::thread::sleep(Duration::from_millis(2));
        Ok(None)
    }
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        self.control.next_timestamp(20_000_000, Some(40_000_000))
    }
    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        self.control.next_timestamp(30_000_000, Some(40_000_000))
    }
    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        self.control.next_timestamp(80_000_000, Some(40_000_000))
    }
}
impl Drop for StatefulClampSource {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
    }
}

#[test]
fn audio_activation_control_clock_original_worker_resumes_clamped_source() {
    let pipeline = Arc::new(AudioPipeline::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let source = StatefulClampSource {
        control: WasapiControlTimeline::default(),
        drops: Arc::clone(&drops),
    };
    let worker = AudioCaptureWorker::spawn_with_factory(
        move |_| Ok(source),
        RecordingSessionClock::new(),
        Arc::clone(&pipeline),
    )
    .unwrap();
    worker.pause().unwrap();
    worker.resume().unwrap();
    let report = worker.stop().unwrap();
    assert_eq!(report.duration_ns, Some(79_999_999));
    assert_eq!(drops.load(Ordering::Acquire), 1);
}

struct ExhaustedControlSource {
    control: WasapiControlTimeline,
    delivered: bool,
    drops: Arc<AtomicUsize>,
}

impl RecordingAudioSource for ExhaustedControlSource {
    type Error = WindowsAudioContractError;
    fn capture_next_available(
        &mut self,
        _: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        if !self.delivered {
            self.delivered = true;
            return Ok(packet_to_chunks(0, 0, 960, None)?.chunks.pop_front());
        }
        std::thread::sleep(Duration::from_millis(2));
        Ok(None)
    }
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        self.control.next_timestamp(u64::MAX, Some(20_000_000))
    }
    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        self.control.next_timestamp(0, Some(20_000_000))
    }
}

impl Drop for ExhaustedControlSource {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
    }
}

#[test]
fn audio_activation_control_clock_error_aborts_worker_preserving_prefix() {
    use crate::recording::audio::AudioPipelineError;
    use crate::recording::audio_worker::AudioCaptureWorkerError;
    let pipeline = Arc::new(AudioPipeline::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let source = ExhaustedControlSource {
        control: WasapiControlTimeline::default(),
        delivered: false,
        drops: Arc::clone(&drops),
    };
    let worker = AudioCaptureWorker::spawn_with_factory(
        move |_| Ok(source),
        RecordingSessionClock::new(),
        Arc::clone(&pipeline),
    )
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while pipeline.stats().unwrap().accepted_chunks != 1 {
        assert!(
            std::time::Instant::now() < deadline,
            "溢出前必须交付真实 PCM 前缀"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    worker.pause().unwrap();
    let expected = AudioCaptureWorkerError::Source(
        WindowsAudioContractError::ControlTimestampExhausted.to_string(),
    );
    assert_eq!(worker.resume(), Err(expected.clone()));
    assert_eq!(worker.wait(), Err(expected));
    assert_eq!(drops.load(Ordering::Acquire), 1);
    assert_eq!(pipeline.pop().unwrap().unwrap().chunk.frame_count, 960);
    assert_eq!(
        pipeline.push(
            packet_to_chunks(1, 20_000_000, 960, None)
                .unwrap()
                .chunks
                .pop_front()
                .unwrap()
        ),
        Err(AudioPipelineError::Aborted)
    );
}
