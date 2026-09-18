//! 长截图控制窗的 wire DTO 与 registry 状态模型。

use super::finish::{FinishStage, RetryPolicy};
use crate::capture::longshot::{LongshotArtifact, LongshotSessionToken, LongshotSnapshot};
use crate::capture::{CaptureError, CaptureSelection};
use crate::pin::PinOrigin;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LongshotIpcError {
    pub code: String,
    pub message: String,
}

impl LongshotIpcError {
    pub(super) fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub(super) fn busy() -> Self {
        Self::new("longshot_controller_busy", "已有长截图控制窗口正在运行")
    }

    pub(super) fn missing() -> Self {
        Self::new(
            "longshot_controller_missing",
            "长截图控制窗口不存在或已经更新",
        )
    }

    pub(super) fn superseded() -> Self {
        Self::new("longshot_controller_superseded", "长截图控制窗口已经更新")
    }

    pub(super) fn cleanup_failed(message: impl Into<String>) -> Self {
        Self::new("longshot_controller_cleanup_failed", message)
    }

    pub(super) fn copy_failed(message: impl Into<String>) -> Self {
        Self::new("longshot_controller_copy_failed", message)
    }

    pub(super) fn save_failed(message: impl Into<String>) -> Self {
        Self::new("longshot_controller_save_failed", message)
    }

    pub(super) fn save_uncertain(message: impl Into<String>) -> Self {
        Self::new("longshot_controller_save_uncertain", message)
    }

    pub(super) fn pin_failed(message: impl Into<String>) -> Self {
        Self::new("longshot_controller_pin_failed", message)
    }

    pub(super) fn pin_uncertain(message: impl Into<String>) -> Self {
        Self::new("longshot_controller_pin_uncertain", message)
    }

    pub(super) fn internal(message: impl Into<String>) -> Self {
        Self::new("longshot_controller_internal", message)
    }
}

