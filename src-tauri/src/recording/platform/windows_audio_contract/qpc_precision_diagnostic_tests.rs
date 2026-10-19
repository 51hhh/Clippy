use crate::recording::audio::{AudioPipeline, AudioPipelineDrain, AudioPipelineError};
use crate::recording::audio_worker::{AudioCaptureWorker, AudioCaptureWorkerError, RecordingAudioSource};
use crate::recording::clock::RecordingSessionClock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

fn mapped_packets() -> VecDeque<CapturedAudioChunk> {
    // 构造一种与 100 ns 输出精度相容的序列；不声称是真机采用的舍入方式。
    let mapper = QpcClockMapper::from_calibration(0, 10_000_000, 0, 0).unwrap();
    let mut chunks = VecDeque::new();
    for (sequence, qpc) in [0, 133_333].into_iter().enumerate() {
        let samples = vec![0.25; 1280];
        let packet = packet_to_chunks(sequence as u64, mapper.map_100ns(qpc).unwrap(), 640, Some(&samples)).unwrap();
        chunks.extend(packet.chunks);
    }
    chunks
}

#[test]
fn wasapi_qpc_original_pipeline_diagnoses_quantized_contiguous_overlap() {
    let pipeline = AudioPipeline::new(0);
    let mut packets = mapped_packets();
    let first = packets.pop_front().unwrap();
    let second = packets.pop_front().unwrap();
    assert_eq!(first.frame_count, 640);
    assert_eq!(second.captured_at_ns, 13_333_300);
    pipeline.push(first).unwrap();
    assert_eq!(pipeline.push(second), Err(AudioPipelineError::PresentationOverlap));
    assert_eq!(pipeline.stats().unwrap().accepted_chunks, 1);
    let first = pipeline.pop().unwrap().unwrap();
    assert_eq!(first.duration_ns, 13_333_333);
    assert_eq!(first.chunk.samples.as_ref(), vec![0.25; 1280]);
}

struct OriginalQpcSource {
    chunks: VecDeque<CapturedAudioChunk>,
    drops: Arc<AtomicUsize>,
}
impl RecordingAudioSource for OriginalQpcSource {
    type Error = std::io::Error;
    fn capture_next_available(&mut self, _: Duration) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        Ok(self.chunks.pop_front())
    }
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(26_666_633)
    }
}
impl Drop for OriginalQpcSource {
    fn drop(&mut self) { self.drops.fetch_add(1, Ordering::AcqRel); }
}

#[test]
fn wasapi_qpc_original_worker_preserves_root_error_and_pcm_prefix() {
    let drops = Arc::new(AtomicUsize::new(0));
    let source = OriginalQpcSource { chunks: mapped_packets(), drops: Arc::clone(&drops) };
    let pipeline = Arc::new(AudioPipeline::new(0));
    let worker = AudioCaptureWorker::spawn_with_factory(move |_| Ok(source), RecordingSessionClock::new(), Arc::clone(&pipeline)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !worker.is_finished() { assert!(Instant::now() < deadline); thread::sleep(Duration::from_millis(2)); }
    let result = worker.wait();
    assert_eq!(result, Err(AudioCaptureWorkerError::Pipeline(AudioPipelineError::PresentationOverlap)));
    assert_eq!(drops.load(Ordering::Acquire), 1);
    match pipeline.pop_wait().unwrap() {
        AudioPipelineDrain::Chunk(chunk) => { assert_eq!(chunk.chunk.frame_count, 640); assert_eq!(chunk.chunk.samples.as_ref(), vec![0.25; 1280]); }
        _ => panic!("中止后必须先交出已接受 PCM"),
    }
    assert!(matches!(pipeline.pop_wait(), Err(AudioPipelineError::Aborted)));
    if let Some(root) = std::env::var_os("CLIPPY_WASAPI_QPC_EVIDENCE") {
        let root = std::path::PathBuf::from(root); std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("original-worker.json"), serde_json::to_vec_pretty(&serde_json::json!({"result":format!("{result:?}"),"acceptedChunks":1,"prefixPcmFrames":640,"sourceDrops":1})).unwrap()).unwrap();
    }
}
