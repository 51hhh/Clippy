use crate::recording::audio::{AudioPipeline, AudioPipelineError};
use crate::recording::audio_worker::{
    AudioCaptureWorker, AudioCaptureWorkerError, RecordingAudioSource,
};
use crate::recording::clock::RecordingSessionClock;
use std::sync::Arc;
use std::time::Duration;

// 旧纯函数仍提供末尾下界，不承诺不同调用严格递增；原 worker 展示该组合为什么需要有状态保护。
struct StatelessClampSource;
impl RecordingAudioSource for StatelessClampSource {
    type Error = std::io::Error;
    fn capture_next_available(
        &mut self,
        _: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        std::thread::sleep(Duration::from_millis(2));
        Ok(None)
    }
    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(safe_control_timestamp(20_000_000, Some(40_000_000)))
    }
    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        Ok(safe_control_timestamp(30_000_000, Some(40_000_000)))
    }
}
#[test]
fn audio_activation_stateless_clamp_preserves_invalid_resume_diagnostic() {
    let pipeline = Arc::new(AudioPipeline::new(0));
    let worker = AudioCaptureWorker::spawn_with_factory(
        |_| Ok(StatelessClampSource),
        RecordingSessionClock::new(),
        Arc::clone(&pipeline),
    )
    .unwrap();
    worker.pause().unwrap();
    let result = worker.resume();
    let expected = AudioCaptureWorkerError::Pipeline(AudioPipelineError::InvalidResumeTimestamp);
    if let Some(root) = std::env::var_os("CLIPPY_AUDIO_ACTIVATION_EVIDENCE") {
        let root = std::path::PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("stateless-clamp.json"),serde_json::to_vec_pretty(&serde_json::json!({"pause":40_000_000,"resume":40_000_000,"result":format!("{result:?}")})).unwrap()).unwrap();
    }
    assert_eq!(result, Err(expected.clone()));
    assert_eq!(worker.wait(), Err(expected));
}
