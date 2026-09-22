#[cfg(target_os = "macos")]
pub(super) mod macos;
mod region;
// ScreenCaptureKit 的 PTS、PCM 布局与拆块合同在所有宿主运行测试，原生对象只进入 macOS AV feature。
#[allow(dead_code)]
mod macos_audio_contract;
#[cfg(target_os = "linux")]
pub(super) mod wayland;
#[cfg(target_os = "windows")]
pub(super) mod windows;
// WASAPI 的纯时间/拆块合同在所有宿主运行测试，原生对象只进入显式 Windows audio feature。
#[cfg(all(target_os = "windows", feature = "recording-windows-audio"))]
#[allow(dead_code)]
pub(super) mod windows_audio;
#[allow(dead_code)]
mod windows_audio_contract;
#[cfg(target_os = "linux")]
pub(super) mod x11;

use super::audio::CapturedAudioChunk;
use super::audio_worker::RecordingAudioSource;
use super::clock::RecordingSessionClock;
use super::frame::CapturedFrame;
use super::worker::RecordingFrameSource;
use crate::capture::RecordingCaptureSpec;
use thiserror::Error;

/// 平台帧源完成原生显示器核验后生成的唯一物理来源描述。
///
/// 录制清单、控制窗规划和后续编码配置必须消费这份描述，不能再用前端逻辑坐标重复推导。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordingSourceDescriptor {
    pub source_id: String,
    pub physical_x: i32,
    pub physical_y: i32,
    pub width: u32,
    pub height: u32,
}

/// 控制窗创建完成后由 Rust 桌面层交给帧源的一次性可信目标。
///
/// 原生窗口 ID 只能来自本次创建的 Tauri 控制窗，不能从 WebView 或 IPC 请求进入录屏链路。
#[derive(Clone)]
pub(super) enum RecordingControlTarget {
    NoNativeWindow,
    #[cfg(target_os = "linux")]
    WaylandPortal(wayland::WaylandPortalControlTarget),
    #[cfg(any(
        test,
        all(target_os = "macos", feature = "recording-macos-screencapturekit")
    ))]
    NativeWindow(u64),
}

impl RecordingControlTarget {
    pub const fn no_native_window() -> Self {
        Self::NoNativeWindow
    }

    #[cfg(any(
        test,
        all(target_os = "macos", feature = "recording-macos-screencapturekit")
    ))]
    pub const fn native_window(window_id: u64) -> Self {
        Self::NativeWindow(window_id)
    }

    #[cfg(target_os = "linux")]
    pub fn wayland_portal(target: wayland::WaylandPortalControlTarget) -> Self {
        Self::WaylandPortal(target)
    }
}

impl std::fmt::Debug for RecordingControlTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoNativeWindow => formatter.write_str("NoNativeWindow"),
            #[cfg(target_os = "linux")]
            Self::WaylandPortal(_) => formatter.write_str("WaylandPortal"),
            #[cfg(any(
                test,
                all(target_os = "macos", feature = "recording-macos-screencapturekit")
            ))]
            Self::NativeWindow(window_id) => formatter
                .debug_tuple("NativeWindow")
                .field(window_id)
                .finish(),
        }
    }
}

impl PartialEq for RecordingControlTarget {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::NoNativeWindow, Self::NoNativeWindow) => true,
            #[cfg(target_os = "linux")]
            (Self::WaylandPortal(left), Self::WaylandPortal(right)) => left.same_instance(right),
            #[cfg(any(
                test,
                all(target_os = "macos", feature = "recording-macos-screencapturekit")
            ))]
            (Self::NativeWindow(left), Self::NativeWindow(right)) => left == right,
            #[cfg(any(
                test,
                target_os = "linux",
                all(target_os = "macos", feature = "recording-macos-screencapturekit")
            ))]
            _ => false,
        }
    }
}

impl Eq for RecordingControlTarget {}

/// 平台对象创建前可跨线程移动的唯一采集计划。
#[derive(Debug, Clone)]
pub(super) enum PlatformFrameSourcePlan {
    #[cfg(target_os = "linux")]
    X11(x11::X11RegionFrameSourcePlan),
    #[cfg(target_os = "linux")]
    Wayland(wayland::WaylandPortalFrameSourcePlan),
    #[cfg(target_os = "windows")]
    Windows(windows::WindowsWgcFrameSourcePlan),
    #[cfg(target_os = "macos")]
    Macos(macos::MacRegionFrameSourcePlan),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlatformAudioSourceKind {
    SystemAudio,
    Microphone,
}

#[derive(Debug, Clone)]
pub(super) enum PlatformAudioSourcePlan {
    /// 只为让默认/无音频编译图保持封闭；可信入口不会构造这个分支。
    Unsupported,
    #[cfg(all(target_os = "windows", feature = "recording-windows-av-qa"))]
    Windows(windows_audio::WindowsWasapiAudioSourcePlan),
    #[cfg(all(target_os = "macos", feature = "recording-macos-av-qa"))]
    Macos(macos::MacScreenCaptureKitAudioSourcePlan),
}

impl PlatformAudioSourcePlan {
    pub const fn channels(&self) -> u16 {
        match self {
            Self::Unsupported => 0,
            #[cfg(all(target_os = "windows", feature = "recording-windows-av-qa"))]
            Self::Windows(plan) => plan.channels(),
            #[cfg(all(target_os = "macos", feature = "recording-macos-av-qa"))]
            Self::Macos(plan) => plan.channels(),
        }
    }

