//! macOS 显示器区域帧源。
//!
//! 默认 macOS 11 构建继续使用 AVFoundation。显式 ScreenCaptureKit QA feature 在 12.3+ 使用
//! 精确窗口排除；两条路径都只在采集 worker 内创建、使用并析构原生对象。

#[cfg(feature = "recording-macos-screencapturekit")]
mod screencapturekit;

#[cfg(not(feature = "recording-macos-screencapturekit"))]
use super::region::take_direct_region_rgba;
use super::region::{validate_direct_region_selection, RegionFrameError};
use super::{RecordingControlTarget, RecordingSourceDescriptor};
use crate::capture::RecordingCaptureSpec;
#[cfg(not(feature = "recording-macos-screencapturekit"))]
use crate::recording::frame::{CapturedFrame, FrameError};
#[cfg(not(feature = "recording-macos-screencapturekit"))]
use crate::recording::worker::RecordingFrameSource;
use objc2_core_graphics::{CGDisplayPixelsHigh, CGDisplayPixelsWide};
#[cfg(not(feature = "recording-macos-screencapturekit"))]
use std::sync::{Arc, Condvar, Mutex};
#[cfg(not(feature = "recording-macos-screencapturekit"))]
use std::thread::{self, JoinHandle};
use std::time::Duration;
#[cfg(not(feature = "recording-macos-screencapturekit"))]
use std::time::Instant;
use thiserror::Error;
use xcap::Monitor;
#[cfg(not(feature = "recording-macos-screencapturekit"))]
use xcap::{Frame, VideoRecorder};

const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);
const FRAME_POLL_TIMEOUT: Duration = Duration::from_millis(50);

