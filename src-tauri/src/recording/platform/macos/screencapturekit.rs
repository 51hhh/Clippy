//! ScreenCaptureKit 区域帧源。
//!
//! 所有 Objective-C 对象都在录屏采集 worker 内创建和销毁。系统回调只向容量为一的通道
//! `try_send` 最新帧，绝不阻塞 ScreenCaptureKit 的串行队列。

use super::{
    MacFrameSourceError, MacRegionFrameSourcePlan, FIRST_FRAME_TIMEOUT, FRAME_POLL_TIMEOUT,
};
use crate::recording::frame::CapturedFrame;
use crate::recording::platform::{RecordingControlTarget, RecordingSourceDescriptor};
use crate::recording::worker::RecordingFrameSource;
use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2::{define_class, rc::Retained, runtime::ProtocolObject, AllocAnyThread, DefinedClass};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::{
    kCVPixelFormatType_32BGRA, kCVReturnSuccess, CVPixelBufferGetBaseAddress,
    CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCShareableContent, SCStream, SCStreamConfiguration,
    SCStreamDelegate, SCStreamOutput, SCStreamOutputType, SCWindow,
};
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ASYNC_OPERATION_TIMEOUT: Duration = Duration::from_secs(8);
const NATIVE_FRAME_QUEUE_CAPACITY: usize = 1;
const SCREEN_CAPTURE_KIT_QUEUE_DEPTH: isize = 3;
const CAPTURE_FRAME_RATE: i32 = 60;

struct NativeFrame {
    captured_at_ns: u64,
    width: u32,
    height: u32,
    rgba: Box<[u8]>,
}

#[derive(Debug, Clone)]
struct StreamOutputVars {
    frames: SyncSender<NativeFrame>,
    failure: Arc<Mutex<Option<String>>>,
    running: Arc<AtomicBool>,
    clock_origin: Instant,
}

impl StreamOutputVars {
    fn fail(&self, message: impl Into<String>) {
        let Ok(mut failure) = self.failure.lock() else {
            return;
        };
        if failure.is_none() {
            *failure = Some(message.into());
        }
    }

    fn capture(&self, sample_buffer: &CMSampleBuffer, output_type: SCStreamOutputType) {
        if output_type != SCStreamOutputType::Screen || !self.running.load(Ordering::Acquire) {
            return;
        }
        let frame = unsafe { copy_bgra_frame(sample_buffer, self.clock_origin) };
        match frame {
            Ok(frame) => {
                if self.running.load(Ordering::Acquire) {
                    let _ = self.frames.try_send(frame);
                }
            }
            Err(message) => self.fail(message),
        }
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ClippyScreenCaptureKitOutput"]
    #[ivars = StreamOutputVars]
    #[derive(Debug)]
    struct StreamOutput;

    unsafe impl SCStreamOutput for StreamOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        #[allow(non_snake_case)]
        unsafe fn stream_didOutputSampleBuffer_ofType(
            &self,
            _stream: &SCStream,
            sample_buffer: &CMSampleBuffer,
            output_type: SCStreamOutputType,
        ) {
            self.ivars().capture(sample_buffer, output_type);
        }
    }

    unsafe impl SCStreamDelegate for StreamOutput {
        #[unsafe(method(stream:didStopWithError:))]
        #[allow(non_snake_case)]
        unsafe fn stream_didStopWithError(&self, _stream: &SCStream, error: &NSError) {
            self.ivars().running.store(false, Ordering::Release);
            self.ivars().fail(format!(
                "ScreenCaptureKit stream stopped: {}",
                error.localizedDescription()
            ));
        }
    }
);

unsafe impl NSObjectProtocol for StreamOutput {}

impl StreamOutput {
    fn new(vars: StreamOutputVars) -> Retained<Self> {
        let this = Self::alloc().set_ivars(vars);
        unsafe { objc2::msg_send![super(this), init] }
    }
}

pub(in crate::recording) struct MacScreenCaptureKitRegionFrameSource {
    stream: Retained<SCStream>,
    _output: Retained<StreamOutput>,
    _queue: DispatchRetained<DispatchQueue>,
    frames: Receiver<NativeFrame>,
    failure: Arc<Mutex<Option<String>>>,
    running_flag: Arc<AtomicBool>,
    descriptor: RecordingSourceDescriptor,
    clock_origin: Instant,
    minimum_timestamp_ns: u64,
    last_timestamp_ns: Option<u64>,
    next_sequence: u64,
    first_frame_since_start: bool,
    started_at: Instant,
    running: bool,
}

