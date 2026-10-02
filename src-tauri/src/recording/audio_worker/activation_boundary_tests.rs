use crate::recording::audio::AudioPushOutcome;
use std::sync::atomic::AtomicUsize;

fn activation_observation(name: &str, value: serde_json::Value) {
    if let Some(root) = std::env::var_os("CLIPPY_AUDIO_ACTIVATION_EVIDENCE") {
        let root = std::path::PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn audio_activation_exact_resume_boundary_is_queued() {
    let pipeline = AudioPipeline::new(0);
    pipeline.push(chunk(0, 0, CHUNK_FRAMES)).unwrap();
    pipeline.pause(CHUNK_NS).unwrap();
    pipeline.resume(120_000_000).unwrap();
    let result = pipeline.push(chunk(1, 120_000_000, CHUNK_FRAMES));
    activation_observation(
        "exact-boundary",
        serde_json::json!({"result":format!("{result:?}")}),
    );
    assert_eq!(
        result,
        Ok(AudioPushOutcome::Queued {
            presentation_at_ns: CHUNK_NS,
            duration_ns: CHUNK_NS,
            gap_before_ns: 0,
        })
    );
    assert_eq!(pipeline.finish(140_000_000).unwrap(), 40_000_000);
}

#[test]
fn audio_activation_boundary_is_consumed_once_and_duplicates_still_fail() {
    let pipeline = AudioPipeline::new(0);
    pipeline.push(chunk(0, 0, CHUNK_FRAMES)).unwrap();
    pipeline.pause(CHUNK_NS).unwrap();
    pipeline.resume(120_000_000).unwrap();
    let first = pipeline.push(chunk(1, 120_000_000, CHUNK_FRAMES));
    activation_observation(
        "duplicate-guard",
        serde_json::json!({"first":format!("{first:?}")}),
    );
    first.unwrap();
    assert_eq!(
        pipeline.push(chunk(2, 120_000_000, CHUNK_FRAMES)),
        Err(AudioPipelineError::SourceTimestampNotIncreasing)
    );
    assert_eq!(
        pipeline.push(chunk(1, 140_000_000, CHUNK_FRAMES)),
        Err(AudioPipelineError::SequenceNotIncreasing)
    );
    assert_eq!(pipeline.stats().unwrap().accepted_chunks, 2);
}

#[test]
fn audio_activation_backpressure_preserves_exact_boundary_for_retry() {
    let pipeline = AudioPipeline::new(0);
    for sequence in 0..10 {
        pipeline
            .push(chunk(sequence, sequence * 100_000_000, 4_800))
            .unwrap();
    }
    pipeline.pause(1_000_000_000).unwrap();
    pipeline.resume(2_000_000_000).unwrap();
    let before = pipeline.stats().unwrap();
    let first = pipeline.push(chunk(10, 2_000_000_000, CHUNK_FRAMES));
    activation_observation(
        "backpressure",
        serde_json::json!({"first":format!("{first:?}")}),
    );
    assert_eq!(first, Err(AudioPipelineError::Backpressure));
    assert_eq!(pipeline.stats().unwrap(), before);
    pipeline.pop().unwrap().unwrap();
    assert!(matches!(
        pipeline.push(chunk(10, 2_000_000_000, CHUNK_FRAMES)),
        Ok(AudioPushOutcome::Queued {
            presentation_at_ns: 1_000_000_000,
            ..
        })
    ));
    assert_eq!(pipeline.stats().unwrap().accepted_chunks, 11);
}

struct ActivationSource {
    initial: bool,
    resumed: bool,
    delivered: bool,
    post_timestamp: u64,
    drops: Arc<AtomicUsize>,
    hooks: Arc<Mutex<Vec<&'static str>>>,
}

impl RecordingAudioSource for ActivationSource {
    type Error = FixtureError;
    fn start_capture(&mut self) -> Result<Option<u64>, Self::Error> {
        Ok(Some(0))
    }
    fn capture_next_available(
        &mut self,
        _: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        if !self.initial {
            self.initial = true;
            return Ok(Some(chunk(0, 0, CHUNK_FRAMES)));
        }
        if self.resumed && !self.delivered {
            self.delivered = true;
            return Ok(Some(chunk(1, self.post_timestamp, CHUNK_FRAMES)));
        }
        thread::sleep(Duration::from_millis(2));
        Ok(None)
    }
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(140_000_000)
    }
    fn pause_capture(&mut self) -> Result<u64, Self::Error> {
        self.hooks.lock().unwrap().push("pause");
        Ok(CHUNK_NS)
    }
    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        self.hooks.lock().unwrap().push("resume");
        self.resumed = true;
        Ok(120_000_000)
    }
    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        self.hooks.lock().unwrap().push("stop");
        Ok(140_000_000)
    }
}

impl Drop for ActivationSource {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
    }
}

fn wait_activation(ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < deadline, "音频边界合同必须在预算内完成");
        thread::sleep(Duration::from_millis(2));
    }
}

type ActivationWorker = (
    AudioCaptureWorker,
    Arc<AudioPipeline>,
    Arc<AtomicUsize>,
    Arc<Mutex<Vec<&'static str>>>,
);

fn activation_worker(timestamp: u64) -> ActivationWorker {
    let pipeline = Arc::new(AudioPipeline::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let hooks = Arc::new(Mutex::new(Vec::new()));
    let source = ActivationSource {
        initial: false,
        resumed: false,
        delivered: false,
        post_timestamp: timestamp,
        drops: Arc::clone(&drops),
        hooks: Arc::clone(&hooks),
    };
    let worker = AudioCaptureWorker::spawn_with_factory(
        move |_| Ok(source),
        RecordingSessionClock::new(),
        Arc::clone(&pipeline),
    )
    .unwrap();
    wait_activation(|| pipeline.stats().unwrap().accepted_chunks == 1);
    (worker, pipeline, drops, hooks)
}

#[test]
fn audio_activation_original_worker_accepts_boundary_and_finishes() {
    let (worker, pipeline, drops, hooks) = activation_worker(120_000_000);
    worker.pause().unwrap();
    worker.resume().unwrap();
    wait_activation(|| worker.is_finished() || pipeline.stats().unwrap().accepted_chunks == 2);
    let result = worker.stop();
    activation_observation(
        "worker-boundary",
        serde_json::json!({"result":format!("{result:?}"),"accepted":pipeline.stats().unwrap().accepted_chunks,"drops":drops.load(Ordering::Acquire)}),
    );
    let report = result.unwrap();
    assert_eq!(report.queued_chunks, 2);
    assert_eq!(report.duration_ns, Some(40_000_000));
    assert_eq!(drops.load(Ordering::Acquire), 1);
    assert_eq!(*hooks.lock().unwrap(), ["pause", "resume", "stop"]);
}

#[test]
fn audio_activation_original_worker_rejects_pcm_before_resume() {
    let (worker, pipeline, drops, _) = activation_worker(119_999_999);
    worker.pause().unwrap();
    worker.resume().unwrap();
    wait_activation(|| worker.is_finished());
    let result = worker.wait();
    assert_eq!(
        result,
        Err(AudioCaptureWorkerError::Pipeline(
            AudioPipelineError::SourceTimestampNotIncreasing
        ))
    );
    assert_eq!(pipeline.stats().unwrap().accepted_chunks, 1);
    assert_eq!(drops.load(Ordering::Acquire), 1);
    activation_observation(
        "stale-boundary",
        serde_json::json!({"result":format!("{result:?}"),"accepted":1,"drops":1}),
    );
}