#[derive(Debug, Error)]
pub(in crate::recording) enum MacFrameSourceError {
    #[error("macOS 录屏显示器不存在或映射不唯一")]
    MonitorMissing,
    #[error("macOS 录屏初始化失败: {0}")]
    Initialize(String),
    #[error("macOS 录屏控制失败: {0}")]
    Control(String),
    #[error("macOS 录屏等待首帧超时")]
    FirstFrameTimeout,
    #[error("macOS 录屏帧流已经关闭")]
    StreamClosed,
    #[error("macOS 录屏帧桥接锁已损坏")]
    BridgePoisoned,
    #[error("macOS 录屏帧序号耗尽")]
    SequenceExhausted,
    #[error("macOS 录屏单调时间戳耗尽")]
    TimestampExhausted,
    #[error("macOS 录屏物理坐标溢出")]
    CoordinateOverflow,
    #[error("macOS 显示器缩放无效")]
    InvalidScale,
    #[error("macOS ScreenCaptureKit 缺少本次控制窗的原生排除目标")]
    ControlTargetMissing,
    #[error("macOS ScreenCaptureKit 控制窗 ID 超出 CGWindowID 范围")]
    ControlTargetOverflow,
    #[error(transparent)]
    Region(#[from] RegionFrameError),
    #[error(transparent)]
    Frame(#[from] FrameError),
}

#[derive(Debug, Clone)]
pub(in crate::recording) struct MacRegionFrameSourcePlan {
    selection: RecordingCaptureSpec,
    capture_region: MacCaptureRegion,
    descriptor: RecordingSourceDescriptor,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct MacCaptureRegion {
    x_points: f64,
    y_points_from_top: f64,
    y_points: f64,
    width_points: f64,
    height_points: f64,
    scale_factor: f64,
}

impl MacRegionFrameSourcePlan {
    pub fn prepare(selection: RecordingCaptureSpec) -> Result<Self, MacFrameSourceError> {
        validate_direct_region_selection(selection)?;
        let monitor = exact_monitor(selection.monitor_id)?;
        let logical_width = monitor
            .width()
            .map_err(|error| MacFrameSourceError::Initialize(error.to_string()))?;
        let logical_height = monitor
            .height()
            .map_err(|error| MacFrameSourceError::Initialize(error.to_string()))?;
        let pixel_width = u32::try_from(CGDisplayPixelsWide(selection.monitor_id))
            .map_err(|_| MacFrameSourceError::InvalidScale)?;
        let pixel_height = u32::try_from(CGDisplayPixelsHigh(selection.monitor_id))
            .map_err(|_| MacFrameSourceError::InvalidScale)?;
        if pixel_width != selection.monitor_pixel_width
            || pixel_height != selection.monitor_pixel_height
        {
            return Err(RegionFrameError::MonitorGeometryChanged.into());
        }
        let capture_region = checked_capture_region(selection, logical_width, logical_height)?;
        let logical_x = monitor
            .x()
            .map_err(|error| MacFrameSourceError::Initialize(error.to_string()))?;
        let logical_y = monitor
            .y()
            .map_err(|error| MacFrameSourceError::Initialize(error.to_string()))?;
        let physical_x = checked_physical_coordinate(
            logical_x,
            logical_width,
            pixel_width,
            selection.crop_left,
        )?;
        let physical_y = checked_physical_coordinate(
            logical_y,
            logical_height,
            pixel_height,
            selection.crop_top,
        )?;
        Ok(Self {
            selection,
            capture_region,
            descriptor: RecordingSourceDescriptor {
                source_id: format!(
                    "macos-{}-{}",
                    if cfg!(feature = "recording-macos-screencapturekit") {
                        "screencapturekit"
                    } else {
                        "avfoundation"
                    },
                    selection.monitor_id
                ),
                physical_x,
                physical_y,
                width: selection.crop_width,
                height: selection.crop_height,
            },
        })
    }

    pub fn descriptor(&self) -> &RecordingSourceDescriptor {
        &self.descriptor
    }

    pub fn connect(
        self,
        control_target: RecordingControlTarget,
    ) -> Result<MacRegionFrameSource, MacFrameSourceError> {
        #[cfg(not(feature = "recording-macos-screencapturekit"))]
        {
            debug_assert_eq!(control_target, RecordingControlTarget::NoNativeWindow);
            MacAvRegionFrameSource::connect(self)
        }
        #[cfg(feature = "recording-macos-screencapturekit")]
        {
            screencapturekit::MacScreenCaptureKitRegionFrameSource::connect(self, control_target)
        }
    }
}

#[cfg(not(feature = "recording-macos-screencapturekit"))]
pub(in crate::recording) type MacRegionFrameSource = MacAvRegionFrameSource;
#[cfg(feature = "recording-macos-screencapturekit")]
pub(in crate::recording) type MacRegionFrameSource =
    screencapturekit::MacScreenCaptureKitRegionFrameSource;

#[cfg(not(feature = "recording-macos-screencapturekit"))]
struct StampedFrame {
    captured_at_ns: u64,
    frame: Frame,
}

#[derive(Default)]
#[cfg(not(feature = "recording-macos-screencapturekit"))]
struct LatestFrame {
    frame: Option<StampedFrame>,
    closed: bool,
}

#[derive(Default)]
#[cfg(not(feature = "recording-macos-screencapturekit"))]
struct FrameBridge {
    latest: Mutex<LatestFrame>,
    changed: Condvar,
}

#[cfg(not(feature = "recording-macos-screencapturekit"))]
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

    fn discard(&self) -> Result<(), MacFrameSourceError> {
        let mut latest = self
            .latest
            .lock()
            .map_err(|_| MacFrameSourceError::BridgePoisoned)?;
        latest.frame = None;
        Ok(())
    }

    fn take_after(
        &self,
        minimum_timestamp_ns: u64,
        timeout: Duration,
    ) -> Result<Option<StampedFrame>, MacFrameSourceError> {
        let deadline = Instant::now() + timeout;
        let mut latest = self
            .latest
            .lock()
            .map_err(|_| MacFrameSourceError::BridgePoisoned)?;
        loop {
            if let Some(frame) = latest.frame.take() {
                if frame.captured_at_ns >= minimum_timestamp_ns {
                    return Ok(Some(frame));
                }
            }
            if latest.closed {
                return Err(MacFrameSourceError::StreamClosed);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            let (guard, waited) = self
                .changed
                .wait_timeout(latest, remaining)
                .map_err(|_| MacFrameSourceError::BridgePoisoned)?;
            latest = guard;
            if waited.timed_out() && latest.frame.is_none() {
                return Ok(None);
            }
        }
    }
}

#[cfg(not(feature = "recording-macos-screencapturekit"))]
pub(in crate::recording) struct MacAvRegionFrameSource {
    recorder: Option<VideoRecorder>,
    bridge: Arc<FrameBridge>,
    bridge_thread: Option<JoinHandle<()>>,
    selection: RecordingCaptureSpec,
    descriptor: RecordingSourceDescriptor,
    clock_origin: Instant,
    minimum_timestamp_ns: u64,
    last_timestamp_ns: Option<u64>,
    next_sequence: u64,
    first_frame: bool,
    running: bool,
}

#[cfg(not(feature = "recording-macos-screencapturekit"))]
impl MacAvRegionFrameSource {
    fn connect(plan: MacRegionFrameSourcePlan) -> Result<Self, MacFrameSourceError> {
        let current = MacRegionFrameSourcePlan::prepare(plan.selection)?;
        if current.capture_region != plan.capture_region || current.descriptor != plan.descriptor {
            return Err(RegionFrameError::MonitorGeometryChanged.into());
        }
        let monitor = exact_monitor(plan.selection.monitor_id)?;
        let region = plan.capture_region;
        let (recorder, frames) = monitor
            .video_recorder_region(
                region.x_points,
                region.y_points,
                region.width_points,
                region.height_points,
                region.scale_factor,
            )
            .map_err(|error| MacFrameSourceError::Initialize(error.to_string()))?;
        let bridge = Arc::new(FrameBridge::default());
        let thread_bridge = Arc::clone(&bridge);
        let clock_origin = Instant::now();
        let bridge_thread = thread::Builder::new()
            .name("clippy-recording-macos-bridge".to_string())
            .spawn(move || {
                let mut last_timestamp_ns = None;
                while let Ok(frame) = frames.recv() {
                    let sampled =
                        clock_origin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
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
            .map_err(|error| MacFrameSourceError::Initialize(error.to_string()))?;
        if let Err(error) = recorder.start() {
            drop(recorder);
            let _ = bridge_thread.join();
            return Err(MacFrameSourceError::Initialize(error.to_string()));
        }
        Ok(Self {
            recorder: Some(recorder),
            bridge,
            bridge_thread: Some(bridge_thread),
            selection: plan.selection,
            descriptor: plan.descriptor,
            clock_origin,
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

    fn recorder(&self) -> Result<&VideoRecorder, MacFrameSourceError> {
        self.recorder
            .as_ref()
            .ok_or(MacFrameSourceError::StreamClosed)
    }

    fn next_timestamp_ns(&self) -> Result<u64, MacFrameSourceError> {
        let sampled = self
            .clock_origin
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        match self.last_timestamp_ns {
            Some(last) => Ok(sampled.max(
                last.checked_add(1)
                    .ok_or(MacFrameSourceError::TimestampExhausted)?,
            )),
            None => Ok(sampled),
        }
    }

    fn set_running(&mut self, running: bool) -> Result<u64, MacFrameSourceError> {
        if self.running != running {
            if running {
                self.recorder().and_then(|recorder| {
                    recorder
                        .start()
                        .map_err(|error| MacFrameSourceError::Control(error.to_string()))
                })?;
            } else {
                self.recorder().and_then(|recorder| {
                    recorder
                        .stop()
                        .map_err(|error| MacFrameSourceError::Control(error.to_string()))
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
    ) -> Result<Option<CapturedFrame>, MacFrameSourceError> {
        let Some(stamped) = self.bridge.take_after(self.minimum_timestamp_ns, timeout)? else {
            if self.first_frame && self.clock_origin.elapsed() >= FIRST_FRAME_TIMEOUT {
                return Err(MacFrameSourceError::FirstFrameTimeout);
            }
            return Ok(None);
        };
        self.first_frame = false;
        let rgba = take_direct_region_rgba(
            self.selection,
            stamped.frame.width,
            stamped.frame.height,
            stamped.frame.raw,
        )?;
        let sequence = self.next_sequence;
        self.next_sequence = sequence
            .checked_add(1)
            .ok_or(MacFrameSourceError::SequenceExhausted)?;
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

#[cfg(not(feature = "recording-macos-screencapturekit"))]
impl RecordingFrameSource for MacAvRegionFrameSource {
    type Error = MacFrameSourceError;

    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
        self.take_frame(FIRST_FRAME_TIMEOUT)?
            .ok_or(MacFrameSourceError::FirstFrameTimeout)
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

#[cfg(not(feature = "recording-macos-screencapturekit"))]
impl Drop for MacAvRegionFrameSource {
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

fn exact_monitor(monitor_id: u32) -> Result<Monitor, MacFrameSourceError> {
    let mut matches = Monitor::all()
        .map_err(|error| MacFrameSourceError::Initialize(error.to_string()))?
        .into_iter()
        .filter_map(|monitor| match monitor.id() {
            Ok(id) if id == monitor_id => Some(Ok(monitor)),
            Ok(_) => None,
            Err(error) => Some(Err(MacFrameSourceError::Initialize(error.to_string()))),
        });
    let monitor = matches
        .next()
        .transpose()?
        .ok_or(MacFrameSourceError::MonitorMissing)?;
    if matches.next().is_some() {
        return Err(MacFrameSourceError::MonitorMissing);
    }
    Ok(monitor)
}

fn checked_physical_coordinate(
    logical_origin: i32,
    logical_extent: u32,
    physical_extent: u32,
    offset: u32,
) -> Result<i32, MacFrameSourceError> {
    if logical_extent == 0 || physical_extent == 0 {
        return Err(MacFrameSourceError::InvalidScale);
    }
    let origin = f64::from(logical_origin) * f64::from(physical_extent) / f64::from(logical_extent);
    if !origin.is_finite() || origin < f64::from(i32::MIN) || origin > f64::from(i32::MAX) {
        return Err(MacFrameSourceError::CoordinateOverflow);
    }
    (origin.round() as i32)
        .checked_add(i32::try_from(offset).map_err(|_| MacFrameSourceError::CoordinateOverflow)?)
        .ok_or(MacFrameSourceError::CoordinateOverflow)
}

fn checked_capture_region(
    selection: RecordingCaptureSpec,
    logical_width: u32,
    logical_height: u32,
) -> Result<MacCaptureRegion, MacFrameSourceError> {
    validate_direct_region_selection(selection)?;
    if logical_width == 0 || logical_height == 0 {
        return Err(MacFrameSourceError::InvalidScale);
    }
    let scale_x = f64::from(selection.monitor_pixel_width) / f64::from(logical_width);
    let scale_y = f64::from(selection.monitor_pixel_height) / f64::from(logical_height);
    let tolerance = scale_x.abs().max(scale_y.abs()).max(1.0) * 1e-6;
    if !scale_x.is_finite()
        || !scale_y.is_finite()
        || scale_x <= 0.0
        || scale_y <= 0.0
        || (scale_x - scale_y).abs() > tolerance
    {
        return Err(MacFrameSourceError::InvalidScale);
    }
    let crop_bottom = selection
        .monitor_pixel_height
        .checked_sub(
            selection
                .crop_top
                .checked_add(selection.crop_height)
                .ok_or(RegionFrameError::InvalidRegion)?,
        )
        .ok_or(RegionFrameError::InvalidRegion)?;
    let region = MacCaptureRegion {
        x_points: f64::from(selection.crop_left) / scale_x,
        y_points_from_top: f64::from(selection.crop_top) / scale_y,
        y_points: f64::from(crop_bottom) / scale_y,
        width_points: f64::from(selection.crop_width) / scale_x,
        height_points: f64::from(selection.crop_height) / scale_y,
        scale_factor: scale_x,
    };
    if [
        region.x_points,
        region.y_points_from_top,
        region.y_points,
        region.width_points,
        region.height_points,
        region.scale_factor,
    ]
    .into_iter()
    .any(|value| !value.is_finite() || value < 0.0)
        || region.width_points == 0.0
        || region.height_points == 0.0
    {
        return Err(MacFrameSourceError::InvalidScale);
    }
    Ok(region)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retina_extents_and_signed_origins_are_scaled_before_crop_offset() {
        assert_eq!(
            checked_physical_coordinate(-1_440, 1_440, 2_880, 20).unwrap(),
            -2_860
        );
        assert_eq!(
            checked_physical_coordinate(100, 2_048, 3_072, 25).unwrap(),
            175
        );
    }

    #[test]
    fn invalid_scale_and_coordinate_overflow_are_rejected() {
        assert!(matches!(
            checked_physical_coordinate(100, 0, 200, 0),
            Err(MacFrameSourceError::InvalidScale)
        ));
        assert!(matches!(
            checked_physical_coordinate(i32::MAX, 1, 4, 0),
            Err(MacFrameSourceError::CoordinateOverflow)
        ));
    }

    #[test]
    fn converts_backing_pixels_to_both_macos_capture_coordinate_systems() {
        let selection = RecordingCaptureSpec {
            monitor_id: 7,
            monitor_pixel_width: 2_880,
            monitor_pixel_height: 1_800,
            crop_left: 20,
            crop_top: 100,
            crop_width: 400,
            crop_height: 300,
        };
        assert_eq!(
            checked_capture_region(selection, 1_440, 900).unwrap(),
            MacCaptureRegion {
                x_points: 10.0,
                y_points_from_top: 50.0,
                y_points: 700.0,
                width_points: 200.0,
                height_points: 150.0,
                scale_factor: 2.0,
            }
        );
    }

    #[test]
    fn preserves_fractional_point_boundaries_and_rejects_anisotropic_scaling() {
        let selection = RecordingCaptureSpec {
            monitor_id: 7,
            monitor_pixel_width: 3_000,
            monitor_pixel_height: 2_000,
            crop_left: 1,
            crop_top: 3,
            crop_width: 401,
            crop_height: 301,
        };
        let region = checked_capture_region(selection, 1_500, 1_000).unwrap();
        assert_eq!(region.x_points, 0.5);
        assert_eq!(region.width_points, 200.5);
        assert_eq!(region.height_points, 150.5);

        assert!(matches!(
            checked_capture_region(selection, 1_500, 800),
            Err(MacFrameSourceError::InvalidScale)
        ));
    }
}
