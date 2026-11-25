//! 录屏会话的持久化基础。
//!
//! X11、Windows 与 macOS 专用 QA 帧源、VP9 原型和控制宿主已经分层接入。可信开始入口只在
//! 对应显式 feature 与受支持的原生会话开放；默认构建继续保持关闭。

use clippy_media::recording::{
    audio, audio_mixer, audio_worker, clock, frame, manifest, segmenting, session, worker,
};
#[cfg(test)]
mod test_artifacts;
// 独立 Recording 覆盖层通过这里完成可信选区交接；跨平台门控仍让部分构建不引用全部类型。
#[allow(dead_code)]
mod selection;
// 原生设备身份由短期 caller 绑定目录封装，WebView 只接触不透明 token 与显示名。
#[allow(dead_code)]
mod audio_devices;
#[cfg(feature = "recording-opus-webm")]
use clippy_media::recording::av_session;
#[cfg(all(test, feature = "recording-opus-webm"))]
use clippy_media::recording::{av_segmenting, pipeline};
// 单活动录屏注册表固定代次与迟到命令语义。
#[allow(dead_code)]
mod manager;
// Portal 等系统授权等待由后端令牌取消；WebView 只凭受限 caller label 触发。
mod authorization;
// 恢复 remux 占用单一大文件 I/O 槽，避免多个会话同时挤压磁盘和内存。
#[cfg(feature = "recording-vp9-prototype")]
mod merge_registry;
// 首帧缩略图使用独立私有缓存；默认构建仍编译清理路径，VP9 解码只在显式 feature 下进入图。
mod thumbnail;
// 截图选区、桌面恢复、控制面与会话注册表必须由一个生命周期按顺序交接。
#[allow(dead_code)]
mod lifecycle;
// 控制窗只从后端 registry 取得 exact generation token；前端不提交可伪造 token。
#[allow(dead_code)]
mod control_registry;
// Tauri 宿主接独立 Recording 覆盖层入口与控制面；产品策略仍按平台门槛关闭入口。
pub(crate) mod control_host;
// 完整输出与中断分段通过独立结果库导出；WebView 不接触应用数据路径。
pub(crate) mod library;
// 录屏结果只通过结果窗专属的有界 Range 协议读取；WebView 不接触真实路径。
pub(crate) mod media_protocol;
// 控制窗是否可见必须先通过平台排除能力与物理几何规划；窗口宿主接入前先固定纯函数合同。
#[allow(dead_code)]
mod control_window;
// 平台原生窗口排除与几何后备必须共享同一能力结论，避免旧系统误调用新 API。
mod control_exclusion;
#[cfg(feature = "recording-vp9-prototype")]
use clippy_media::recording::mux;
// 平台帧源先在各自系统内验证；尚未接入录屏产品会话。
#[allow(dead_code)]
mod platform;
// 工程基准直接串起真实 X11 帧源、三槽 pipeline、分段与最终 VP9。
#[cfg(all(target_os = "linux", feature = "recording-vp9-prototype"))]
mod benchmark;

pub(crate) use audio_devices::RecordingAudioDeviceCatalog;
#[cfg(all(target_os = "linux", feature = "recording-vp9-prototype"))]
pub use benchmark::X11Vp9BenchmarkReport;
#[cfg(all(target_os = "linux", feature = "recording-vp9-prototype"))]
pub(crate) use benchmark::{run_x11_vp9_benchmark, X11Vp9BenchmarkOptions};
pub(crate) use control_host::handle_control_destroyed;
pub(crate) use control_registry::RecordingControlRegistry;
pub(crate) use lifecycle::RecordingLifecycle;
pub(crate) use media_protocol::RecordingMediaManager;
#[cfg(feature = "recording-vp9-prototype")]
pub(crate) use merge_registry::RecordingMergeRegistry;
#[cfg(feature = "recording-vp9-prototype")]
pub(crate) use mux::vp9_webm::Vp9WebmWriter;
pub(crate) use thumbnail::RecordingThumbnailManager;

/// 当前阶段只允许显式 QA feature 的原生 X11、Wayland、Windows 与 macOS 12.3+ 构建进入产品选区。
/// 这是 QA 入口，不改变默认发布 feature；Wayland 还额外要求专用 feature。
pub(crate) fn product_entry_available() -> bool {
    product_entry_available_for(crate::platform::current_session())
}

pub(crate) fn wayland_tray_controls_available() -> bool {
    cfg!(all(target_os = "linux", feature = "recording-wayland-qa"))
        && crate::platform::current_session() == crate::platform::DesktopSession::Wayland
}

fn product_entry_available_for(session: crate::platform::DesktopSession) -> bool {
    if !cfg!(feature = "recording-vp9-prototype") {
        return false;
    }
    match session {
        crate::platform::DesktopSession::X11 => cfg!(target_os = "linux"),
        crate::platform::DesktopSession::Wayland => {
            cfg!(all(target_os = "linux", feature = "recording-wayland-qa"))
        }
        crate::platform::DesktopSession::Native => {
            cfg!(target_os = "windows") || macos_screencapturekit_runtime_available()
        }
        crate::platform::DesktopSession::Unknown => false,
    }
}

