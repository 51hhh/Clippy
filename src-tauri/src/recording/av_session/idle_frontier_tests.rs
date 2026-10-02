use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
include!("../idle_frontier_read_tests.rs");

struct IdleVideo {
    first: Option<CapturedFrame>,
    lower_bound: Arc<AtomicU64>,
    stop_at_ns: u64,
}

impl RecordingFrameSource for IdleVideo {
    type Error = FixtureError;

    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
        self.first.take().ok_or(FixtureError("no new native video"))
    }

    fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
        Ok(self.first.take())
    }

    fn capture_lower_bound_ns(&mut self) -> Result<Option<u64>, Self::Error> {
        Ok(Some(self.lower_bound.load(Ordering::Acquire)))
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(self.stop_at_ns)
    }
}

struct IdlePcm {
    next: u64,
    count: u64,
    lower_bound: Arc<AtomicU64>,
    finished: Option<mpsc::Sender<()>>,
}

impl RecordingAudioSource for IdlePcm {
    type Error = FixtureError;

    fn capture_next_available(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        if self.next < self.count {
            thread::sleep(Duration::from_millis(20));
            let value = chunk(self.next, self.next * 100_000_000);
            self.next += 1;
            self.lower_bound
                .fetch_max(self.next * 100_000_000, Ordering::Release);
            return Ok(Some(value));
        }
        if let Some(finished) = self.finished.take() {
            let _ = finished.send(());
        }
        thread::sleep(timeout);
        Ok(None)
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(self.count * 100_000_000)
    }
}

fn run_idle(id: &str, fps: u32, count: u64, initial_bound: u64) {
    let temporary = tempfile::tempdir().unwrap();
    let lower_bound = Arc::new(AtomicU64::new(initial_bound));
    let video_bound = Arc::clone(&lower_bound);
    let audio_bound = Arc::clone(&lower_bound);
    let (finished, completion) = mpsc::channel();
    let mut settings = config(id);
    settings.frames_per_second = fps;
    let mut session = AvRecordingSession::start_with_factories(
        temporary.path(),
        settings,
        move |_| {
            Ok(IdleVideo {
                first: Some(frame(0, 0)),
                lower_bound: video_bound,
                stop_at_ns: count * 100_000_000,
            })
        },
        move |_| {
            Ok(IdlePcm {
                next: 0,
                count,
                lower_bound: audio_bound,
                finished: Some(finished),
            })
        },
    )
    .unwrap();
    let finished_result = completion.recv_timeout(Duration::from_secs(3));
    let audio_source_disconnected =
        matches!(finished_result, Err(mpsc::RecvTimeoutError::Disconnected));
    // 完成 sender 已随 source 析构时，线程可能尚未发布 is_finished；join 才能读取原错。
    let early_audio_error = if audio_source_disconnected || session.has_terminated_worker() {
        Some(session.audio_capture.take().unwrap().wait())
    } else {
        None
    };
    // 仅在成功分支检查停止前的周期提交；失败分支仍暴露实际采集 worker 的背压原错。
    let periodic = if early_audio_error.is_none() && finished_result.is_ok() {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let value: Value = serde_json::from_slice(
                &fs::read(session.session_directory().join("manifest.json")).unwrap(),
            )
            .unwrap();
            let segments = value["segments"].as_array().unwrap();
            // journal 先提交清单再提升已 fsync 的 partial，读取必须等待这两个原有步骤完成。
            if !segments.is_empty()
                && segments.iter().all(|segment| {
                    session
                        .session_directory()
                        .join(segment["fileName"].as_str().unwrap())
                        .is_file()
                })
            {
                for segment in value["segments"].as_array().unwrap() {
                    read_idle_artifact(session.session_directory(), &value, segment, fps);
                }
                break Some(value);
            }
            if session.has_terminated_worker() || std::time::Instant::now() >= deadline {
                break None;
            }
            thread::sleep(Duration::from_millis(5));
        }
    } else {
        None
    };
    let result = session.stop();
    assert!(
        early_audio_error.is_none(),
        "首帧后空闲不应溢出 PCM 队列：{early_audio_error:?}; stop={result:?}"
    );
    finished_result.unwrap();
    assert!(
        periodic.is_some(),
        "首帧后空闲应在 Stop 前提交可严格读取的周期分段"
    );
    let report = result.unwrap();
    assert_eq!(report.video_captured_frames, 1);
    assert_eq!(report.video_encoder_input_frames, 1);
    assert_eq!(report.audio_accepted_chunks, count);
    assert_eq!(report.audio_encoder_input_chunks, count);
    assert_eq!(report.audio_dropped_before_video_frames, 0);
    let value: Value = serde_json::from_slice(
        &fs::read(
            temporary
                .path()
                .join(format!("recordings/{id}/manifest.json")),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(value["state"], "complete");
    assert_eq!(
        value["finalOutput"]["frameCount"],
        count * u64::from(fps) / 10
    );
    assert_eq!(
        value["finalOutput"]["audio"]["pcmFrameCount"],
        count * 4_800
    );
    read_idle_complete(
        &temporary.path().join(format!("recordings/{id}")),
        &value,
        fps,
        count * 100_000_000,
    );
}

#[test]
fn idle_after_first_frame_continues_pcm_without_backpressure() {
    run_idle("idle-av-pressure", 10, 30, 0);
}

#[test]
fn one_fps_idle_preserves_native_frame_and_consumes_pcm() {
    run_idle("idle-av-one", 1, 20, 0);
}
