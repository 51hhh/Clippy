use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};

/// 使用原 WGC bridge、两个合成 Frame 和会话时钟；不连接任何显示器/设备。
pub(in crate::recording) struct PendingWgcSource {
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

pub(in crate::recording) fn pending_wgc_source(
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
