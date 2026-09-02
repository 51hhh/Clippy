mod error;
#[cfg(target_os = "linux")]
mod portal;
#[cfg(target_os = "linux")]
mod token_store;
#[cfg(target_os = "linux")]
mod x11;

pub use error::PasteError;
#[cfg(target_os = "linux")]
use portal::PortalState;
use serde::Serialize;
use std::path::Path;
#[cfg(target_os = "linux")]
use std::{path::PathBuf, sync::Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PasteBackend {
    X11,
    WaylandPortal,
    CopyOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PastePhase {
    Ready,
    PermissionRequired,
    Initializing,
    Denied,
    Unavailable,
}

#[derive(Debug, Clone, Serialize)]
pub struct PasteStatus {
    pub backend: PasteBackend,
    pub phase: PastePhase,
    pub auto_paste_enabled: bool,
    pub can_request_permission: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PasteOutcome {
    pub copied: bool,
    pub pasted: bool,
    pub backend: PasteBackend,
    pub detail: Option<String>,
}

impl PasteOutcome {
    pub fn copied_only(backend: PasteBackend, detail: impl Into<Option<String>>) -> Self {
        Self {
            copied: true,
            pasted: false,
            backend,
            detail: detail.into(),
        }
    }
}

#[cfg(target_os = "linux")]
pub struct PasteManager {
    backend: PasteBackend,
    x11_target: Mutex<Option<u32>>,
    portal: tokio::sync::Mutex<PortalState>,
    token_path: PathBuf,
}

#[cfg(not(target_os = "linux"))]
pub struct PasteManager {
    backend: PasteBackend,
}

#[cfg(target_os = "linux")]
impl PasteManager {
    pub fn new(app_data_dir: &Path) -> Self {
        let backend = detect_backend();
        let phase = match backend {
            PasteBackend::X11 => PastePhase::Ready,
            PasteBackend::WaylandPortal => PastePhase::PermissionRequired,
            PasteBackend::CopyOnly => PastePhase::Unavailable,
        };
        Self {
            backend,
            x11_target: Mutex::new(None),
            portal: tokio::sync::Mutex::new(PortalState::new(phase)),
            token_path: app_data_dir.join("remote-desktop-restore-token"),
        }
    }

    pub fn backend(&self) -> PasteBackend {
        self.backend
    }

    /// 在 Clippy 抢占焦点前记录目标窗口，避免 Ctrl+V 注入到自身。
    pub fn capture_target(&self) {
        if self.backend != PasteBackend::X11 {
            return;
        }
        match x11::active_window() {
            Ok(window) => {
                if let Ok(mut target) = self.x11_target.lock() {
                    *target = Some(window);
                }
                log::debug!("记录 X11 粘贴目标窗口: {window}");
            }
            Err(error) => log::warn!("无法记录 X11 活动窗口: {error}"),
        }
    }

    pub async fn status(&self, auto_paste_enabled: bool) -> PasteStatus {
        match self.backend {
            PasteBackend::X11 => PasteStatus {
                backend: self.backend,
                phase: PastePhase::Ready,
                auto_paste_enabled,
                can_request_permission: false,
                detail: None,
            },
            PasteBackend::CopyOnly => PasteStatus {
                backend: self.backend,
                phase: PastePhase::Unavailable,
                auto_paste_enabled,
                can_request_permission: false,
                detail: Some("No supported input injection backend is available".to_string()),
            },
            PasteBackend::WaylandPortal => {
                let state = self.portal.lock().await;
                PasteStatus {
                    backend: self.backend,
                    phase: state.phase(),
                    auto_paste_enabled,
                    can_request_permission: state.can_request_permission(),
                    detail: state.detail().map(str::to_string),
                }
            }
        }
    }

    pub async fn request_permission(
        &self,
        auto_paste_enabled: bool,
    ) -> Result<PasteStatus, PasteError> {
        if self.backend != PasteBackend::WaylandPortal {
            return Ok(self.status(auto_paste_enabled).await);
        }

        let mut state = self.portal.lock().await;
        // 用户显式点击授权时允许重新尝试；自动粘贴失败不会自行循环弹窗。
        state.allow_explicit_retry();
        portal::ensure_session(&mut state, &self.token_path, true).await?;
        drop(state);
        Ok(self.status(auto_paste_enabled).await)
    }

    pub async fn paste(&self) -> Result<PasteOutcome, PasteError> {
        match self.backend {
            PasteBackend::X11 => {
                let target = self.x11_target.lock().ok().and_then(|target| *target);
                let result = tauri::async_runtime::spawn_blocking(move || {
                    let target = target.ok_or(PasteError::X11TargetMissing)?;
                    x11::paste(target)
                })
                .await
                .map_err(|error| PasteError::X11ThreadPanic(error.to_string()))?;
                result?;
                Ok(PasteOutcome {
                    copied: true,
                    pasted: true,
                    backend: self.backend,
                    detail: None,
                })
            }
            PasteBackend::WaylandPortal => {
                let mut state = self.portal.lock().await;
                portal::paste(&mut state, &self.token_path).await?;
                Ok(PasteOutcome {
                    copied: true,
                    pasted: true,
                    backend: self.backend,
                    detail: None,
                })
            }
            PasteBackend::CopyOnly => Ok(PasteOutcome::copied_only(
                self.backend,
                Some("Automatic paste is unavailable in this session".to_string()),
            )),
        }
    }
}

#[cfg(not(target_os = "linux"))]
impl PasteManager {
    pub fn new(_app_data_dir: &Path) -> Self {
        Self {
            backend: PasteBackend::CopyOnly,
        }
    }

    pub fn backend(&self) -> PasteBackend {
        self.backend
    }

    pub fn capture_target(&self) {}

    pub async fn status(&self, auto_paste_enabled: bool) -> PasteStatus {
        PasteStatus {
            backend: self.backend,
            phase: PastePhase::Unavailable,
            auto_paste_enabled,
            can_request_permission: false,
            detail: Some(
                "Native automatic paste backend has not been initialized on this platform"
                    .to_string(),
            ),
        }
    }

    pub async fn request_permission(
        &self,
        auto_paste_enabled: bool,
    ) -> Result<PasteStatus, PasteError> {
        Ok(self.status(auto_paste_enabled).await)
    }

    pub async fn paste(&self) -> Result<PasteOutcome, PasteError> {
        Ok(PasteOutcome::copied_only(
            self.backend,
            Some("Automatic paste is unavailable on this platform".to_string()),
        ))
    }
}

#[cfg(target_os = "linux")]
fn detect_backend() -> PasteBackend {
    let session_type = std::env::var("XDG_SESSION_TYPE")
        .unwrap_or_default()
        .to_ascii_lowercase();
    detect_backend_from(
        &session_type,
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        std::env::var_os("DISPLAY").is_some(),
    )
}

#[cfg(target_os = "linux")]
fn detect_backend_from(
    session_type: &str,
    has_wayland_display: bool,
    has_x11_display: bool,
) -> PasteBackend {
    match session_type {
        "wayland" => PasteBackend::WaylandPortal,
        "x11" => PasteBackend::X11,
        _ if has_wayland_display => PasteBackend::WaylandPortal,
        _ if has_x11_display => PasteBackend::X11,
        _ => PasteBackend::CopyOnly,
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn explicit_session_type_wins_over_stale_display_variables() {
        assert_eq!(detect_backend_from("x11", true, true), PasteBackend::X11);
        assert_eq!(
            detect_backend_from("wayland", false, true),
            PasteBackend::WaylandPortal
        );
        assert_eq!(
            detect_backend_from("", true, true),
            PasteBackend::WaylandPortal
        );
        assert_eq!(detect_backend_from("", false, true), PasteBackend::X11);
        assert_eq!(
            detect_backend_from("tty", false, false),
            PasteBackend::CopyOnly
        );
    }
}
