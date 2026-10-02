#[test]
fn wasapi_qpc_packet_guard_accepts_one_tick_and_rejects_real_overlap() {
    assert_eq!(WASAPI_TIMESTAMP_PRECISION,AudioTimestampPrecision::HundredNanoseconds);
    for overlap in [0,1,33,99,100] { assert!(packet_timestamp_is_valid(20_000_000-overlap,Some(20_000_000))); }
    assert!(!packet_timestamp_is_valid(19_999_899,Some(20_000_000)));
    assert!(!packet_timestamp_is_valid(0,Some(13_333_333)));
    assert!(packet_timestamp_is_valid(20_000_100,Some(20_000_000)));
    assert!(packet_timestamp_is_valid(0,None));
    let first=packet_to_chunks(0,0,640,Some(&vec![0.25;1280])).unwrap();
    let second=packet_to_chunks(first.next_sequence,13_333_300,640,None).unwrap();
    assert!(packet_timestamp_is_valid(second.chunks[0].captured_at_ns,Some(first.end_ns)));
    assert_eq!(second.chunks[0].captured_at_ns,13_333_300); assert_eq!(second.end_ns,26_666_633);
    assert_eq!(second.chunks[0].samples.as_ref(),vec![0.0;1280]);
}

use crate::recording::audio::{AudioPipeline,AudioPipelineDrain,AudioPipelineError};
use crate::recording::audio_worker::{AudioCaptureWorker,AudioCaptureWorkerError,RecordingAudioSource};
use crate::recording::clock::RecordingSessionClock;
use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
use std::time::{Duration,Instant};
use std::thread;

struct PrecisionSource { chunks:VecDeque<CapturedAudioChunk>, drops:Arc<AtomicUsize>, starts:Arc<AtomicUsize> }
impl RecordingAudioSource for PrecisionSource {
    type Error=std::io::Error;
    fn timestamp_precision(&self)->AudioTimestampPrecision { WASAPI_TIMESTAMP_PRECISION }
    fn start_capture(&mut self)->Result<Option<u64>,Self::Error> { self.starts.fetch_add(1,Ordering::AcqRel); Ok(None) }
    fn capture_next_available(&mut self,_:Duration)->Result<Option<CapturedAudioChunk>,Self::Error> { if self.chunks.is_empty() { thread::sleep(Duration::from_millis(2)); } Ok(self.chunks.pop_front()) }
    fn control_timestamp_ns(&mut self)->Result<u64,Self::Error> { Ok(26_666_633) }
}
impl Drop for PrecisionSource { fn drop(&mut self) { self.drops.fetch_add(1,Ordering::AcqRel); } }
fn source(drops:Arc<AtomicUsize>,starts:Arc<AtomicUsize>)->PrecisionSource {
    let mut chunks=VecDeque::new();
    for (sequence,timestamp) in [0,13_333_300].into_iter().enumerate() {
        chunks.extend(packet_to_chunks(sequence as u64,timestamp,640,Some(&vec![0.25;1280])).unwrap().chunks);
    }
    PrecisionSource { chunks,drops,starts }
}
fn wait_qpc_worker(ready:impl Fn()->bool) { let deadline=Instant::now()+Duration::from_secs(5); while !ready() { assert!(Instant::now()<deadline); thread::sleep(Duration::from_millis(2)); } }

#[test]
fn wasapi_qpc_precision_source_runs_original_worker_and_finishes_all_pcm() {
    let drops=Arc::new(AtomicUsize::new(0)); let starts=Arc::new(AtomicUsize::new(0));
    let source=source(Arc::clone(&drops),Arc::clone(&starts)); let pipeline=Arc::new(AudioPipeline::new(0));
    let worker=AudioCaptureWorker::spawn_with_factory(move |_| Ok(source),RecordingSessionClock::new(),Arc::clone(&pipeline)).unwrap();
    wait_qpc_worker(|| worker.is_finished()||pipeline.stats().unwrap().accepted_chunks==2);
    let report=worker.stop().unwrap(); assert_eq!(report.captured_frames,1280); assert_eq!(report.queued_chunks,2); assert_eq!(report.duration_ns,Some(26_666_666));
    for timestamp in [0,13_333_300] { let chunk=pipeline.pop().unwrap().unwrap(); assert_eq!(chunk.chunk.captured_at_ns,timestamp); assert_eq!(chunk.chunk.samples.as_ref(),vec![0.25;1280]); }
    assert!(matches!(pipeline.pop_wait(),Ok(AudioPipelineDrain::Finished { duration_ns:26_666_666 })));
    assert_eq!(drops.load(Ordering::Acquire),1); assert_eq!(starts.load(Ordering::Acquire),1);
}

#[test]
fn wasapi_qpc_late_precision_change_aborts_before_start_and_keeps_prefix() {
    let drops=Arc::new(AtomicUsize::new(0)); let starts=Arc::new(AtomicUsize::new(0));
    let source=source(Arc::clone(&drops),Arc::clone(&starts)); let pipeline=Arc::new(AudioPipeline::new(0));
    let packet=packet_to_chunks(0,0,640,None).unwrap().chunks.pop_front().unwrap(); pipeline.push(packet).unwrap();
    let worker=AudioCaptureWorker::spawn_with_factory(move |_| Ok(source),RecordingSessionClock::new(),Arc::clone(&pipeline)).unwrap();
    wait_qpc_worker(|| worker.is_finished());
    assert_eq!(worker.wait(),Err(AudioCaptureWorkerError::Pipeline(AudioPipelineError::TimestampPrecisionAlreadyStarted)));
    assert_eq!(starts.load(Ordering::Acquire),0); assert_eq!(drops.load(Ordering::Acquire),1);
    assert_eq!(pipeline.pop().unwrap().unwrap().chunk.frame_count,640); assert!(matches!(pipeline.pop_wait(),Err(AudioPipelineError::Aborted)));
}
