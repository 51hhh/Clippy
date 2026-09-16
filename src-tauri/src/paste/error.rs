//! 自动粘贴领域错误。
//!
//! `Display` 文案与结构化前的字符串保持一致，command 层继续 `.map_err(|e| e.to_string())`，
//! 因此对前端可见的错误文案没有变化；`code()` 提供稳定分类，用于日志聚合与后续策略判断。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PasteError {
    // ── X11 ──
    #[error("X11 screen 不存在")]
    X11ScreenMissing,
    #[error("X11 活动窗口为空")]
    X11ActiveWindowEmpty,
    #[error("X11 atom 不存在: {0}")]
    X11AtomMissing(String),
    #[error("X11 窗口管理器未恢复原活动窗口，已取消按键注入")]
    X11FocusNotRestored,
    #[error("没有可恢复的 X11 目标窗口")]
    X11TargetMissing,
    #[error("{0}")]
    X11Protocol(String),
    #[error("X11 粘贴线程异常: {0}")]
    X11ThreadPanic(String),

    // ── Windows / macOS ──
    #[error("没有可恢复的原生目标窗口")]
    NativeTargetMissing,
    #[error("原生目标窗口已经失效")]
    NativeTargetInvalid,
    #[error("无法读取 Windows 进程完整性级别: {0}")]
    WindowsIntegrityQuery(String),
    #[error("Windows 拒绝向更高完整性进程注入输入（当前 {current_rid:#x}，目标 {target_rid:#x}）")]
    WindowsIntegrityBoundary { current_rid: u32, target_rid: u32 },
    #[error("无法恢复原生目标窗口焦点: {0}")]
    NativeFocusNotRestored(String),
    #[error("macOS 辅助功能权限尚未授予")]
    MacosAccessibilityPermissionRequired,
    #[error("原生粘贴线程异常: {0}")]
    NativeThreadPanic(String),

    // ── 按键注入 ──
    #[error("初始化 enigo 失败: {0}")]
    InputBackendUnavailable(String),
    #[error("{action}失败: {detail}")]
    KeyInjection { action: String, detail: String },

    // ── Wayland Portal ──
    #[error("RemoteDesktop Portal 会话未建立")]
    PortalSessionMissing,
    #[error("连接 RemoteDesktop Portal 失败: {0}")]
    PortalConnect(String),
    #[error("创建 RemoteDesktop 会话失败: {0}")]
    PortalCreateSession(String),
    #[error("请求键盘控制失败: {0}")]
    PortalSelectDevices(String),
    #[error("键盘控制请求被拒绝: {0}")]
    PortalSelectDevicesRejected(String),
    #[error("启动 RemoteDesktop 会话失败: {0}")]
    PortalStart(String),
    #[error("RemoteDesktop 授权未通过: {0}")]
    PortalStartRejected(String),
    #[error("RemoteDesktop Portal 未授予键盘控制权限")]
    PortalKeyboardNotGranted,
    #[error("{0}")]
    PortalAttemptExhausted(String),

    // ── restore token ──
    #[error("Portal restore token 长度无效")]
    TokenInvalidLength,
    #[error("Portal token 路径没有父目录")]
    TokenPathMissingParent,
    #[error("{0}")]
    TokenIo(String),
}