impl MacScreenCaptureKitRegionFrameSource {
    pub(super) fn connect(
        plan: MacRegionFrameSourcePlan,
        control_target: RecordingControlTarget,
    ) -> Result<Self, MacFrameSourceError> {
        let window_id = match control_target {
            RecordingControlTarget::NativeWindow(window_id) => {
                u32::try_from(window_id).map_err(|_| MacFrameSourceError::ControlTargetOverflow)?
            }
            RecordingControlTarget::NoNativeWindow => {
                return Err(MacFrameSourceError::ControlTargetMissing);
            }
        };
        let current = MacRegionFrameSourcePlan::prepare(plan.selection)?;
        if current.capture_region != plan.capture_region || current.descriptor != plan.descriptor {
            return Err(super::RegionFrameError::MonitorGeometryChanged.into());
        }

        let content = request_shareable_content()?;
        let display = unique_display(&content, plan.selection.monitor_id)?;
        let excluded_window = unique_window(&content, window_id)?;
        let excluded = NSArray::from_slice(&[excluded_window.as_ref()]);
        let filter = unsafe {
            SCContentFilter::initWithDisplay_excludingWindows(
                SCContentFilter::alloc(),
                &display,
                &excluded,
            )
        };
        let configuration = unsafe { SCStreamConfiguration::new() };
        let region = plan.capture_region;
        unsafe {
            configuration.setWidth(plan.selection.crop_width as usize);
            configuration.setHeight(plan.selection.crop_height as usize);
            configuration.setMinimumFrameInterval(CMTime::new(1, CAPTURE_FRAME_RATE));
            configuration.setPixelFormat(kCVPixelFormatType_32BGRA);
            configuration.setShowsCursor(true);
            configuration.setSourceRect(CGRect::new(
                CGPoint::new(region.x_points, region.y_points_from_top),
                CGSize::new(region.width_points, region.height_points),
            ));
            configuration.setQueueDepth(SCREEN_CAPTURE_KIT_QUEUE_DEPTH);
        }

        let (frame_tx, frames) = mpsc::sync_channel(NATIVE_FRAME_QUEUE_CAPACITY);
        let failure = Arc::new(Mutex::new(None));
        let running_flag = Arc::new(AtomicBool::new(true));
        let clock_origin = Instant::now();
        let output = StreamOutput::new(StreamOutputVars {
            frames: frame_tx,
            failure: Arc::clone(&failure),
            running: Arc::clone(&running_flag),
            clock_origin,
        });
        let delegate: &ProtocolObject<dyn SCStreamDelegate> = ProtocolObject::from_ref(&*output);
        let stream = unsafe {
            SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                &filter,
                &configuration,
                Some(delegate),
            )
        };
        let queue = DispatchQueue::new(
            "com.clippy.recording.screencapturekit",
            DispatchQueueAttr::SERIAL,
        );
        let output_protocol: &ProtocolObject<dyn SCStreamOutput> =
            ProtocolObject::from_ref(&*output);
        unsafe {
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    output_protocol,
                    SCStreamOutputType::Screen,
                    Some(&queue),
                )
                .map_err(|error| {
                    MacFrameSourceError::Initialize(error.localizedDescription().to_string())
                })?;
        }
        if let Err(error) = set_stream_running(&stream, true) {
            running_flag.store(false, Ordering::Release);
            return Err(error);
        }

        Ok(Self {
            stream,
            _output: output,
            _queue: queue,
            frames,
            failure,
            running_flag,
            descriptor: plan.descriptor,
            clock_origin,
            minimum_timestamp_ns: 0,
            last_timestamp_ns: None,
            next_sequence: 0,
            first_frame_since_start: true,
            started_at: Instant::now(),
            running: true,
        })
    }

    pub fn descriptor(&self) -> &RecordingSourceDescriptor {
        &self.descriptor
    }

    fn take_failure(&self) -> Result<Option<String>, MacFrameSourceError> {
        self.failure
            .lock()
            .map(|failure| failure.clone())
            .map_err(|_| MacFrameSourceError::BridgePoisoned)
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

    fn discard_frames(&self) {
        while self.frames.try_recv().is_ok() {}
    }

    fn set_running(&mut self, running: bool) -> Result<u64, MacFrameSourceError> {
        if self.running != running {
            if running {
                self.running_flag.store(true, Ordering::Release);
                if let Err(error) = set_stream_running(&self.stream, true) {
                    self.running_flag.store(false, Ordering::Release);
                    return Err(error);
                }
                self.started_at = Instant::now();
                self.first_frame_since_start = true;
            } else {
                self.running_flag.store(false, Ordering::Release);
                set_stream_running(&self.stream, false)?;
            }
            self.running = running;
        }
        let timestamp = self.next_timestamp_ns()?;
        self.last_timestamp_ns = Some(timestamp);
        self.minimum_timestamp_ns = timestamp;
        self.discard_frames();
        Ok(timestamp)
    }

    fn take_frame(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedFrame>, MacFrameSourceError> {
        if let Some(message) = self.take_failure()? {
            return Err(MacFrameSourceError::Control(message));
        }
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                if self.first_frame_since_start && self.started_at.elapsed() >= FIRST_FRAME_TIMEOUT
                {
                    return Err(MacFrameSourceError::FirstFrameTimeout);
                }
                return Ok(None);
            }
            let native = match self.frames.recv_timeout(remaining) {
                Ok(frame) => frame,
                Err(RecvTimeoutError::Timeout) => {
                    if self.first_frame_since_start
                        && self.started_at.elapsed() >= FIRST_FRAME_TIMEOUT
                    {
                        return Err(MacFrameSourceError::FirstFrameTimeout);
                    }
                    return Ok(None);
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(MacFrameSourceError::StreamClosed);
                }
            };
            if native.captured_at_ns < self.minimum_timestamp_ns {
                continue;
            }
            if native.width != self.descriptor.width || native.height != self.descriptor.height {
                return Err(super::RegionFrameError::MonitorGeometryChanged.into());
            }
            self.first_frame_since_start = false;
            let sequence = self.next_sequence;
            self.next_sequence = sequence
                .checked_add(1)
                .ok_or(MacFrameSourceError::SequenceExhausted)?;
            self.last_timestamp_ns = Some(native.captured_at_ns);
            let frame = CapturedFrame {
                sequence,
                captured_at_ns: native.captured_at_ns,
                width: native.width,
                height: native.height,
                stride: native.width * 4,
                rgba: native.rgba,
            };
            frame.validate()?;
            return Ok(Some(frame));
        }
    }
}

