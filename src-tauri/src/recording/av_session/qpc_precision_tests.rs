use super::qpc_precision_diagnostic_tests::*;
use crate::recording::audio::AudioTimestampPrecision;
use std::sync::atomic::{AtomicBool,AtomicUsize,Ordering};

struct PrecisionAudio(QpcAudio);
impl RecordingAudioSource for PrecisionAudio {
    type Error=FixtureError;
    fn timestamp_precision(&self)->AudioTimestampPrecision { AudioTimestampPrecision::HundredNanoseconds }
    fn capture_next_available(&mut self,timeout:Duration)->Result<Option<CapturedAudioChunk>,Self::Error> { self.0.capture_next_available(timeout) }
    fn control_timestamp_ns(&mut self)->Result<u64,Self::Error> { self.0.control_timestamp_ns() }
}

#[test]
fn wasapi_qpc_precision_source_commits_complete_av_with_all_pcm() {
    let temporary=tempfile::tempdir().unwrap(); let progress=Arc::new(AtomicUsize::new(0)); let stopping=Arc::new(AtomicBool::new(false));
    let vd=Arc::new(AtomicUsize::new(0)); let ad=Arc::new(AtomicUsize::new(0));
    let video=QpcVideo { delivered:0,audio_progress:Arc::clone(&progress),stopping:Arc::clone(&stopping),drops:Arc::clone(&vd) };
    let audio=PrecisionAudio(QpcAudio { delivered:0,progress,drops:Arc::clone(&ad) });
    let mut configuration=config("qpc-precision"); configuration.width=64; configuration.height=48; configuration.frames_per_second=100; configuration.segment_duration_ns=100_000_000;
    let session=AvRecordingSession::start_with_factories(temporary.path(),configuration,move |_| Ok(video),move |_| Ok(audio)).unwrap();
    let directory=session.session_directory().to_path_buf();
    wait_qpc(|| session.audio_capture.as_ref().unwrap().is_finished()||(session.audio_pipeline.stats().unwrap().accepted_chunks==3&&session.video_pipeline.stats().unwrap().accepted_frames==2));
    stopping.store(true,Ordering::Release); let result=session.stop();
    let drops=[vd.load(Ordering::Acquire),ad.load(Ordering::Acquire)]; let manifest=save_qpc_session("precision-av-session",&directory,&result,drops);
    let report=result.unwrap(); assert_eq!(manifest["state"],"complete"); assert_eq!(report.duration_ns,40_000_000); assert_eq!(report.audio_pcm_frames,1920); assert_eq!(report.audio_accepted_chunks,3); assert_eq!(report.video_encoded_frames,4); assert_eq!(drops,[1,1]); assert!(report.final_output_path.unwrap().is_file());
}
