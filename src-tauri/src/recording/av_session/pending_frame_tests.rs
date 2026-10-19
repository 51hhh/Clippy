use super::*;
use crate::recording::platform::windows::pending_frame_fixture::pending_wgc_source;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
include!("../idle_frontier_read_tests.rs");

struct RealtimePcm {
    clock: RecordingSessionClock,
    first_origin: Arc<AtomicU64>,
    next: u64,
    count: u64,
    finished: Option<mpsc::Sender<()>>,
}

impl RecordingAudioSource for RealtimePcm {
    type Error = FixtureError;

    fn capture_next_available(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        if self.next == self.count {
            if let Some(finished) = self.finished.take() {
                let _ = finished.send(());
            }
            thread::sleep(timeout);
            return Ok(None);
        }
        let origin = self.first_origin.load(Ordering::Acquire);
        let end = origin + (self.next + 1) * 1_000_000;
        // 同一时钟已到达本块末尾才返回；没有快进 PTS 或提前声称采集完成。
        while self.clock.now_ns() < end {
            thread::sleep(Duration::from_nanos(end - self.clock.now_ns().min(end)));
        }
        let value = CapturedAudioChunk {
            sequence: self.next,
            captured_at_ns: origin + self.next * 1_000_000,
            format: AudioFormat::normalized(2),
            frame_count: 48,
            samples: vec![0.1; 96].into_boxed_slice(),
        };
        self.next += 1;
        Ok(Some(value))
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(self.clock.now_ns())
    }
}

#[test]
fn one_fps_pending_wgc_frame_keeps_realtime_pcm_within_original_budget() {
    let temporary = tempfile::tempdir().unwrap();
    let first_origin = Arc::new(AtomicU64::new(0));
    let dropped = Arc::new(AtomicBool::new(false));
    let video_origin = Arc::clone(&first_origin);
    let video_dropped = Arc::clone(&dropped);
    let (finished, completion) = mpsc::channel();
    let mut settings = config("pending-wgc-realtime");
    settings.frames_per_second = 1;
    let mut session = AvRecordingSession::start_with_factories(
        temporary.path(),
        settings,
        move |clock| pending_wgc_source(clock, video_origin, video_dropped),
        move |clock| {
            Ok(RealtimePcm {
                clock,
                first_origin,
                next: 0,
                count: 1_200,
                finished: Some(finished),
            })
        },
    )
    .unwrap();
    let done = completion.recv_timeout(Duration::from_secs(5));
    let early = if matches!(done, Err(mpsc::RecvTimeoutError::Disconnected))
        || session.has_terminated_worker()
    {
        Some(session.audio_capture.take().unwrap().wait())
    } else {
        None
    };
    let result = session.stop();
    assert!(
        dropped.load(Ordering::Acquire),
        "原 bridge owner 应在停止/错误后 join"
    );
    assert!(
        early.is_none(),
        "缓存第二帧不得使实际 PCM worker 溢出原预算：{early:?}; stop={result:?}"
    );
    done.unwrap();
    let report = result.unwrap();
    assert_eq!(report.video_captured_frames, 2);
    assert_eq!(report.video_encoder_input_frames, 2);
    assert_eq!(report.video_dropped_by_backpressure, 0);
    assert_eq!(report.audio_accepted_chunks, 1_200);
    assert_eq!(report.audio_encoder_input_frames, 57_600);
    assert_eq!(report.audio_dropped_before_video_frames, 0);
    let path = temporary.path().join("recordings/pending-wgc-realtime");
    let value: Value =
        serde_json::from_slice(&fs::read(path.join("manifest.json")).unwrap()).unwrap();
    read_idle_complete(&path, &value, 1, report.duration_ns);
}