impl RecordingFrameSource for MacScreenCaptureKitRegionFrameSource {
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

impl Drop for MacScreenCaptureKitRegionFrameSource {
    fn drop(&mut self) {
        self.running_flag.store(false, Ordering::Release);
        if self.running {
            let _ = set_stream_running(&self.stream, false);
            self.running = false;
        }
    }
}

fn request_shareable_content() -> Result<Retained<SCShareableContent>, MacFrameSourceError> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let completion = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            let message =
                unsafe { error.as_ref() }.map(|error| error.localizedDescription().to_string());
            let retained = unsafe { Retained::retain(content) };
            let raw = retained
                .map(Retained::into_raw)
                .map(|pointer| pointer as usize);
            let _ = sender.try_send((raw, message));
        },
    );
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            false,
            false,
            &completion,
        );
    }
    let (raw, message) = receiver
        .recv_timeout(ASYNC_OPERATION_TIMEOUT)
        .map_err(|_| {
            MacFrameSourceError::Initialize(
                "等待 SCShareableContent 超时或回调已经断开".to_string(),
            )
        })?;
    if let Some(message) = message {
        return Err(MacFrameSourceError::Initialize(message));
    }
    let raw = raw.ok_or_else(|| {
        MacFrameSourceError::Initialize("SCShareableContent 返回空内容".to_string())
    })?;
    unsafe { Retained::from_raw(raw as *mut SCShareableContent) }.ok_or_else(|| {
        MacFrameSourceError::Initialize("SCShareableContent 返回无效内容".to_string())
    })
}

fn unique_display(
    content: &SCShareableContent,
    display_id: u32,
) -> Result<Retained<SCDisplay>, MacFrameSourceError> {
    let displays = unsafe { content.displays() };
    let mut matches = displays
        .to_vec()
        .into_iter()
        .filter(|display| unsafe { display.displayID() == display_id });
    let display = matches.next().ok_or(MacFrameSourceError::MonitorMissing)?;
    if matches.next().is_some() {
        return Err(MacFrameSourceError::MonitorMissing);
    }
    Ok(display)
}

fn unique_window(
    content: &SCShareableContent,
    window_id: u32,
) -> Result<Retained<SCWindow>, MacFrameSourceError> {
    let windows = unsafe { content.windows() };
    let mut matches = windows
        .to_vec()
        .into_iter()
        .filter(|window| unsafe { window.windowID() == window_id });
    let window = matches.next().ok_or_else(|| {
        MacFrameSourceError::Initialize(
            "SCShareableContent 未找到本次 Rust 创建的控制窗".to_string(),
        )
    })?;
    if matches.next().is_some() {
        return Err(MacFrameSourceError::Initialize(
            "SCShareableContent 中的控制窗 ID 不唯一".to_string(),
        ));
    }
    Ok(window)
}