impl From<CaptureError> for LongshotIpcError {
    fn from(error: CaptureError) -> Self {
        Self::new(error.code(), error.to_string())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LongshotControllerLaunch {
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LongshotControllerHandle {
    pub session_id: String,
    pub generation: String,
}

impl LongshotControllerHandle {
    pub(super) fn from_token(token: &LongshotSessionToken) -> Self {
        let (session_id, generation) = token.wire_parts();
        Self {
            session_id: session_id.to_string(),
            generation: generation.to_string(),
        }
    }

    pub(super) fn to_token(&self) -> Result<LongshotSessionToken, LongshotIpcError> {
        if self.session_id.is_empty() {
            return Err(LongshotIpcError::superseded());
        }
        let bytes = self.generation.as_bytes();
        if bytes.is_empty()
            || !bytes.iter().all(u8::is_ascii_digit)
            || (bytes.len() > 1 && bytes[0] == b'0')
        {
            return Err(LongshotIpcError::superseded());
        }
        let generation = self
            .generation
            .parse::<u64>()
            .map_err(|_| LongshotIpcError::superseded())?;
        Ok(LongshotSessionToken::from_wire_parts(
            self.session_id.clone(),
            generation,
        ))
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LongshotSnapshotDto {
    pub(super) frame_count: usize,
    /// 当前 union 画布总宽度。
    pub(super) width: u32,
    /// 固定捕获 viewport 的高度。
    pub(super) frame_height: u32,
    /// 当前 union 画布总高度。
    pub(super) total_height: u32,
}

impl From<LongshotSnapshot> for LongshotSnapshotDto {
    fn from(snapshot: LongshotSnapshot) -> Self {
        Self {
            frame_count: snapshot.frame_count,
            width: snapshot.width,
            frame_height: snapshot.frame_height,
            total_height: snapshot.total_height,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LongshotActivation {
    pub handle: LongshotControllerHandle,
    pub snapshot: LongshotSnapshotDto,
    pub auto_scroll: LongshotAutoCapability,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LongshotAutoCapabilityState {
    Available,
    Unsupported,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LongshotAutoCapabilityReason {
    WaylandRemoteDesktopRequired,
    NoDisplayServer,
    PlatformNotImplemented,
}

/// 控制窗只在真实后端可用时展示自动入口；状态不能由前端根据 UA 猜测。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LongshotAutoCapability {
    pub state: LongshotAutoCapabilityState,
    pub reason: Option<LongshotAutoCapabilityReason>,
    pub directions: Vec<crate::capture::longshot::LongshotAutoDirection>,
}

impl LongshotAutoCapability {
    pub(super) fn current() -> Self {
        Self::for_platform(
            crate::platform::current_operating_system(),
            crate::platform::current_session(),
        )
    }

    fn for_platform(
        operating_system: crate::platform::OperatingSystem,
        session: crate::platform::DesktopSession,
    ) -> Self {
        use crate::platform::{DesktopSession, OperatingSystem};

        if operating_system == OperatingSystem::Linux && session == DesktopSession::X11 {
            return Self {
                state: LongshotAutoCapabilityState::Available,
                reason: None,
                directions: vec![
                    crate::capture::longshot::LongshotAutoDirection::Down,
                    crate::capture::longshot::LongshotAutoDirection::Up,
                    crate::capture::longshot::LongshotAutoDirection::Right,
                    crate::capture::longshot::LongshotAutoDirection::Left,
                ],
            };
        }
        let reason = match (operating_system, session) {
            (OperatingSystem::Linux, DesktopSession::Wayland) => {
                LongshotAutoCapabilityReason::WaylandRemoteDesktopRequired
            }
            (OperatingSystem::Linux, DesktopSession::Unknown | DesktopSession::Native) => {
                LongshotAutoCapabilityReason::NoDisplayServer
            }
            _ => LongshotAutoCapabilityReason::PlatformNotImplemented,
        };
        Self {
            state: LongshotAutoCapabilityState::Unsupported,
            reason: Some(reason),
            directions: Vec::new(),
        }
    }
}

#[cfg(test)]
mod auto_capability_tests {
    use super::*;
    use crate::platform::{DesktopSession, OperatingSystem};

    #[test]
    fn only_x11_advertises_the_implemented_auto_scroll_backend() {
        let x11 = LongshotAutoCapability::for_platform(OperatingSystem::Linux, DesktopSession::X11);
        assert_eq!(x11.state, LongshotAutoCapabilityState::Available);
        assert_eq!(x11.directions.len(), 4);

        let wayland =
            LongshotAutoCapability::for_platform(OperatingSystem::Linux, DesktopSession::Wayland);
        assert_eq!(wayland.state, LongshotAutoCapabilityState::Unsupported);
        assert_eq!(
            wayland.reason,
            Some(LongshotAutoCapabilityReason::WaylandRemoteDesktopRequired)
        );
        assert!(wayland.directions.is_empty());

        for operating_system in [OperatingSystem::Windows, OperatingSystem::Macos] {
            let native =
                LongshotAutoCapability::for_platform(operating_system, DesktopSession::Native);
            assert_eq!(native.state, LongshotAutoCapabilityState::Unsupported);
            assert_eq!(
                native.reason,
                Some(LongshotAutoCapabilityReason::PlatformNotImplemented)
            );
            assert!(native.directions.is_empty());
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LongshotOutputAction {
    Copy,
    Save,
    Pin,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LongshotOutputResult {
    pub action: LongshotOutputAction,
    pub path: Option<String>,
    pub pin_label: Option<String>,
}

/// 一次长截图编码产生的不可拆分输出载荷。
///
/// 外层 `Arc` 是 registry 的 exact 身份；内层 PNG `Arc` 可由具体输出实现零复制共享。
#[derive(Debug)]
pub(super) struct LongshotOutputArtifact {
    pub(super) png: Arc<Vec<u8>>,
    pub(super) origin: PinOrigin,
}

impl From<LongshotArtifact> for LongshotOutputArtifact {
    fn from(artifact: LongshotArtifact) -> Self {
        Self {
            png: Arc::new(artifact.png),
            origin: artifact.origin,
        }
    }
}

#[derive(Debug, Default)]
pub(super) enum Slot {
    #[default]
    Empty,
    Building(Launch),
    Pending(Launch),
    Activating {
        label: String,
        cancel_requested: bool,
    },
    Failed {
        label: String,
        revealed: bool,
    },
    Active {
        label: String,
        token: LongshotSessionToken,
        snapshot: LongshotSnapshot,
        revealed: bool,
    },
    Appending {
        label: String,
        token: LongshotSessionToken,
        snapshot: LongshotSnapshot,
    },
    Finishing {
        label: String,
        token: LongshotSessionToken,
        snapshot: LongshotSnapshot,
        stage: FinishStage,
        window_destroyed: bool,
    },
    OutputPending {
        label: String,
        token: LongshotSessionToken,
        snapshot: LongshotSnapshot,
        artifact: Arc<LongshotOutputArtifact>,
        retry_policy: RetryPolicy,
    },
    Terminating {
        label: String,
        token: LongshotSessionToken,
        snapshot: Option<LongshotSnapshot>,
        window_destroyed: bool,
        origin: TerminationOrigin,
    },
    CleanupFailed {
        label: String,
        _token: Option<LongshotSessionToken>,
        revealed: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RetryVisibility {
    NotClaimed,
    InProgress,
    Forbidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TerminationOrigin {
    RevealedActive,
    HiddenAppending(RetryVisibility),
}

#[derive(Debug)]
pub(super) struct Launch {
    pub(super) label: String,
    pub(super) selection: CaptureSelection,
    pub(super) caller_label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HandoffResult {
    pub(super) controller_label: String,
    pub(super) session_id: String,
    pub(super) accepted: bool,
}

#[derive(Default)]
pub(crate) struct LongshotControllerRegistry {
    pub(super) slot: Mutex<Slot>,
    pub(super) origins: Mutex<std::collections::HashMap<String, (String, String)>>,
}

#[derive(Debug)]
pub(super) enum ReadyAction {
    None,
    ShowFailed,
    ShowCleanup,
    ShowActive(LongshotSessionToken),
}

#[derive(Debug)]
pub(super) enum CancelAction {
    Close,
    Requested,
    Terminate(LongshotSessionToken),
}

#[derive(Debug)]
pub(super) struct AppendClaim {
    pub(super) token: LongshotSessionToken,
    pub(super) old_snapshot: LongshotSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CancelFailureRecovery {
    Revealed,
    Hidden,
    CleanupFailed,
}

#[derive(Debug)]
pub(super) enum DeadlineAction {
    None,
    Close,
    Terminate(LongshotSessionToken),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EmergencyDecision {
    Destroy,
    AwaitReady,
    Reveal,
}
