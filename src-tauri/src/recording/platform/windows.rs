//! Windows WGC 显示器区域帧源。
//!
//! xcap 的 WGC 回调先进入零容量通道；独立桥接线程立即接走并只保留最新一帧，避免暂停期间阻塞
//! WGC 回调，也避免在 Clippy 侧形成无界队列。仓库固定的 xcap 补丁会要求 WGC 包含光标；原生
//! Windows 原型构建通过受门控入口取得真机像素证据；默认发布 feature 仍保持关闭。

use super::region::{crop_tight_rgba, validate_selection, RegionFrameError};
use super::RecordingSourceDescriptor;
use crate::capture::RecordingCaptureSpec;
use crate::recording::clock::RecordingSessionClock;
use crate::recording::frame::{CapturedFrame, FrameError};
use crate::recording::worker::RecordingFrameSource;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use thiserror::Error;
use xcap::{Frame, Monitor, VideoRecorder};

const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);
const FRAME_POLL_TIMEOUT: Duration = Duration::from_millis(50);

#[derive(Debug, Error)]
pub(in crate::recording) enum WindowsFrameSourceError {
    #[error("Windows 录屏显示器不存在或映射不唯一")]
    MonitorMissing,
    #[error("Windows WGC 初始化失败: {0}")]
    Initialize(String),
    #[error("Windows WGC 控制失败: {0}")]
    Control(String),
    #[error("Windows WGC 等待首帧超时")]
    FirstFrameTimeout,
    #[error("Windows WGC 帧流已经关闭")]
    StreamClosed,
    #[error("Windows WGC 帧桥接锁已损坏")]
    BridgePoisoned,
    #[error("Windows 录屏帧序号耗尽")]
    SequenceExhausted,
    #[error("Windows 录屏单调时间戳耗尽")]
    TimestampExhausted,
    #[error("Windows 录屏物理坐标溢出")]
    CoordinateOverflow,
    #[error(transparent)]
    Region(#[from] RegionFrameError),
    #[error(transparent)]
    Frame(#[from] FrameError),
}

/// WGC 对象必须在采集 worker 内创建；计划阶段只保存重新枚举后得到的可信几何。
#[derive(Debug, Clone)]
pub(in crate::recording) struct WindowsWgcFrameSourcePlan {
    selection: RecordingCaptureSpec,
    descriptor: RecordingSourceDescriptor,
}

impl WindowsWgcFrameSourcePlan {
    pub fn prepare(selection: RecordingCaptureSpec) -> Result<Self, WindowsFrameSourceError> {
        validate_selection(selection)?;
        let monitor = exact_monitor(selection.monitor_id)?;
        let descriptor = monitor_descriptor(selection, &monitor)?;
        Ok(Self {
            selection,
            descriptor,
        })
    }

    pub fn descriptor(&self) -> &RecordingSourceDescriptor {
        &self.descriptor
    }

    pub fn connect(
        self,
        clock: RecordingSessionClock,
    ) -> Result<WindowsWgcRegionFrameSource, WindowsFrameSourceError> {
        let source = WindowsWgcRegionFrameSource::connect(self.selection, clock)?;
        if source.descriptor != self.descriptor {
            return Err(RegionFrameError::MonitorGeometryChanged.into());
        }
        Ok(source)
    }
}

struct StampedFrame {
    captured_at_ns: u64,
    frame: Frame,
}

#[derive(Default)]
struct LatestFrame {
    frame: Option<StampedFrame>,
    closed: bool,
}

#[derive(Default)]
struct FrameBridge {
    latest: Mutex<LatestFrame>,
    changed: Condvar,
}

impl FrameBridge {
    fn replace(&self, frame: StampedFrame) {
        let Ok(mut latest) = self.latest.lock() else {
            return;
        };
        latest.frame = Some(frame);
        self.changed.notify_one();
    }

    fn close(&self) {
        let Ok(mut latest) = self.latest.lock() else {
            return;
        };
        latest.closed = true;
        self.changed.notify_all();
    }

    fn discard(&self) -> Result<(), WindowsFrameSourceError> {
        let mut latest = self
            .latest
            .lock()
            .map_err(|_| WindowsFrameSourceError::BridgePoisoned)?;
        latest.frame = None;
        Ok(())
    }

    fn take_after(
        &self,
        minimum_timestamp_ns: u64,
        timeout: Duration,
    ) -> Result<Option<StampedFrame>, WindowsFrameSourceError> {
        let deadline = Instant::now() + timeout;
        let mut latest = self
            .latest
            .lock()
            .map_err(|_| WindowsFrameSourceError::BridgePoisoned)?;
        loop {
            if let Some(frame) = latest.frame.take() {
                if frame.captured_at_ns >= minimum_timestamp_ns {
                    return Ok(Some(frame));
                }
            }
            if latest.closed {
                return Err(WindowsFrameSourceError::StreamClosed);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            let (guard, waited) = self
                .changed
                .wait_timeout(latest, remaining)
                .map_err(|_| WindowsFrameSourceError::BridgePoisoned)?;
            latest = guard;
            if waited.timed_out() && latest.frame.is_none() {
                return Ok(None);
            }
        }
    }
}

pub(in crate::recording) struct WindowsWgcRegionFrameSource {
    recorder: Option<VideoRecorder>,
    bridge: Arc<FrameBridge>,
    bridge_thread: Option<JoinHandle<()>>,
    selection: RecordingCaptureSpec,
    descriptor: RecordingSourceDescriptor,
    clock: RecordingSessionClock,
    first_frame_started_at: Instant,
    minimum_timestamp_ns: u64,
    last_timestamp_ns: Option<u64>,
    next_sequence: u64,
    first_frame: bool,
    running: bool,
}

impl WindowsWgcRegionFrameSource {
    pub fn connect(
        selection: RecordingCaptureSpec,
        clock: RecordingSessionClock,
    ) -> Result<Self, WindowsFrameSourceError> {
        validate_selection(selection)?;
        let monitor = exact_monitor(selection.monitor_id)?;
        let descriptor = monitor_descriptor(selection, &monitor)?;
        let (recorder, frames) = monitor
            .video_recorder()
            .map_err(|error| WindowsFrameSourceError::Initialize(error.to_string()))?;
        let bridge = Arc::new(FrameBridge::default());
        let thread_bridge = Arc::clone(&bridge);
        let callback_clock = clock.clone();
        let bridge_thread = thread::Builder::new()
            .name("clippy-recording-wgc-bridge".to_string())
            .spawn(move || {
                let mut last_timestamp_ns = None;
                while let Ok(frame) = frames.recv() {
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
            })
            .map_err(|error| WindowsFrameSourceError::Initialize(error.to_string()))?;
        recorder
            .start()
            .map_err(|error| WindowsFrameSourceError::Initialize(error.to_string()))?;
        Ok(Self {
            recorder: Some(recorder),
            bridge,
            bridge_thread: Some(bridge_thread),
            selection,
            descriptor,
            clock,
            first_frame_started_at: Instant::now(),
            minimum_timestamp_ns: 0,
            last_timestamp_ns: None,
            next_sequence: 0,
            first_frame: true,
            running: true,
        })
    }

    pub fn descriptor(&self) -> &RecordingSourceDescriptor {
        &self.descriptor
    }

    fn recorder(&self) -> Result<&VideoRecorder, WindowsFrameSourceError> {
        self.recorder
            .as_ref()
            .ok_or(WindowsFrameSourceError::StreamClosed)
    }

    fn next_timestamp_ns(&self) -> Result<u64, WindowsFrameSourceError> {
        let sampled = self.clock.now_ns();
        match self.last_timestamp_ns {
            Some(last) => Ok(sampled.max(
                last.checked_add(1)
                    .ok_or(WindowsFrameSourceError::TimestampExhausted)?,
            )),
            None => Ok(sampled),
        }
    }

    fn set_running(&mut self, running: bool) -> Result<u64, WindowsFrameSourceError> {
        if self.running != running {
            if running {
                self.recorder().and_then(|recorder| {
                    recorder
                        .start()
                        .map_err(|error| WindowsFrameSourceError::Control(error.to_string()))
                })?;
            } else {
                self.recorder().and_then(|recorder| {
                    recorder
                        .stop()
                        .map_err(|error| WindowsFrameSourceError::Control(error.to_string()))
                })?;
            }
            self.running = running;
        }
        let timestamp = self.next_timestamp_ns()?;
        self.last_timestamp_ns = Some(timestamp);
        self.minimum_timestamp_ns = timestamp;
        self.bridge.discard()?;
        Ok(timestamp)
    }

    fn take_frame(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedFrame>, WindowsFrameSourceError> {
        let Some(stamped) = self.bridge.take_after(self.minimum_timestamp_ns, timeout)? else {
            if self.first_frame && self.first_frame_started_at.elapsed() >= FIRST_FRAME_TIMEOUT {
                return Err(WindowsFrameSourceError::FirstFrameTimeout);
            }
            return Ok(None);
        };
        self.first_frame = false;
        let rgba = crop_tight_rgba(
            self.selection,
            stamped.frame.width,
            stamped.frame.height,
            &stamped.frame.raw,
        )?;
        let sequence = self.next_sequence;
        self.next_sequence = sequence
            .checked_add(1)
            .ok_or(WindowsFrameSourceError::SequenceExhausted)?;
        self.last_timestamp_ns = Some(stamped.captured_at_ns);
        let frame = CapturedFrame {
            sequence,
            captured_at_ns: stamped.captured_at_ns,
            width: self.selection.crop_width,
            height: self.selection.crop_height,
            stride: self.selection.crop_width * 4,
            rgba,
        };
        frame.validate()?;
        Ok(Some(frame))
    }
}

fn exact_monitor(monitor_id: u32) -> Result<Monitor, WindowsFrameSourceError> {
    let mut matches = Monitor::all()
        .map_err(|error| WindowsFrameSourceError::Initialize(error.to_string()))?
        .into_iter()
        .filter_map(|monitor| match monitor.id() {
            Ok(id) if id == monitor_id => Some(Ok(monitor)),
            Ok(_) => None,
            Err(error) => Some(Err(WindowsFrameSourceError::Initialize(error.to_string()))),
        });
    let monitor = matches
        .next()
        .transpose()?
        .ok_or(WindowsFrameSourceError::MonitorMissing)?;
    if matches.next().is_some() {
        return Err(WindowsFrameSourceError::MonitorMissing);
    }
    Ok(monitor)
}

fn monitor_descriptor(
    selection: RecordingCaptureSpec,
    monitor: &Monitor,
) -> Result<RecordingSourceDescriptor, WindowsFrameSourceError> {
    let monitor_width = monitor
        .width()
        .map_err(|error| WindowsFrameSourceError::Initialize(error.to_string()))?;
    let monitor_height = monitor
        .height()
        .map_err(|error| WindowsFrameSourceError::Initialize(error.to_string()))?;
    if monitor_width != selection.monitor_pixel_width
        || monitor_height != selection.monitor_pixel_height
    {
        return Err(RegionFrameError::MonitorGeometryChanged.into());
    }
    let monitor_x = monitor
        .x()
        .map_err(|error| WindowsFrameSourceError::Initialize(error.to_string()))?;
    let monitor_y = monitor
        .y()
        .map_err(|error| WindowsFrameSourceError::Initialize(error.to_string()))?;
    Ok(RecordingSourceDescriptor {
        source_id: format!("windows-wgc-{}", selection.monitor_id),
        physical_x: checked_coordinate(monitor_x, selection.crop_left)?,
        physical_y: checked_coordinate(monitor_y, selection.crop_top)?,
        width: selection.crop_width,
        height: selection.crop_height,
    })
}

impl RecordingFrameSource for WindowsWgcRegionFrameSource {
    type Error = WindowsFrameSourceError;

    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
        self.take_frame(FIRST_FRAME_TIMEOUT)?
            .ok_or(WindowsFrameSourceError::FirstFrameTimeout)
    }

    fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
        self.take_frame(FRAME_POLL_TIMEOUT)
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        let timestamp = self.next_timestamp_ns()?;
        self.last_timestamp_ns = Some(timestamp);
        Ok(timestamp)
    }

    fn pause_capture(&mut self) -> Result<u64, Self::Error> {
        self.set_running(false)
    }

    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        self.set_running(true)
    }

    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        self.set_running(false)
    }
}

impl Drop for WindowsWgcRegionFrameSource {
    fn drop(&mut self) {
        if let Some(recorder) = self.recorder.take() {
            let _ = recorder.stop();
            drop(recorder);
        }
        if let Some(thread) = self.bridge_thread.take() {
            let _ = thread.join();
        }
    }
}

fn checked_coordinate(origin: i32, offset: u32) -> Result<i32, WindowsFrameSourceError> {
    origin
        .checked_add(
            i32::try_from(offset).map_err(|_| WindowsFrameSourceError::CoordinateOverflow)?,
        )
        .ok_or(WindowsFrameSourceError::CoordinateOverflow)
}