fn set_stream_running(stream: &SCStream, running: bool) -> Result<(), MacFrameSourceError> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let completion = RcBlock::new(move |error: *mut NSError| {
        let result = unsafe { error.as_ref() }
            .map(|error| Err(error.localizedDescription().to_string()))
            .unwrap_or(Ok(()));
        let _ = sender.try_send(result);
    });
    unsafe {
        if running {
            stream.startCaptureWithCompletionHandler(Some(&completion));
        } else {
            stream.stopCaptureWithCompletionHandler(Some(&completion));
        }
    }
    match receiver.recv_timeout(ASYNC_OPERATION_TIMEOUT) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => Err(MacFrameSourceError::Control(message)),
        Err(_) => Err(MacFrameSourceError::Control(format!(
            "等待 ScreenCaptureKit {}超时或回调已经断开",
            if running { "启动" } else { "停止" }
        ))),
    }
}

/// 回调期间锁定 IOSurface 并立即复制 BGRA；返回前总会解锁，Objective-C 对象不会离开系统队列。
unsafe fn copy_bgra_frame(
    sample_buffer: &CMSampleBuffer,
    clock_origin: Instant,
) -> Result<NativeFrame, String> {
    let pixel_buffer = CMSampleBuffer::image_buffer(sample_buffer)
        .ok_or_else(|| "ScreenCaptureKit 帧缺少 CVPixelBuffer".to_string())?;
    let lock_status = CVPixelBufferLockBaseAddress(&pixel_buffer, CVPixelBufferLockFlags::ReadOnly);
    if lock_status != kCVReturnSuccess {
        return Err(format!(
            "ScreenCaptureKit 无法锁定 BGRA 缓冲区: {lock_status}"
        ));
    }
    let result = (|| {
        let format = CVPixelBufferGetPixelFormatType(&pixel_buffer);
        if format != kCVPixelFormatType_32BGRA {
            return Err(format!("ScreenCaptureKit 返回意外像素格式 0x{format:08X}"));
        }
        let width = CVPixelBufferGetWidth(&pixel_buffer);
        let height = CVPixelBufferGetHeight(&pixel_buffer);
        let bytes_per_row = CVPixelBufferGetBytesPerRow(&pixel_buffer);
        let base = CVPixelBufferGetBaseAddress(&pixel_buffer);
        let row_bytes = width
            .checked_mul(4)
            .ok_or_else(|| "ScreenCaptureKit 帧宽度溢出".to_string())?;
        let source_len = bytes_per_row
            .checked_mul(height)
            .ok_or_else(|| "ScreenCaptureKit 帧缓冲区长度溢出".to_string())?;
        let output_len = row_bytes
            .checked_mul(height)
            .ok_or_else(|| "ScreenCaptureKit RGBA 长度溢出".to_string())?;
        if base.is_null() || width == 0 || height == 0 || bytes_per_row < row_bytes {
            return Err("ScreenCaptureKit 返回无效 BGRA 缓冲区".to_string());
        }
        let source = unsafe { slice::from_raw_parts(base.cast::<u8>(), source_len) };
        let mut rgba = vec![0_u8; output_len];
        for row in 0..height {
            let source_start = row * bytes_per_row;
            let output_start = row * row_bytes;
            let source_row = &source[source_start..source_start + row_bytes];
            let output_row = &mut rgba[output_start..output_start + row_bytes];
            for column in 0..width {
                let pixel = column * 4;
                output_row[pixel] = source_row[pixel + 2];
                output_row[pixel + 1] = source_row[pixel + 1];
                output_row[pixel + 2] = source_row[pixel];
                output_row[pixel + 3] = source_row[pixel + 3];
            }
        }
        Ok(NativeFrame {
            captured_at_ns: clock_origin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64,
            width: u32::try_from(width)
                .map_err(|_| "ScreenCaptureKit 帧宽度超出 u32".to_string())?,
            height: u32::try_from(height)
                .map_err(|_| "ScreenCaptureKit 帧高度超出 u32".to_string())?,
            rgba: rgba.into_boxed_slice(),
        })
    })();
    let unlock_status =
        CVPixelBufferUnlockBaseAddress(&pixel_buffer, CVPixelBufferLockFlags::ReadOnly);
    if unlock_status != kCVReturnSuccess && result.is_ok() {
        return Err(format!(
            "ScreenCaptureKit 无法解锁 BGRA 缓冲区: {unlock_status}"
        ));
    }
    result
}