    pub fn connect(
        self,
        _clock: RecordingSessionClock,
    ) -> Result<PlatformAudioSource, PlatformAudioSourceError> {
        match self {
            Self::Unsupported => Err(PlatformAudioSourceError::Unsupported),
            #[cfg(all(target_os = "windows", feature = "recording-windows-av-qa"))]
            Self::Windows(plan) => Ok(PlatformAudioSource::Windows(Box::new(
                plan.connect(_clock)?,
            ))),
            #[cfg(all(target_os = "macos", feature = "recording-macos-av-qa"))]
            Self::Macos(plan) => Ok(PlatformAudioSource::Macos(Box::new(plan.connect(_clock)?))),
        }
    }
}

impl PlatformFrameSourcePlan {
    pub fn prepare(selection: RecordingCaptureSpec) -> Result<Self, PlatformFrameSourceError> {
        #[cfg(target_os = "linux")]
        {
            match crate::platform::current_session() {
                crate::platform::DesktopSession::Wayland => Ok(Self::Wayland(
                    wayland::WaylandPortalFrameSourcePlan::prepare(selection)?,
                )),
                crate::platform::DesktopSession::X11 => Ok(Self::X11(
                    x11::X11RegionFrameSourcePlan::prepare(selection)?,
                )),
                crate::platform::DesktopSession::Native
                | crate::platform::DesktopSession::Unknown => {
                    Err(PlatformFrameSourceError::UnsupportedLinuxSession)
                }
            }
        }
        #[cfg(target_os = "windows")]
        {
            Ok(Self::Windows(windows::WindowsWgcFrameSourcePlan::prepare(
                selection,
            )?))
        }
        #[cfg(target_os = "macos")]
        {
            Ok(Self::Macos(macos::MacRegionFrameSourcePlan::prepare(
                selection,
            )?))
        }
    }

    pub fn descriptor(&self) -> &RecordingSourceDescriptor {
        match self {
            #[cfg(target_os = "linux")]
            Self::X11(plan) => plan.descriptor(),
            #[cfg(target_os = "linux")]
            Self::Wayland(plan) => plan.descriptor(),
            #[cfg(target_os = "windows")]
            Self::Windows(plan) => plan.descriptor(),
            #[cfg(target_os = "macos")]
            Self::Macos(plan) => plan.descriptor(),
        }
    }

    pub fn audio_plan(
        &self,
        _kind: PlatformAudioSourceKind,
    ) -> Result<PlatformAudioSourcePlan, PlatformAudioSourceError> {
        match self {
            #[cfg(all(target_os = "windows", feature = "recording-windows-av-qa"))]
            Self::Windows(_) => Ok(PlatformAudioSourcePlan::Windows(match _kind {
                PlatformAudioSourceKind::SystemAudio => {
                    windows_audio::WindowsWasapiAudioSourcePlan::system_loopback()
                }
                PlatformAudioSourceKind::Microphone => {
                    windows_audio::WindowsWasapiAudioSourcePlan::default_microphone()
                }
            })),
            #[cfg(all(target_os = "macos", feature = "recording-macos-av-qa"))]
            Self::Macos(plan) => match _kind {
                PlatformAudioSourceKind::SystemAudio => Ok(PlatformAudioSourcePlan::Macos(
                    macos::MacScreenCaptureKitAudioSourcePlan::from_frame_plan(plan),
                )),
                PlatformAudioSourceKind::Microphone => Err(PlatformAudioSourceError::Unsupported),
            },
            #[allow(unreachable_patterns)]
            _ => Err(PlatformAudioSourceError::Unsupported),
        }
    }

