//! 操作系统与桌面会话能力的单一事实源。
//!
//! 编译期只负责确定操作系统；Linux 的 X11/Wayland、桌面环境和 Portal 能力必须在
//! 运行时决定。业务模块不应再各自读取环境变量，否则 XWayland 和混合会话会被不同
//! 功能判成不同平台。

use serde::Serialize;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub(crate) use macos::{
    accessibility_trusted as macos_accessibility_trusted,
    request_accessibility_permission as request_macos_accessibility_permission,
    request_screen_capture_permission as request_macos_screen_capture_permission,
    screen_capture_trusted as macos_screen_capture_trusted,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatingSystem {
    Linux,
    Windows,
    Macos,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopSession {
    X11,
    Wayland,
    Native,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallType {
    Appimage,
    Deb,
    Windows,
    Macos,
    Development,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Available,
    PermissionRequired,
    Degraded,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityReason {
    NoDisplayServer,
    WaylandProtocolLimited,
    WaylandPortalPermission,
    NonGnomeWaylandShortcut,
    WindowsIntegrityBoundary,
    MacosScreenRecordingPermission,
    MacosAccessibilityPermission,
    OcrNotInstalled,
    UnsupportedPlatform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Capability {
    pub state: CapabilityState,
    pub reason: Option<CapabilityReason>,
}

impl Capability {
    const fn available() -> Self {
        Self {
            state: CapabilityState::Available,
            reason: None,
        }
    }

    const fn with_reason(state: CapabilityState, reason: CapabilityReason) -> Self {
        Self {
            state,
            reason: Some(reason),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlatformCapabilities {
    pub clipboard_text: Capability,
    pub clipboard_image: Capability,
    pub auto_paste: Capability,
    pub global_shortcuts: Capability,
    pub screen_capture: Capability,
    pub window_pick: Capability,
    pub absolute_window_position: Capability,
    pub always_on_top: Capability,
    pub ocr: Capability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlatformInfo {
    pub operating_system: OperatingSystem,
    pub session: DesktopSession,
    pub desktop_environment: Option<String>,
    pub architecture: String,
    pub capabilities: PlatformCapabilities,
}

pub fn current_operating_system() -> OperatingSystem {
    if cfg!(target_os = "linux") {
        OperatingSystem::Linux
    } else if cfg!(target_os = "windows") {
        OperatingSystem::Windows
    } else if cfg!(target_os = "macos") {
        OperatingSystem::Macos
    } else {
        OperatingSystem::Other
    }
}

fn install_type_from(
    operating_system: OperatingSystem,
    appimage: bool,
    development: bool,
) -> InstallType {
    if development {
        return InstallType::Development;
    }
    match operating_system {
        OperatingSystem::Linux if appimage => InstallType::Appimage,
        OperatingSystem::Linux => InstallType::Deb,
        OperatingSystem::Windows => InstallType::Windows,
        OperatingSystem::Macos => InstallType::Macos,
        OperatingSystem::Other => InstallType::Unknown,
    }
}

pub fn is_dev_binary() -> bool {
    match std::env::current_exe() {
        Ok(path) => {
            let path = path.to_string_lossy().replace('\\', "/");
            path.contains("/target/debug/") || path.contains("/target/release/")
        }
        Err(_) => false,
    }
}

pub fn current_install_type() -> InstallType {
    install_type_from(
        current_operating_system(),
        std::env::var_os("APPIMAGE").is_some(),
        is_dev_binary(),
    )
}

fn detect_session_from(
    operating_system: OperatingSystem,
    session_type: Option<&str>,
    has_wayland_display: bool,
    has_x11_display: bool,
) -> DesktopSession {
    if operating_system != OperatingSystem::Linux {
        return match operating_system {
            OperatingSystem::Windows | OperatingSystem::Macos => DesktopSession::Native,
            _ => DesktopSession::Unknown,
        };
    }

    match session_type
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("wayland") => DesktopSession::Wayland,
        Some("x11") => DesktopSession::X11,
        _ if has_wayland_display => DesktopSession::Wayland,
        _ if has_x11_display => DesktopSession::X11,
        _ => DesktopSession::Unknown,
    }
}

pub fn current_session() -> DesktopSession {
    detect_session_from(
        current_operating_system(),
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::var_os("DISPLAY").is_some(),
    )
}

fn normalize_desktop(desktop: Option<&str>, session: Option<&str>) -> Option<String> {
    let parts = [desktop, session]
        .into_iter()
        .flatten()
        .flat_map(|value| value.split(':'))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    if parts.iter().any(|part| part == "gnome") {
        Some("gnome".to_string())
    } else if parts
        .iter()
        .any(|part| part == "kde" || part.starts_with("plasma"))
    {
        Some("kde".to_string())
    } else {
        parts.into_iter().next()
    }
}

pub fn current_desktop_environment() -> Option<String> {
    normalize_desktop(
        std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
        std::env::var("XDG_SESSION_DESKTOP").ok().as_deref(),
    )
}

pub fn is_wayland() -> bool {
    current_session() == DesktopSession::Wayland
}

pub fn is_gnome_desktop() -> bool {
    current_desktop_environment().as_deref() == Some("gnome")
}

pub fn uses_gnome_shortcuts() -> bool {
    is_wayland()
}

fn capabilities_for(
    operating_system: OperatingSystem,
    session: DesktopSession,
    desktop_environment: Option<&str>,
) -> PlatformCapabilities {
    let available = Capability::available();
    let unsupported = Capability::with_reason(
        CapabilityState::Unsupported,
        CapabilityReason::UnsupportedPlatform,
    );

    match operating_system {
        OperatingSystem::Linux => match session {
            DesktopSession::X11 => PlatformCapabilities {
                clipboard_text: available,
                clipboard_image: available,
                auto_paste: available,
                global_shortcuts: available,
                screen_capture: available,
                window_pick: available,
                absolute_window_position: available,
                always_on_top: available,
                ocr: available,
            },
            DesktopSession::Wayland => {
                let gnome =
                    normalize_desktop(desktop_environment, None).as_deref() == Some("gnome");
                PlatformCapabilities {
                    clipboard_text: available,
                    clipboard_image: available,
                    auto_paste: Capability::with_reason(
                        CapabilityState::PermissionRequired,
                        CapabilityReason::WaylandPortalPermission,
                    ),
                    global_shortcuts: if gnome {
                        available
                    } else {
                        Capability::with_reason(
                            CapabilityState::Degraded,
                            CapabilityReason::NonGnomeWaylandShortcut,
                        )
                    },
                    screen_capture: Capability::with_reason(
                        CapabilityState::PermissionRequired,
                        CapabilityReason::WaylandPortalPermission,
                    ),
                    window_pick: Capability::with_reason(
                        CapabilityState::Degraded,
                        CapabilityReason::WaylandProtocolLimited,
                    ),
                    absolute_window_position: Capability::with_reason(
                        CapabilityState::Unsupported,
                        CapabilityReason::WaylandProtocolLimited,
                    ),
                    always_on_top: Capability::with_reason(
                        CapabilityState::Degraded,
                        CapabilityReason::WaylandProtocolLimited,
                    ),
                    ocr: available,
                }
            }
            DesktopSession::Unknown | DesktopSession::Native => PlatformCapabilities {
                clipboard_text: available,
                clipboard_image: available,
                auto_paste: Capability::with_reason(
                    CapabilityState::Unsupported,
                    CapabilityReason::NoDisplayServer,
                ),
                global_shortcuts: Capability::with_reason(
                    CapabilityState::Unsupported,
                    CapabilityReason::NoDisplayServer,
                ),
                screen_capture: Capability::with_reason(
                    CapabilityState::Unsupported,
                    CapabilityReason::NoDisplayServer,
                ),
                window_pick: Capability::with_reason(
                    CapabilityState::Unsupported,
                    CapabilityReason::NoDisplayServer,
                ),
                absolute_window_position: Capability::with_reason(
                    CapabilityState::Unsupported,
                    CapabilityReason::NoDisplayServer,
                ),
                always_on_top: Capability::with_reason(
                    CapabilityState::Unsupported,
                    CapabilityReason::NoDisplayServer,
                ),
                ocr: available,
            },
        },
        OperatingSystem::Windows => PlatformCapabilities {
            clipboard_text: available,
            clipboard_image: available,
            auto_paste: Capability::with_reason(
                CapabilityState::Degraded,
                CapabilityReason::WindowsIntegrityBoundary,
            ),
            global_shortcuts: available,
            screen_capture: available,
            window_pick: available,
            absolute_window_position: available,
            always_on_top: available,
            ocr: available,
        },
        OperatingSystem::Macos => PlatformCapabilities {
            clipboard_text: available,
            clipboard_image: available,
            auto_paste: Capability::with_reason(
                CapabilityState::PermissionRequired,
                CapabilityReason::MacosAccessibilityPermission,
            ),
            global_shortcuts: available,
            screen_capture: Capability::with_reason(
                CapabilityState::PermissionRequired,
                CapabilityReason::MacosScreenRecordingPermission,
            ),
            window_pick: Capability::with_reason(
                CapabilityState::PermissionRequired,
                CapabilityReason::MacosScreenRecordingPermission,
            ),
            absolute_window_position: available,
            always_on_top: available,
            ocr: available,
        },
        OperatingSystem::Other => PlatformCapabilities {
            clipboard_text: unsupported,
            clipboard_image: unsupported,
            auto_paste: unsupported,
            global_shortcuts: unsupported,
            screen_capture: unsupported,
            window_pick: unsupported,
            absolute_window_position: unsupported,
            always_on_top: unsupported,
            ocr: unsupported,
        },
    }
}

pub fn current_info() -> PlatformInfo {
    let operating_system = current_operating_system();
    let session = current_session();
    let desktop_environment = current_desktop_environment();
    let capabilities = capabilities_for(operating_system, session, desktop_environment.as_deref());
    #[cfg(target_os = "macos")]
    let capabilities = {
        let mut capabilities = capabilities;
        capabilities.auto_paste = if macos_accessibility_trusted() {
            Capability::available()
        } else {
            Capability::with_reason(
                CapabilityState::PermissionRequired,
                CapabilityReason::MacosAccessibilityPermission,
            )
        };
        let capture = if macos_screen_capture_trusted() {
            Capability::available()
        } else {
            Capability::with_reason(
                CapabilityState::PermissionRequired,
                CapabilityReason::MacosScreenRecordingPermission,
            )
        };
        capabilities.screen_capture = capture;
        capabilities.window_pick = capture;
        capabilities
    };
    let capabilities = with_ocr_availability(capabilities, crate::ocr::is_available());
    PlatformInfo {
        operating_system,
        session,
        capabilities,
        desktop_environment,
        architecture: std::env::consts::ARCH.to_string(),
    }
}

fn with_ocr_availability(
    mut capabilities: PlatformCapabilities,
    ocr_available: bool,
) -> PlatformCapabilities {
    if !ocr_available && capabilities.ocr.state != CapabilityState::Unsupported {
        capabilities.ocr =
            Capability::with_reason(CapabilityState::Degraded, CapabilityReason::OcrNotInstalled);
    }
    capabilities
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_linux_session_wins_over_stale_display_variables() {
        assert_eq!(
            detect_session_from(OperatingSystem::Linux, Some("x11"), true, true),
            DesktopSession::X11
        );
        assert_eq!(
            detect_session_from(OperatingSystem::Linux, Some("wayland"), false, true),
            DesktopSession::Wayland
        );
    }

    #[test]
    fn display_variables_are_only_a_fallback() {
        assert_eq!(
            detect_session_from(OperatingSystem::Linux, None, true, true),
            DesktopSession::Wayland
        );
        assert_eq!(
            detect_session_from(OperatingSystem::Linux, None, false, true),
            DesktopSession::X11
        );
        assert_eq!(
            detect_session_from(OperatingSystem::Linux, Some("tty"), false, false),
            DesktopSession::Unknown
        );
    }

    #[test]
    fn native_platforms_do_not_inherit_linux_environment() {
        assert_eq!(
            detect_session_from(OperatingSystem::Windows, Some("wayland"), true, true),
            DesktopSession::Native
        );
        assert_eq!(
            detect_session_from(OperatingSystem::Macos, Some("x11"), false, true),
            DesktopSession::Native
        );
    }

    #[test]
    fn install_type_never_labels_native_platforms_as_deb() {
        assert_eq!(
            install_type_from(OperatingSystem::Windows, false, false),
            InstallType::Windows
        );
        assert_eq!(
            install_type_from(OperatingSystem::Macos, false, false),
            InstallType::Macos
        );
        assert_eq!(
            install_type_from(OperatingSystem::Linux, true, false),
            InstallType::Appimage
        );
        assert_eq!(
            install_type_from(OperatingSystem::Linux, false, false),
            InstallType::Deb
        );
        assert_eq!(
            install_type_from(OperatingSystem::Windows, false, true),
            InstallType::Development
        );
    }

    #[test]
    fn missing_tesseract_degrades_supported_platforms() {
        let windows = capabilities_for(OperatingSystem::Windows, DesktopSession::Native, None);
        let windows = with_ocr_availability(windows, false);
        assert_eq!(windows.ocr.state, CapabilityState::Degraded);
        assert_eq!(windows.ocr.reason, Some(CapabilityReason::OcrNotInstalled));

        let other = capabilities_for(OperatingSystem::Other, DesktopSession::Unknown, None);
        let other = with_ocr_availability(other, false);
        assert_eq!(other.ocr.state, CapabilityState::Unsupported);
        assert_eq!(
            other.ocr.reason,
            Some(CapabilityReason::UnsupportedPlatform)
        );
    }

    #[test]
    fn wayland_reports_protocol_and_permission_boundaries() {
        let capabilities = capabilities_for(
            OperatingSystem::Linux,
            DesktopSession::Wayland,
            Some("GNOME"),
        );
        assert_eq!(
            capabilities.auto_paste.state,
            CapabilityState::PermissionRequired
        );
        assert_eq!(
            capabilities.absolute_window_position,
            Capability::with_reason(
                CapabilityState::Unsupported,
                CapabilityReason::WaylandProtocolLimited
            )
        );
        assert_eq!(
            capabilities.global_shortcuts.state,
            CapabilityState::Available
        );
    }

    #[test]
    fn non_gnome_wayland_shortcuts_are_not_reported_as_fully_available() {
        let capabilities =
            capabilities_for(OperatingSystem::Linux, DesktopSession::Wayland, Some("KDE"));
        assert_eq!(
            capabilities.global_shortcuts,
            Capability::with_reason(
                CapabilityState::Degraded,
                CapabilityReason::NonGnomeWaylandShortcut
            )
        );
    }

    #[test]
    fn ubuntu_compound_desktop_is_normalized_to_gnome() {
        assert_eq!(
            normalize_desktop(Some("ubuntu:GNOME"), None),
            Some("gnome".to_string())
        );
        assert_eq!(
            normalize_desktop(Some("KDE"), Some("plasmawayland")),
            Some("kde".to_string())
        );
    }
}
