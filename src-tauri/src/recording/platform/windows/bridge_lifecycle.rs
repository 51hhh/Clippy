use super::{
    FrameBridge, RecordingSessionClock, StampedFrame, WindowsFrameSourceError, FRAME_POLL_TIMEOUT,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use xcap::Frame;

/// 已创建的桥接线程在转移前后都有 owner；任何返回或销毁路径均取消并 join。
pub(super) struct FrameBridgeThread {
    cancelled: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FrameBridgeThread {
    fn new(cancelled: Arc<AtomicBool>, thread: JoinHandle<()>) -> Self {
        Self {
            cancelled,
            thread: Some(thread),
        }
    }

    pub(super) fn spawn(
        frames: Receiver<Frame>,
        bridge: Arc<FrameBridge>,
        clock: RecordingSessionClock,
    ) -> Result<Self, WindowsFrameSourceError> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let thread_cancelled = Arc::clone(&cancelled);
        let thread = thread::Builder::new()
            .name("clippy-recording-wgc-bridge".to_string())
            .spawn(move || forward_frames(frames, bridge, clock, thread_cancelled))
            .map_err(|error| WindowsFrameSourceError::Initialize(error.to_string()))?;
        Ok(Self::new(cancelled, thread))
    }

    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    fn finish(&mut self) {
        self.cancel();
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                log::warn!("WGC 帧桥线程退出时 panic，已完成 join");
            }
        }
    }
}

impl Drop for FrameBridgeThread {
    fn drop(&mut self) {
        self.finish();
    }
}

/// 取消不等待 callback sender 全部释放；退出时关闭共享桥并释放接收端。
fn forward_frames(
    frames: Receiver<Frame>,
    thread_bridge: Arc<FrameBridge>,
    callback_clock: RecordingSessionClock,
    cancelled: Arc<AtomicBool>,
) {
    let mut last_timestamp_ns = None;
    while !cancelled.load(Ordering::Acquire) {
        let frame = match frames.recv_timeout(FRAME_POLL_TIMEOUT) {
            Ok(frame) => frame,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if cancelled.load(Ordering::Acquire) {
            break;
        }
        let sampled = callback_clock.now_ns();
        let captured_at_ns = last_timestamp_ns
            .and_then(|last: u64| last.checked_add(1))
            .map_or(sampled, |next| sampled.max(next));
        last_timestamp_ns = Some(captured_at_ns);
        thread_bridge.replace(StampedFrame {
            captured_at_ns,
            frame,
        });
    }
    thread_bridge.close();
}

pub(super) fn start_recorder<R, E>(
    recorder: R,
    bridge: FrameBridgeThread,
    start: impl FnOnce(&R) -> Result<(), E>,
) -> Result<(R, FrameBridgeThread), E> {
    start(&recorder)?;
    Ok((recorder, bridge))
}

pub(super) fn shutdown_bridge<R>(
    recorder: Option<R>,
    mut bridge: FrameBridgeThread,
    stop: impl FnOnce(&R),
) {
    // 先使接收端可退出，避免 native Close 等待一个仍在同步发送的回调。
    bridge.cancel();
    if let Some(recorder) = recorder {
        stop(&recorder);
        drop(recorder);
    }
    bridge.finish();
}

#[cfg(test)]
mod tests;
