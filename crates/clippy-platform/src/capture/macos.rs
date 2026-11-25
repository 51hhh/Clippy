use super::{
    latest::LatestFrames,
    macos_frame::{bgra_span, bgra_to_rgba, region_layout},
};
use anyhow::{anyhow, bail, ensure, Context, Result};
use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use image::RgbaImage;
use libloading::Library;
use objc2::{
    define_class, extern_protocol, msg_send,
    rc::{autoreleasepool, Allocated, Retained},
    runtime::{AnyClass, AnyObject},
    AllocAnyThread, DefinedClass,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGDisplayBounds, CGDisplayIsActive, CGDisplayPixelsHigh, CGDisplayPixelsWide, CGDisplayRotation,
};
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::{
    kCVPixelFormatType_32BGRA, CVPixelBufferGetHeight, CVPixelBufferGetIOSurface,
    CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidth, CVPixelBufferIsPlanar,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSNumber, NSObject, NSObjectProtocol, NSOperatingSystemVersion,
    NSProcessInfo, NSString,
};
use objc2_io_surface::{IOSurface, IOSurfaceLockOptions};
use std::{
    ffi::CStr,
    ptr, slice,
    sync::{mpsc, Arc, OnceLock},
    time::{Duration, Instant},
};

const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);
const STOP_TIMEOUT: Duration = Duration::from_secs(2);
const FRAME_WAIT: Duration = Duration::from_millis(16);
const SCREEN: isize = 0;
const COMPLETE: isize = 0;
const BLANK: isize = 2;
const SUSPENDED: isize = 3;
const STARTED: isize = 4;
const STOPPED: isize = 5;
const FRAMEWORK_PATH: &str =
    "/System/Library/Frameworks/ScreenCaptureKit.framework/ScreenCaptureKit";

pub(super) fn stream_supported() -> bool {
    NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
        majorVersion: 12,
        minorVersion: 3,
        patchVersion: 0,
    })
}

fn frame_status_key() -> Result<&'static NSString> {
    // 框架对象由 Objective-C 运行时登记，进程内保持加载才能保证类和常量有效。
    static FRAMEWORK: OnceLock<Result<Library, libloading::Error>> = OnceLock::new();
    let library = FRAMEWORK
        .get_or_init(|| unsafe { Library::new(FRAMEWORK_PATH) })
        .as_ref()
        .map_err(|error| anyhow!("Loading ScreenCaptureKit: {error}"))?;
    let symbol = unsafe { library.get::<*const *const NSString>(b"SCStreamFrameInfoStatus\0")? };
    unsafe { (**symbol).as_ref() }.context("Missing ScreenCaptureKit frame status key")
}

fn class(name: &CStr) -> Result<&'static AnyClass> {
    AnyClass::get(name).with_context(|| format!("Missing ScreenCaptureKit class {name:?}"))
}

extern_protocol!(
    /// ScreenCaptureKit 的画面输出协议。
    ///
    /// # Safety
    /// 实现必须匹配 ScreenCaptureKit 的回调 ABI，样本的借用不能离开本次回调。
    #[allow(clippy::missing_safety_doc)]
    unsafe trait SCStreamOutput: NSObjectProtocol {
        /// # Safety
        /// 仅在系统提供的样本仍然有效时调用。
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn did_output(&self, stream: &NSObject, sample: &CMSampleBuffer, kind: isize);
    }
);

extern_protocol!(
    /// ScreenCaptureKit 的流终止通知协议。
    ///
    /// # Safety
    /// 实现必须匹配 ScreenCaptureKit 的回调 ABI，错误对象的借用不能离开本次回调。
    #[allow(clippy::missing_safety_doc)]
    unsafe trait SCStreamDelegate: NSObjectProtocol {
        /// # Safety
        /// 仅在系统提供的错误对象仍然有效时调用。
        #[optional]
        #[unsafe(method(stream:didStopWithError:))]
        unsafe fn did_stop(&self, stream: &NSObject, error: &NSError);
    }
);

struct FrameOutputState {
    frames: Arc<LatestFrames<Arc<SurfaceLease>>>,
    size: (u32, u32),
    status_key: &'static NSString,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ClippySlintScreenStreamOutput"]
    #[ivars = FrameOutputState]
    struct FrameOutput;

    unsafe impl SCStreamOutput for FrameOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn did_output(&self, _stream: &NSObject, sample: &CMSampleBuffer, kind: isize) {
            if kind == SCREEN {
                autoreleasepool(|_| self.ivars().capture(sample));
            }
        }
    }

    unsafe impl SCStreamDelegate for FrameOutput {
        #[unsafe(method(stream:didStopWithError:))]
        unsafe fn did_stop(&self, _stream: &NSObject, error: &NSError) {
            autoreleasepool(|_| {
                self.ivars()
                    .frames
                    .publish(Err(error.localizedDescription().to_string()))
            });
        }
    }
);