#[cfg(all(target_os = "macos", feature = "recording-macos-screencapturekit"))]
fn macos_screencapturekit_runtime_available() -> bool {
    let version = objc2_foundation::NSProcessInfo::processInfo().operatingSystemVersion();
    macos_version_supports_screencapturekit(version.majorVersion, version.minorVersion)
}

#[cfg(all(target_os = "macos", feature = "recording-macos-av-qa"))]
pub(in crate::recording) fn macos_screencapturekit_audio_runtime_available() -> bool {
    let version = objc2_foundation::NSProcessInfo::processInfo().operatingSystemVersion();
    macos_version_supports_screencapturekit_audio(version.majorVersion, version.minorVersion)
}

#[cfg(all(target_os = "macos", feature = "recording-macos-av-qa"))]
pub(in crate::recording) fn macos_screencapturekit_microphone_runtime_available() -> bool {
    let version = objc2_foundation::NSProcessInfo::processInfo().operatingSystemVersion();
    macos_version_supports_screencapturekit_microphone(version.majorVersion, version.minorVersion)
}

#[cfg(all(test, not(all(target_os = "macos", feature = "recording-macos-av-qa"))))]
pub(in crate::recording) const fn macos_screencapturekit_audio_runtime_available() -> bool {
    false
}

#[cfg(all(test, not(all(target_os = "macos", feature = "recording-macos-av-qa"))))]
pub(in crate::recording) const fn macos_screencapturekit_microphone_runtime_available() -> bool {
    false
}

#[cfg(not(all(target_os = "macos", feature = "recording-macos-screencapturekit")))]
const fn macos_screencapturekit_runtime_available() -> bool {
    false
}

#[cfg(any(
    test,
    all(target_os = "macos", feature = "recording-macos-screencapturekit")
))]
const fn macos_version_supports_screencapturekit(major: isize, minor: isize) -> bool {
    major > 12 || (major == 12 && minor >= 3)
}

#[cfg(any(test, all(target_os = "macos", feature = "recording-macos-av-qa")))]
const fn macos_version_supports_screencapturekit_audio(major: isize, _minor: isize) -> bool {
    major >= 13
}

#[cfg(any(test, all(target_os = "macos", feature = "recording-macos-av-qa")))]
const fn macos_version_supports_screencapturekit_microphone(major: isize, _minor: isize) -> bool {
    major >= 15
}

#[cfg(test)]
mod product_entry_tests {
    use super::{
        macos_version_supports_screencapturekit, macos_version_supports_screencapturekit_audio,
        macos_version_supports_screencapturekit_microphone, product_entry_available_for,
    };
    use crate::platform::DesktopSession;

    #[test]
    fn product_entry_policy_rejects_unknown_sessions() {
        assert!(!product_entry_available_for(DesktopSession::Unknown));
    }

    #[test]
    fn x11_and_native_sessions_require_their_explicit_platform_builds() {
        assert_eq!(
            product_entry_available_for(DesktopSession::X11),
            cfg!(all(
                feature = "recording-vp9-prototype",
                target_os = "linux"
            ))
        );
        assert_eq!(
            product_entry_available_for(DesktopSession::Wayland),
            cfg!(all(feature = "recording-wayland-qa", target_os = "linux"))
        );
        assert_eq!(
            product_entry_available_for(DesktopSession::Native),
            cfg!(all(
                feature = "recording-vp9-prototype",
                target_os = "windows"
            )) || cfg!(all(
                target_os = "macos",
                feature = "recording-macos-screencapturekit"
            ))
        );
    }

    #[test]
    fn macos_runtime_policy_starts_at_12_3() {
        assert!(!macos_version_supports_screencapturekit(11, 7));
        assert!(!macos_version_supports_screencapturekit(12, 2));
        assert!(macos_version_supports_screencapturekit(12, 3));
        assert!(macos_version_supports_screencapturekit(13, 0));
    }

    #[test]
    fn macos_audio_runtime_policy_starts_at_13() {
        assert!(!macos_version_supports_screencapturekit_audio(12, 6));
        assert!(macos_version_supports_screencapturekit_audio(13, 0));
        assert!(macos_version_supports_screencapturekit_audio(15, 4));
    }

    #[test]
    fn macos_microphone_runtime_policy_starts_at_15() {
        assert!(!macos_version_supports_screencapturekit_microphone(12, 6));
        assert!(!macos_version_supports_screencapturekit_microphone(13, 0));
        assert!(!macos_version_supports_screencapturekit_microphone(14, 7));
        assert!(macos_version_supports_screencapturekit_microphone(15, 0));
        assert!(macos_version_supports_screencapturekit_microphone(16, 0));
    }
}

pub(crate) use clippy_media::recording::recover_interrupted_sessions;
#[cfg(all(test, target_os = "linux"))]
mod native_x11_tests;