    pub fn connect(
        self,
        control_target: RecordingControlTarget,
        clock: RecordingSessionClock,
    ) -> Result<PlatformFrameSource, PlatformFrameSourceError> {
        match self {
            #[cfg(target_os = "linux")]
            Self::X11(plan) => {
                debug_assert_eq!(&control_target, &RecordingControlTarget::NoNativeWindow);
                Ok(PlatformFrameSource::X11(Box::new(plan.connect(clock)?)))
            }
            #[cfg(target_os = "linux")]
            Self::Wayland(plan) => Ok(PlatformFrameSource::Wayland(Box::new(
                plan.connect(require_wayland_portal_target(control_target)?, clock)?,
            ))),
            #[cfg(target_os = "windows")]
            Self::Windows(plan) => {
                debug_assert_eq!(&control_target, &RecordingControlTarget::NoNativeWindow);
                Ok(PlatformFrameSource::Windows(Box::new(plan.connect(clock)?)))
            }
            #[cfg(target_os = "macos")]
            Self::Macos(plan) => Ok(PlatformFrameSource::Macos(Box::new(
                plan.connect(control_target, clock)?,
            ))),
        }
    }
}

#[cfg(target_os = "linux")]
fn require_wayland_portal_target(
    control_target: RecordingControlTarget,
) -> Result<wayland::WaylandPortalControlTarget, PlatformFrameSourceError> {
    match control_target {
        RecordingControlTarget::WaylandPortal(target) => Ok(target),
        RecordingControlTarget::NoNativeWindow => {
            Err(wayland::WaylandFrameSourceError::MissingAuthorizationTarget.into())
        }
        #[cfg(test)]
        RecordingControlTarget::NativeWindow(_) => {
            Err(wayland::WaylandFrameSourceError::MissingAuthorizationTarget.into())
        }
    }
}

pub(super) enum PlatformFrameSource {
    #[cfg(target_os = "linux")]
    X11(Box<x11::X11RegionFrameSource>),
    #[cfg(target_os = "linux")]
    Wayland(Box<wayland::WaylandPortalRegionFrameSource>),
    #[cfg(target_os = "windows")]
    Windows(Box<windows::WindowsWgcRegionFrameSource>),
    #[cfg(target_os = "macos")]
    Macos(Box<macos::MacRegionFrameSource>),
}

pub(super) enum PlatformAudioSource {
    Unsupported,
    #[cfg(all(target_os = "windows", feature = "recording-windows-av-qa"))]
    Windows(Box<windows_audio::WindowsWasapiAudioSource>),
    #[cfg(all(target_os = "macos", feature = "recording-macos-av-qa"))]
    Macos(Box<macos::MacScreenCaptureKitAudioSource>),
}

#[derive(Debug, Error)]
pub(super) enum PlatformFrameSourceError {
    #[cfg(target_os = "linux")]
    #[error("当前 Linux 桌面会话既不是原生 X11 也不是原生 Wayland")]
    UnsupportedLinuxSession,
    #[cfg(target_os = "linux")]
    #[error(transparent)]
    X11(#[from] x11::X11FrameSourceError),
    #[cfg(target_os = "linux")]
    #[error(transparent)]
    Wayland(#[from] wayland::WaylandFrameSourceError),
    #[cfg(target_os = "windows")]
    #[error(transparent)]
    Windows(#[from] windows::WindowsFrameSourceError),
    #[cfg(target_os = "macos")]
    #[error(transparent)]
    Macos(#[from] macos::MacFrameSourceError),
}

#[derive(Debug, Error)]
pub(super) enum PlatformAudioSourceError {
    #[error("当前平台构建不支持请求的录屏音频源")]
    Unsupported,
    #[cfg(all(target_os = "windows", feature = "recording-windows-av-qa"))]
    #[error(transparent)]
    Windows(#[from] windows_audio::WindowsWasapiAudioSourceError),
    #[cfg(all(target_os = "macos", feature = "recording-macos-av-qa"))]
    #[error(transparent)]
    Macos(#[from] macos::MacScreenCaptureKitAudioSourceError),
}

impl RecordingFrameSource for PlatformFrameSource {
    type Error = PlatformFrameSourceError;

    fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
        match self {
            #[cfg(target_os = "linux")]
            Self::X11(source) => Ok(source.capture_next()?),
            #[cfg(target_os = "linux")]
            Self::Wayland(source) => Ok(source.capture_next()?),
            #[cfg(target_os = "windows")]
            Self::Windows(source) => Ok(source.capture_next()?),
            #[cfg(target_os = "macos")]
            Self::Macos(source) => Ok(source.capture_next()?),
        }
    }

    fn capture_next_available(&mut self) -> Result<Option<CapturedFrame>, Self::Error> {
        match self {
            #[cfg(target_os = "linux")]
            Self::X11(source) => Ok(source.capture_next_available()?),
            #[cfg(target_os = "linux")]
            Self::Wayland(source) => Ok(source.capture_next_available()?),
            #[cfg(target_os = "windows")]
            Self::Windows(source) => Ok(source.capture_next_available()?),
            #[cfg(target_os = "macos")]
            Self::Macos(source) => Ok(source.capture_next_available()?),
        }
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        match self {
            #[cfg(target_os = "linux")]
            Self::X11(source) => Ok(source.control_timestamp_ns()?),
            #[cfg(target_os = "linux")]
            Self::Wayland(source) => Ok(source.control_timestamp_ns()?),
            #[cfg(target_os = "windows")]
            Self::Windows(source) => Ok(source.control_timestamp_ns()?),
            #[cfg(target_os = "macos")]
            Self::Macos(source) => Ok(source.control_timestamp_ns()?),
        }
    }

    fn pause_capture(&mut self) -> Result<u64, Self::Error> {
        match self {
            #[cfg(target_os = "linux")]
            Self::X11(source) => Ok(source.pause_capture()?),
            #[cfg(target_os = "linux")]
            Self::Wayland(source) => Ok(source.pause_capture()?),
            #[cfg(target_os = "windows")]
            Self::Windows(source) => Ok(source.pause_capture()?),
            #[cfg(target_os = "macos")]
            Self::Macos(source) => Ok(source.pause_capture()?),
        }
    }

    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        match self {
            #[cfg(target_os = "linux")]
            Self::X11(source) => Ok(source.resume_capture()?),
            #[cfg(target_os = "linux")]
            Self::Wayland(source) => Ok(source.resume_capture()?),
            #[cfg(target_os = "windows")]
            Self::Windows(source) => Ok(source.resume_capture()?),
            #[cfg(target_os = "macos")]
            Self::Macos(source) => Ok(source.resume_capture()?),
        }
    }

    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        match self {
            #[cfg(target_os = "linux")]
            Self::X11(source) => Ok(source.stop_capture()?),
            #[cfg(target_os = "linux")]
            Self::Wayland(source) => Ok(source.stop_capture()?),
            #[cfg(target_os = "windows")]
            Self::Windows(source) => Ok(source.stop_capture()?),
            #[cfg(target_os = "macos")]
            Self::Macos(source) => Ok(source.stop_capture()?),
        }
    }
}

impl RecordingAudioSource for PlatformAudioSource {
    type Error = PlatformAudioSourceError;

    fn capture_next_available(
        &mut self,
        _timeout: std::time::Duration,
    ) -> Result<Option<CapturedAudioChunk>, Self::Error> {
        match self {
            Self::Unsupported => Err(PlatformAudioSourceError::Unsupported),
            #[cfg(all(target_os = "windows", feature = "recording-windows-av-qa"))]
            Self::Windows(source) => Ok(source.capture_next_available(_timeout)?),
            #[cfg(all(target_os = "macos", feature = "recording-macos-av-qa"))]
            Self::Macos(source) => Ok(source.capture_next_available(_timeout)?),
        }
    }

    fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
        match self {
            Self::Unsupported => Err(PlatformAudioSourceError::Unsupported),
            #[cfg(all(target_os = "windows", feature = "recording-windows-av-qa"))]
            Self::Windows(source) => Ok(source.control_timestamp_ns()?),
            #[cfg(all(target_os = "macos", feature = "recording-macos-av-qa"))]
            Self::Macos(source) => Ok(source.control_timestamp_ns()?),
        }
    }

    fn pause_capture(&mut self) -> Result<u64, Self::Error> {
        match self {
            Self::Unsupported => Err(PlatformAudioSourceError::Unsupported),
            #[cfg(all(target_os = "windows", feature = "recording-windows-av-qa"))]
            Self::Windows(source) => Ok(source.pause_capture()?),
            #[cfg(all(target_os = "macos", feature = "recording-macos-av-qa"))]
            Self::Macos(source) => Ok(source.pause_capture()?),
        }
    }

    fn resume_capture(&mut self) -> Result<u64, Self::Error> {
        match self {
            Self::Unsupported => Err(PlatformAudioSourceError::Unsupported),
            #[cfg(all(target_os = "windows", feature = "recording-windows-av-qa"))]
            Self::Windows(source) => Ok(source.resume_capture()?),
            #[cfg(all(target_os = "macos", feature = "recording-macos-av-qa"))]
            Self::Macos(source) => Ok(source.resume_capture()?),
        }
    }

    fn stop_capture(&mut self) -> Result<u64, Self::Error> {
        match self {
            Self::Unsupported => Err(PlatformAudioSourceError::Unsupported),
            #[cfg(all(target_os = "windows", feature = "recording-windows-av-qa"))]
            Self::Windows(source) => Ok(source.stop_capture()?),
            #[cfg(all(target_os = "macos", feature = "recording-macos-av-qa"))]
            Self::Macos(source) => Ok(source.stop_capture()?),
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn wayland_rejects_missing_or_foreign_native_control_target() {
        for target in [
            RecordingControlTarget::NoNativeWindow,
            RecordingControlTarget::NativeWindow(42),
        ] {
            assert!(matches!(
                require_wayland_portal_target(target),
                Err(PlatformFrameSourceError::Wayland(
                    wayland::WaylandFrameSourceError::MissingAuthorizationTarget
                ))
            ));
        }
    }
}