unsafe impl NSObjectProtocol for FrameOutput {}

impl FrameOutputState {
    fn capture(&self, sample: &CMSampleBuffer) {
        match sample_status(sample, self.status_key) {
            Some(COMPLETE | STARTED) => {
                if let Some(buffer) = unsafe { sample.image_buffer() } {
                    let frame = SurfaceLease::new(&buffer, self.size)
                        .map(|surface| Some(Arc::new(surface)))
                        .map_err(|error| error.to_string());
                    self.frames.publish(frame);
                }
            }
            Some(BLANK | SUSPENDED) => self.frames.publish(Ok(None)),
            Some(STOPPED) => self
                .frames
                .publish(Err("ScreenCaptureKit stream stopped".into())),
            _ => {}
        }
    }
}

fn sample_status(sample: &CMSampleBuffer, key: &NSString) -> Option<isize> {
    let attachments = unsafe { sample.sample_attachments_array(false)? };
    // CoreMedia 的附件是可桥接的 CFArray/CFDictionary，状态键对应一个 NSNumber。
    let array: &NSArray<NSDictionary<NSString, AnyObject>> =
        unsafe { &*ptr::from_ref(&*attachments).cast() };
    let status = array.firstObject()?.objectForKey(key)?;
    Some(status.downcast::<NSNumber>().ok()?.integerValue())
}

struct SurfaceLease(Retained<IOSurface>);

impl SurfaceLease {
    fn new(buffer: &objc2_core_video::CVPixelBuffer, (width, height): (u32, u32)) -> Result<Self> {
        ensure!(
            (
                CVPixelBufferGetWidth(buffer),
                CVPixelBufferGetHeight(buffer)
            ) == (width as usize, height as usize),
            "ScreenCaptureKit returned an unexpected frame size"
        );
        ensure!(
            CVPixelBufferGetPixelFormatType(buffer) == kCVPixelFormatType_32BGRA
                && !CVPixelBufferIsPlanar(buffer),
            "ScreenCaptureKit returned an unexpected pixel format"
        );
        let surface =
            CVPixelBufferGetIOSurface(Some(buffer)).context("Missing capture IOSurface")?;
        // IOSurfaceRef 与 IOSurface 可桥接。占用计数让系统在消费者读完前保留画面。
        let surface = unsafe {
            Retained::retain(ptr::from_ref(&*surface).cast_mut().cast::<IOSurface>())
                .expect("IOSurfaceRef 持有非空对象")
        };
        surface.incrementUseCount();
        Ok(Self(surface))
    }

    fn read(&self, size: (u32, u32)) -> Result<RgbaImage> {
        let surface = &self.0;
        ensure!(
            (surface.width(), surface.height()) == (size.0 as isize, size.1 as isize)
                && surface.pixelFormat() == kCVPixelFormatType_32BGRA
                && surface.planeCount() == 0,
            "ScreenCaptureKit returned an unexpected surface layout"
        );
        let stride = usize::try_from(surface.bytesPerRow())?;
        let len = bgra_span(size.0, size.1, stride)?;
        ensure!(
            len <= usize::try_from(surface.allocationSize())?,
            "ScreenCaptureKit surface is smaller than its frame"
        );
        let lock = IOSurfaceLockOptions::ReadOnly;
        let status = surface.lockWithOptions_seed(lock, ptr::null_mut());
        ensure!(
            status == 0,
            "Locking ScreenCaptureKit surface failed: {status}"
        );
        let bytes = unsafe { slice::from_raw_parts(surface.baseAddress().as_ptr().cast(), len) };
        let rgba = bgra_to_rgba(bytes, size, stride);
        let status = surface.unlockWithOptions_seed(lock, ptr::null_mut());
        ensure!(
            status == 0,
            "Unlocking ScreenCaptureKit surface failed: {status}"
        );
        Ok(rgba)
    }
}

impl Drop for SurfaceLease {
    fn drop(&mut self) {
        self.0.decrementUseCount();
    }
}

#[derive(PartialEq)]
struct DisplayState {
    bounds: CGRect,
    pixels: (usize, usize),
    rotation: f64,
}

impl DisplayState {
    fn current(id: u32) -> Self {
        Self {
            bounds: CGDisplayBounds(id),
            pixels: (CGDisplayPixelsWide(id), CGDisplayPixelsHigh(id)),
            rotation: CGDisplayRotation(id),
        }
    }
}

pub(super) struct ScreenStream {
    stream: Retained<NSObject>,
    output: Retained<FrameOutput>,
    _queue: DispatchRetained<DispatchQueue>,
    frames: Arc<LatestFrames<Arc<SurfaceLease>>>,
    id: u32,
    display: DisplayState,
    size: (u32, u32),
    seen: u64,
}