impl PasteError {
    /// 稳定错误分类。与 `Display` 文案解耦，文案调整不会破坏日志聚合。
    pub fn code(&self) -> &'static str {
        match self {
            Self::X11ScreenMissing => "x11_screen_missing",
            Self::X11ActiveWindowEmpty => "x11_active_window_empty",
            Self::X11AtomMissing(_) => "x11_atom_missing",
            Self::X11FocusNotRestored => "x11_focus_not_restored",
            Self::X11TargetMissing => "x11_target_missing",
            Self::X11Protocol(_) => "x11_protocol",
            Self::X11ThreadPanic(_) => "x11_thread_panic",
            Self::NativeTargetMissing => "native_target_missing",
            Self::NativeTargetInvalid => "native_target_invalid",
            Self::WindowsIntegrityQuery(_) => "windows_integrity_query_failed",
            Self::WindowsIntegrityBoundary { .. } => "windows_integrity_boundary",
            Self::NativeFocusNotRestored(_) => "native_focus_not_restored",
            Self::MacosAccessibilityPermissionRequired => "macos_accessibility_permission_required",
            Self::NativeThreadPanic(_) => "native_thread_panic",
            Self::InputBackendUnavailable(_) => "input_backend_unavailable",
            Self::KeyInjection { .. } => "key_injection",
            Self::PortalSessionMissing => "portal_session_missing",
            Self::PortalConnect(_) => "portal_connect",
            Self::PortalCreateSession(_) => "portal_create_session",
            Self::PortalSelectDevices(_) => "portal_select_devices",
            Self::PortalSelectDevicesRejected(_) => "portal_select_devices_rejected",
            Self::PortalStart(_) => "portal_start",
            Self::PortalStartRejected(_) => "portal_start_rejected",
            Self::PortalKeyboardNotGranted => "portal_keyboard_not_granted",
            Self::PortalAttemptExhausted(_) => "portal_attempt_exhausted",
            Self::TokenInvalidLength => "token_invalid_length",
            Self::TokenPathMissingParent => "token_path_missing_parent",
            Self::TokenIo(_) => "token_io",
        }
    }

    /// 该失败是否源自用户尚未授权，而非环境不可用。设置页据此决定是否提供显式重试。
    pub fn is_authorization_failure(&self) -> bool {
        matches!(
            self,
            Self::PortalSelectDevicesRejected(_)
                | Self::PortalStartRejected(_)
                | Self::PortalKeyboardNotGranted
                | Self::PortalAttemptExhausted(_)
                | Self::MacosAccessibilityPermissionRequired
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_matches_pre_refactor_messages() {
        assert_eq!(
            PasteError::X11ScreenMissing.to_string(),
            "X11 screen 不存在"
        );
        assert_eq!(
            PasteError::X11FocusNotRestored.to_string(),
            "X11 窗口管理器未恢复原活动窗口，已取消按键注入"
        );
        assert_eq!(
            PasteError::InputBackendUnavailable("boom".to_string()).to_string(),
            "初始化 enigo 失败: boom"
        );
        assert_eq!(
            PasteError::KeyInjection {
                action: "按下 Control".to_string(),
                detail: "boom".to_string(),
            }
            .to_string(),
            "按下 Control失败: boom"
        );
        assert_eq!(
            PasteError::PortalKeyboardNotGranted.to_string(),
            "RemoteDesktop Portal 未授予键盘控制权限"
        );
        assert_eq!(
            PasteError::TokenInvalidLength.to_string(),
            "Portal restore token 长度无效"
        );
    }

    #[test]
    fn codes_are_stable_and_unique() {
        let errors = [
            PasteError::X11ScreenMissing,
            PasteError::X11ActiveWindowEmpty,
            PasteError::X11AtomMissing(String::new()),
            PasteError::X11FocusNotRestored,
            PasteError::X11TargetMissing,
            PasteError::X11Protocol(String::new()),
            PasteError::X11ThreadPanic(String::new()),
            PasteError::NativeTargetMissing,
            PasteError::NativeTargetInvalid,
            PasteError::WindowsIntegrityQuery(String::new()),
            PasteError::WindowsIntegrityBoundary {
                current_rid: 0x2000,
                target_rid: 0x3000,
            },
            PasteError::NativeFocusNotRestored(String::new()),
            PasteError::MacosAccessibilityPermissionRequired,
            PasteError::NativeThreadPanic(String::new()),
            PasteError::InputBackendUnavailable(String::new()),
            PasteError::KeyInjection {
                action: String::new(),
                detail: String::new(),
            },
            PasteError::PortalSessionMissing,
            PasteError::PortalConnect(String::new()),
            PasteError::PortalCreateSession(String::new()),
            PasteError::PortalSelectDevices(String::new()),
            PasteError::PortalSelectDevicesRejected(String::new()),
            PasteError::PortalStart(String::new()),
            PasteError::PortalStartRejected(String::new()),
            PasteError::PortalKeyboardNotGranted,
            PasteError::PortalAttemptExhausted(String::new()),
            PasteError::TokenInvalidLength,
            PasteError::TokenPathMissingParent,
            PasteError::TokenIo(String::new()),
        ];
        let mut codes: Vec<&str> = errors.iter().map(PasteError::code).collect();
        let total = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), total, "错误码必须唯一");
    }

    #[test]
    fn only_authorization_failures_offer_explicit_retry() {
        assert!(PasteError::PortalKeyboardNotGranted.is_authorization_failure());
        assert!(PasteError::PortalStartRejected(String::new()).is_authorization_failure());
        assert!(PasteError::MacosAccessibilityPermissionRequired.is_authorization_failure());
        assert!(!PasteError::PortalConnect(String::new()).is_authorization_failure());
        assert!(!PasteError::X11ScreenMissing.is_authorization_failure());
    }
}
