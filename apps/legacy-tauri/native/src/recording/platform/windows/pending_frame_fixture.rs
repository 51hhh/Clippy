use super::*;
use clippy_media::recording::audio::{AudioFormat, CapturedAudioChunk};
use clippy_media::recording::audio_worker::RecordingAudioSource;
use clippy_media::recording::av_session::{AvRecordingConfig, AvRecordingSession};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::{fs, thread};

/// 使用原 WGC bridge、两个合成 Frame 和会话时钟；不连接任何显示器/设备。
struct PendingWgcSource {
    sender: SyncSender<Frame>,
    bridge: Arc<FrameBridge>,
    owner: Option<bridge_lifecycle::FrameBridgeThread>,
    clock: RecordingSessionClock,
    first_origin: Arc<AtomicU64>,
    dropped: Arc<AtomicBool>,
    first: bool,
    sequence: u64,
    last_timestamp_ns: u64,
}

fn pending_wgc_source(
    clock: RecordingSessionClock,
    first_origin: Arc<AtomicU64>,
    dropped: Arc<AtomicBool>,
) -> Result<PendingWgcSource, String> {
    let (sender, frames) = mpsc::sync_channel(0);
    let bridge = Arc::new(FrameBridge::default());
    let owner =
        bridge_lifecycle::FrameBridgeThread::spawn(frames, Arc::clone(&bridge), clock.clone())
            .map_err(|error| error.to_string())?;
    Ok(PendingWgcSource {
        sender,
        bridge,
        owner: Some(owner),
        clock,
        first_origin,
        dropped,
        first: true,
        sequence: 0,
        last_timestamp_ns: 0,
    })
}

impl RecordingFrameSource for PendingWgcSource {
    type Error = std::io::Error;

    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
        self.capture_next_available()?.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::WouldBlock, "native frame pending")
        })
    }

    fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
        if self.first {
            self.sender
                .send(Frame::new(2, 2, vec![1; 16]))
                .map_err(std::io::Error::other)?;
        }
        let captured = self
            .bridge
            .take_after(
                0,
                if self.first {
                    Duration::from_secs(5)
                } else {
                    Duration::ZERO
                },
            )
            .map_err(std::io::Error::other)?;
        let Some(captured) = captured else {
            return Ok(None);
        };
        if self.first {
            self.first_origin
                .store(captured.captured_at_ns, Ordering::Release);
            self.sender
                .send(Frame::new(2, 2, vec![200; 16]))
                .map_err(std::io::Error::other)?;
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.bridge.latest.lock().unwrap().frame.is_none() {
                if Instant::now() >= deadline {
                    return Err(std::io::Error::other("bridge did not retain second frame"));
                }
                std::thread::yield_now();
            }
            self.first = false;
        }
        let value = CapturedFrame {
            sequence: self.sequence,
            captured_at_ns: captured.captured_at_ns,
            width: captured.frame.width,
            height: captured.frame.height,
            stride: captured.frame.width * 4,
            rgba: captured.frame.raw.into_boxed_slice(),
        };
        self.last_timestamp_ns = value.captured_at_ns;
        self.sequence += 1;
        Ok(Some(value))
    }

    fn capture_lower_bound_ns(&mut self) -> Result<Option<u64>, Self::Error> {
        self.bridge
            .capture_lower_bound_ns(0)
            .map(|bound| bound.map(|value| value.max(self.last_timestamp_ns)))
            .map_err(std::io::Error::other)
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        Ok(self.clock.now_ns())
    }
}

impl Drop for PendingWgcSource {
    fn drop(&mut self) {
        // sender 仍存活时取消并 join 原 bridge；标志只在实际 join 返回后发布。
        drop(self.owner.take());
        self.dropped.store(true, Ordering::Release);
    }
}

include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../crates/clippy-media/src/recording/idle_frontier_read_tests.rs"
));

struct RealtimePcm {
    clock: RecordingSessionClock,
    first_origin: Arc<AtomicU64>,
    next: u64,
    count: u64,
    finished: Option<mpsc::Sender<()>>,
}

impl RecordingAudioSource for RealtimePcm {
    type Error = std::io::Error;

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
    let settings = AvRecordingConfig {
        session_id: "pending-wgc-realtime".into(),
        source_id: "fixture-av-session".into(),
        physical_x: -100,
        physical_y: 20,
        width: 2,
        height: 2,
        frames_per_second: 1,
        include_cursor: false,
        audio_channels: 2,
        segment_duration_ns: 200_000_000,
    };
    let session = AvRecordingSession::start_with_factories(
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
    let early = matches!(done, Err(mpsc::RecvTimeoutError::Disconnected))
        || session.has_terminated_worker();
    let directory = session.session_directory().to_path_buf();
    let result = session.stop();
    assert!(
        dropped.load(Ordering::Acquire),
        "原 bridge owner 应在停止/错误后 join"
    );
    assert!(
        !early,
        "缓存第二帧不得使实际 PCM worker 溢出原预算：stop={result:?}"
    );
    done.unwrap();
    let report = result.unwrap();
    assert_eq!(report.video_captured_frames, 2);
    assert_eq!(report.video_encoder_input_frames, 2);
    assert_eq!(report.video_dropped_by_backpressure, 0);
    assert_eq!(report.audio_accepted_chunks, 1_200);
    assert_eq!(report.audio_encoder_input_frames, 57_600);
    assert_eq!(report.audio_dropped_before_video_frames, 0);
    let value: Value =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    read_idle_complete(&directory, &value, 1, report.duration_ns);
}