impl ScreenStream {
    pub fn open(id: u32, source: (u32, u32), bounds: [f32; 4]) -> Result<Self> {
        autoreleasepool(|_| Self::connect(id, source, bounds))
    }

    fn connect(id: u32, source: (u32, u32), bounds: [f32; 4]) -> Result<Self> {
        let status_key = frame_status_key()?;
        let deadline = Instant::now() + FIRST_FRAME_TIMEOUT;
        let display = DisplayState::current(id);
        let logical = display.bounds.size;
        let region = region_layout(source, (logical.width, logical.height), bounds)?;
        let active_display = request_display(id, deadline)?;
        let windows = NSArray::<NSObject>::new();
        let filter_class = class(c"SCContentFilter")?;
        let configuration_class = class(c"SCStreamConfiguration")?;
        let stream_class = class(c"SCStream")?;
        let filter: Retained<NSObject>;
        let configuration: Retained<NSObject>;
        unsafe {
            let filter_alloc: Allocated<NSObject> = msg_send![filter_class, alloc];
            filter = msg_send![filter_alloc, initWithDisplay: &*active_display, excludingWindows: &*windows];
            configuration = msg_send![configuration_class, new];
            let [x, y, width, height] = region.points;
            let rect = CGRect::new(CGPoint::new(x, y), CGSize::new(width, height));
            let _: () = msg_send![&*configuration, setSourceRect: rect];
            let _: () = msg_send![&*configuration, setWidth: region.pixels.0 as usize];
            let _: () = msg_send![&*configuration, setHeight: region.pixels.1 as usize];
            let _: () = msg_send![&*configuration, setPixelFormat: kCVPixelFormatType_32BGRA];
            let _: () = msg_send![&*configuration, setMinimumFrameInterval: CMTime::new(1, 30)];
            let _: () = msg_send![&*configuration, setQueueDepth: 3_isize];
            let _: () = msg_send![&*configuration, setShowsCursor: false];
        }

        let size = region.pixels;
        let frames = Arc::new(LatestFrames::new());
        let output = FrameOutput::alloc().set_ivars(FrameOutputState {
            frames: Arc::clone(&frames),
            size,
            status_key,
        });
        let output: Retained<FrameOutput> = unsafe { msg_send![super(output), init] };
        let queue = DispatchQueue::new("com.clippy.slint.screen-stream", DispatchQueueAttr::SERIAL);
        let stream: Retained<NSObject> = unsafe {
            let alloc: Allocated<NSObject> = msg_send![stream_class, alloc];
            msg_send![alloc, initWithFilter: &*filter, configuration: &*configuration, delegate: &*output]
        };
        let added: std::result::Result<(), Retained<NSError>> = unsafe {
            msg_send![&*stream, addStreamOutput: &*output, type: SCREEN, sampleHandlerQueue: &*queue, error: _]
        };
        added.map_err(|error| {
            anyhow!(
                "Adding ScreenCaptureKit output: {}",
                error.localizedDescription()
            )
        })?;

        let source = Self {
            stream,
            output,
            _queue: queue,
            frames,
            id,
            display,
            size,
            seen: 0,
        };
        request_running(&source.stream, true, deadline)?;
        source
            .frames
            .wait(0, deadline.saturating_duration_since(Instant::now()))?;
        source.check_display()?;
        Ok(source)
    }

    pub fn read(&mut self) -> Result<RgbaImage> {
        self.check_display()?;
        let (sequence, surface) = self.frames.wait(self.seen, FRAME_WAIT)?;
        let result = surface.read(self.size)?;
        self.frames.check_active()?;
        self.seen = sequence;
        Ok(result)
    }

    fn check_display(&self) -> Result<()> {
        if !CGDisplayIsActive(self.id) || self.display != DisplayState::current(self.id) {
            let message = "Capture output disconnected or changed display mode";
            self.frames.publish(Err(message.into()));
            bail!("{message}");
        }
        Ok(())
    }
}

impl Drop for ScreenStream {
    fn drop(&mut self) {
        self.frames
            .publish(Err("ScreenCaptureKit session closed".into()));
        autoreleasepool(|_| {
            let _ = request_running(&self.stream, false, Instant::now() + STOP_TIMEOUT);
            unsafe {
                let _: bool = msg_send![&*self.stream, removeStreamOutput: &*self.output, type: SCREEN, error: ptr::null_mut::<*mut NSError>()];
            }
        });
    }
}

struct ShareableDisplay(Retained<NSObject>);

// 只交接 Apple 提供的只读 SCDisplay 快照，SCStream 本身由采集线程持有。
unsafe impl Send for ShareableDisplay {}

fn request_display(id: u32, deadline: Instant) -> Result<Retained<NSObject>> {
    let shareable = class(c"SCShareableContent")?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let completion = RcBlock::new(move |content: *mut NSObject, error: *mut NSError| {
        let result = autoreleasepool(|_| unsafe {
            if let Some(error) = error.as_ref() {
                Err(anyhow!("{}", error.localizedDescription()))
            } else {
                let content = content.as_ref().context("Empty SCShareableContent")?;
                let displays: Retained<NSArray<NSObject>> = msg_send![content, displays];
                displays
                    .to_vec()
                    .into_iter()
                    .find(|display| {
                        let display_id: u32 = msg_send![&**display, displayID];
                        display_id == id
                    })
                    .map(ShareableDisplay)
                    .context("Capture output is missing from ScreenCaptureKit")
            }
        });
        let _ = sender.try_send(result);
    });
    unsafe {
        let _: () = msg_send![shareable, getShareableContentWithCompletionHandler: &*completion];
    }
    let display = receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .context("Waiting for ScreenCaptureKit displays")??;
    Ok(display.0)
}

fn request_running(stream: &NSObject, start: bool, deadline: Instant) -> Result<()> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let completion = RcBlock::new(move |error: *mut NSError| {
        let error = autoreleasepool(|_| unsafe {
            error
                .as_ref()
                .map(|error| error.localizedDescription().to_string())
        });
        let _ = sender.try_send(error);
    });
    unsafe {
        if start {
            let _: () = msg_send![stream, startCaptureWithCompletionHandler: &*completion];
        } else {
            let _: () = msg_send![stream, stopCaptureWithCompletionHandler: &*completion];
        }
    }
    let action = if start { "start" } else { "stop" };
    if let Some(error) = receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .with_context(|| format!("Waiting for ScreenCaptureKit to {action}"))?
    {
        bail!("ScreenCaptureKit {action} failed: {error}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2::ClassType;
    use objc2_core_foundation::{CFDictionary, CFRetained, CFString, CFType};
    use objc2_core_video::{
        kCVPixelBufferIOSurfacePropertiesKey, kCVReturnSuccess, CVPixelBufferCreate,
        CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferLockBaseAddress,
        CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
    };
    use std::ptr::NonNull;

    #[test]
    fn screen_capture_kit_registers_its_runtime_types_without_starting_capture() {
        if stream_supported() {
            assert!(!frame_status_key().unwrap().to_string().is_empty());
            for name in [
                c"SCShareableContent",
                c"SCContentFilter",
                c"SCStreamConfiguration",
                c"SCStream",
            ] {
                class(name).unwrap();
            }
            let _ = FrameOutput::class();
        }
    }

    #[test]
    fn iosurface_outlives_the_sample_and_releases_its_use_count() {
        let size = (3, 2);
        let properties = CFDictionary::<CFString, CFType>::empty();
        let attributes = CFDictionary::from_slices(
            &[unsafe { kCVPixelBufferIOSurfacePropertiesKey }],
            &[&*properties],
        );
        let mut buffer = ptr::null_mut();
        let buffer = unsafe {
            assert_eq!(
                CVPixelBufferCreate(
                    None,
                    size.0 as usize,
                    size.1 as usize,
                    kCVPixelFormatType_32BGRA,
                    Some(attributes.as_opaque()),
                    NonNull::from(&mut buffer),
                ),
                kCVReturnSuccess
            );
            CFRetained::from_raw(NonNull::new(buffer).unwrap())
        };
        let fill = CVPixelBufferLockFlags::empty();
        unsafe {
            assert_eq!(
                CVPixelBufferLockBaseAddress(&buffer, fill),
                kCVReturnSuccess
            );
            let base = CVPixelBufferGetBaseAddress(&buffer).cast::<u8>();
            let stride = CVPixelBufferGetBytesPerRow(&buffer);
            for (row, bgra) in [[3_u8, 2, 1, 4], [7, 6, 5, 8]].into_iter().enumerate() {
                for column in 0..size.0 as usize {
                    ptr::copy_nonoverlapping(bgra.as_ptr(), base.add(row * stride + column * 4), 4);
                }
            }
            assert_eq!(
                CVPixelBufferUnlockBaseAddress(&buffer, fill),
                kCVReturnSuccess
            );
        }
        assert!(SurfaceLease::new(&buffer, (2, 2)).is_err());
        let frame = SurfaceLease::new(&buffer, size).unwrap();
        let surface = frame.0.clone();
        drop(buffer);
        let count = surface.localUseCount();
        assert!(count > 0);
        let rgba = frame.read(size).unwrap();
        assert_eq!(rgba.get_pixel(2, 0).0, [1, 2, 3, 4]);
        assert_eq!(rgba.get_pixel(2, 1).0, [5, 6, 7, 8]);
        drop(frame);
        assert_eq!(surface.localUseCount(), count - 1);
    }
}
