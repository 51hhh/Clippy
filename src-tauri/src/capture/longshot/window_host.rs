//! 独立长截图控制窗的两阶段接管与补偿状态机。

mod finish;
mod output;

use super::{LongshotArtifact, LongshotSessionToken, LongshotSnapshot};
use crate::capture::{CaptureError, CaptureSelection};
use crate::commands::AppState;
use crate::pin::PinOrigin;
use finish::{
    execute_finish_with_ops, run_finish_worker, run_output_worker, FinishOperations, FinishStage,
    OutputValue, OutputWorkerError, RetryPolicy,
};
#[cfg(test)]
use finish::{FinishBoundary, FinishClaim, FinishWorkerError, OutputFailureAction};
use output::{copy_longshot_artifact, pin_longshot_artifact};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager};

const CONTROLLER_PREFIX: &str = "longshot-controller-";
const CONTROLLER_PAGE: &str = "/longshot-controller.html";
const LOAD_DEADLINE_MS: u64 = 5_000;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LongshotIpcError {
    pub code: String,
    pub message: String,
}

impl LongshotIpcError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    fn busy() -> Self {
        Self::new("longshot_controller_busy", "已有长截图控制窗口正在运行")
    }

    fn missing() -> Self {
        Self::new(
            "longshot_controller_missing",
            "长截图控制窗口不存在或已经更新",
        )
    }

    fn superseded() -> Self {
        Self::new("longshot_controller_superseded", "长截图控制窗口已经更新")
    }

    fn cleanup_failed(message: impl Into<String>) -> Self {
        Self::new("longshot_controller_cleanup_failed", message)
    }

    fn copy_failed(message: impl Into<String>) -> Self {
        Self::new("longshot_controller_copy_failed", message)
    }

    fn save_failed(message: impl Into<String>) -> Self {
        Self::new("longshot_controller_save_failed", message)
    }

    fn save_uncertain(message: impl Into<String>) -> Self {
        Self::new("longshot_controller_save_uncertain", message)
    }

    fn pin_failed(message: impl Into<String>) -> Self {
        Self::new("longshot_controller_pin_failed", message)
    }

    fn pin_uncertain(message: impl Into<String>) -> Self {
        Self::new("longshot_controller_pin_uncertain", message)
    }

    fn internal(message: impl Into<String>) -> Self {
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
    fn from_token(token: &LongshotSessionToken) -> Self {
        let (session_id, generation) = token.wire_parts();
        Self {
            session_id: session_id.to_string(),
            generation: generation.to_string(),
        }
    }

    fn to_token(&self) -> Result<LongshotSessionToken, LongshotIpcError> {
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
    frame_count: usize,
    width: u32,
    frame_height: u32,
    total_height: u32,
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
enum Slot {
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
enum RetryVisibility {
    NotClaimed,
    InProgress,
    Forbidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminationOrigin {
    RevealedActive,
    HiddenAppending(RetryVisibility),
}

#[derive(Debug)]
struct Launch {
    label: String,
    selection: CaptureSelection,
    caller_label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HandoffResult {
    controller_label: String,
    session_id: String,
    accepted: bool,
}

#[derive(Default)]
pub(crate) struct LongshotControllerRegistry {
    slot: Mutex<Slot>,
    origins: Mutex<std::collections::HashMap<String, (String, String)>>,
}

#[derive(Debug)]
enum ReadyAction {
    None,
    ShowFailed,
    ShowCleanup,
    ShowActive(LongshotSessionToken),
}

#[derive(Debug)]
enum CancelAction {
    Close,
    Requested,
    Terminate(LongshotSessionToken),
}

#[derive(Debug)]
struct AppendClaim {
    token: LongshotSessionToken,
    old_snapshot: LongshotSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CancelFailureRecovery {
    Revealed,
    Hidden,
    CleanupFailed,
}

#[derive(Debug)]
enum DeadlineAction {
    None,
    Close,
    Terminate(LongshotSessionToken),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmergencyDecision {
    Destroy,
    AwaitReady,
    Reveal,
}

trait ControlWindowActions {
    fn destroy(&self, label: &str);
    fn hide(&self, label: &str) -> Result<(), LongshotIpcError>;
    fn show(&self, label: &str) -> Result<(), LongshotIpcError>;
    fn focus(&self, label: &str);
    fn exists(&self, label: &str) -> bool;
}

struct TauriControlWindowActions<'a> {
    app: &'a tauri::AppHandle,
}

impl ControlWindowActions for TauriControlWindowActions<'_> {
    fn destroy(&self, label: &str) {
        if let Some(window) = self.app.get_webview_window(label) {
            if let Err(error) = window.destroy() {
                log::warn!("销毁长截图控制窗 {label} 失败: {error}");
            }
        }
    }

    fn hide(&self, label: &str) -> Result<(), LongshotIpcError> {
        let window = self
            .app
            .get_webview_window(label)
            .ok_or_else(LongshotIpcError::missing)?;
        window.hide().map_err(|error| {
            LongshotIpcError::new("longshot_controller_hide_failed", error.to_string())
        })
    }

    fn show(&self, label: &str) -> Result<(), LongshotIpcError> {
        let window = self
            .app
            .get_webview_window(label)
            .ok_or_else(LongshotIpcError::missing)?;
        window.show().map_err(|error| {
            LongshotIpcError::new("longshot_controller_show_failed", error.to_string())
        })
    }

    fn focus(&self, label: &str) {
        if let Some(window) = self.app.get_webview_window(label) {
            let _ = window.set_focus();
        }
    }

    fn exists(&self, label: &str) -> bool {
        self.app.get_webview_window(label).is_some()
    }
}

fn emergency_decision(cleanup_succeeded: bool, registry_settled: bool) -> EmergencyDecision {
    if cleanup_succeeded && registry_settled {
        EmergencyDecision::Destroy
    } else if registry_settled {
        EmergencyDecision::AwaitReady
    } else {
        EmergencyDecision::Reveal
    }
}

impl LongshotControllerRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn reserve(
        &self,
        caller_label: String,
        selection: CaptureSelection,
    ) -> Result<String, LongshotIpcError> {
        let mut slot = self
            .slot
            .lock()
            .map_err(|error| LongshotIpcError::internal(error.to_string()))?;
        if !matches!(*slot, Slot::Empty) {
            return Err(LongshotIpcError::busy());
        }
        let label = format!("{CONTROLLER_PREFIX}{}", crate::image_io::unique_image_id());
        self.origins
            .lock()
            .map_err(|error| LongshotIpcError::internal(error.to_string()))?
            .insert(
                label.clone(),
                (caller_label.clone(), selection.session_id.clone()),
            );
        *slot = Slot::Building(Launch {
            label: label.clone(),
            selection,
            caller_label,
        });
        Ok(label)
    }

    fn take_handoff(
        &self,
        label: &str,
        accepted: bool,
        ordinary_current: impl FnOnce(&str) -> bool,
    ) -> Option<(String, HandoffResult)> {
        let (caller, session_id) = self.origins.lock().ok()?.remove(label)?;
        if !accepted && !ordinary_current(&session_id) {
            return None;
        }
        Some((
            caller,
            HandoffResult {
                controller_label: label.to_string(),
                session_id,
                accepted,
            },
        ))
    }

    fn publish_started(&self, label: &str, path: &str) -> bool {
        if path != CONTROLLER_PAGE {
            return false;
        }
        let Ok(mut slot) = self.slot.lock() else {
            return false;
        };
        let previous = std::mem::take(&mut *slot);
        match previous {
            Slot::Building(launch) if launch.label == label => {
                *slot = Slot::Pending(launch);
                true
            }
            other => {
                *slot = other;
                false
            }
        }
    }

    fn abort_build(&self, label: &str) {
        if let Ok(mut origins) = self.origins.lock() {
            origins.remove(label);
        }
        let Ok(mut slot) = self.slot.lock() else {
            return;
        };
        let previous = std::mem::take(&mut *slot);
        *slot = match previous {
            Slot::Building(launch) | Slot::Pending(launch) if launch.label == label => Slot::Empty,
            Slot::Activating {
                label: active_label,
                ..
            } if active_label == label => Slot::Activating {
                label: active_label,
                cancel_requested: true,
            },
            other => other,
        };
    }

    fn accepts_built_window(&self, label: &str) -> bool {
        self.slot.lock().is_ok_and(|slot| match &*slot {
            Slot::Building(launch) | Slot::Pending(launch) => launch.label == label,
            Slot::Activating { label: current, .. }
            | Slot::Failed { label: current, .. }
            | Slot::Active { label: current, .. }
            | Slot::Appending { label: current, .. }
            | Slot::Finishing { label: current, .. }
            | Slot::OutputPending { label: current, .. }
            | Slot::Terminating { label: current, .. }
            | Slot::CleanupFailed { label: current, .. } => current == label,
            Slot::Empty => false,
        })
    }

    fn claim_activation(&self, label: &str) -> Result<CaptureSelection, LongshotIpcError> {
        let mut slot = self
            .slot
            .lock()
            .map_err(|error| LongshotIpcError::internal(error.to_string()))?;
        let previous = std::mem::take(&mut *slot);
        match previous {
            Slot::Pending(launch) if launch.label == label => {
                debug_assert!(launch.caller_label.starts_with("capture-overlay-"));
                let selection = launch.selection;
                *slot = Slot::Activating {
                    label: label.to_string(),
                    cancel_requested: false,
                };
                Ok(selection)
            }
            other => {
                *slot = other;
                Err(LongshotIpcError::missing())
            }
        }
    }

    fn complete_activation(
        &self,
        label: &str,
        result: Result<super::LongshotStart, CaptureError>,
    ) -> Result<(Option<LongshotActivation>, Option<LongshotSessionToken>), LongshotIpcError> {
        let mut slot = self
            .slot
            .lock()
            .map_err(|error| LongshotIpcError::internal(error.to_string()))?;
        let previous = std::mem::take(&mut *slot);
        let Slot::Activating {
            label: current,
            cancel_requested,
        } = previous
        else {
            *slot = previous;
            return Err(LongshotIpcError::superseded());
        };
        if current != label {
            *slot = Slot::Activating {
                label: current,
                cancel_requested,
            };
            return Err(LongshotIpcError::superseded());
        }
        match result {
            Ok(start) if cancel_requested => {
                let token = start.token;
                *slot = Slot::Terminating {
                    label: label.to_string(),
                    token: token.clone(),
                    snapshot: Some(start.snapshot),
                    window_destroyed: true,
                    origin: TerminationOrigin::RevealedActive,
                };
                Ok((None, Some(token)))
            }
            Ok(start) => {
                let activation = LongshotActivation {
                    handle: LongshotControllerHandle::from_token(&start.token),
                    snapshot: start.snapshot.into(),
                };
                *slot = Slot::Active {
                    label: label.to_string(),
                    token: start.token,
                    snapshot: start.snapshot,
                    revealed: false,
                };
                Ok((Some(activation), None))
            }
            Err(error) => {
                if cancel_requested {
                    *slot = Slot::Empty;
                } else {
                    *slot = Slot::Failed {
                        label: label.to_string(),
                        revealed: false,
                    };
                }
                Err(error.into())
            }
        }
    }

    fn claim_ready(&self, label: &str) -> Result<ReadyAction, LongshotIpcError> {
        let mut slot = self
            .slot
            .lock()
            .map_err(|error| LongshotIpcError::internal(error.to_string()))?;
        match &mut *slot {
            Slot::Active {
                label: current,
                token,
                revealed,
                ..
            } if current == label => {
                if *revealed {
                    Ok(ReadyAction::None)
                } else {
                    *revealed = true;
                    Ok(ReadyAction::ShowActive(token.clone()))
                }
            }
            Slot::Failed {
                label: current,
                revealed,
            } if current == label => {
                if *revealed {
                    Ok(ReadyAction::None)
                } else {
                    *revealed = true;
                    Ok(ReadyAction::ShowFailed)
                }
            }
            Slot::CleanupFailed {
                label: current,
                revealed,
                ..
            } if current == label => {
                if *revealed {
                    Ok(ReadyAction::None)
                } else {
                    *revealed = true;
                    Ok(ReadyAction::ShowCleanup)
                }
            }
            _ => Err(LongshotIpcError::missing()),
        }
    }

    fn claim_append(
        &self,
        label: &str,
        handle: &LongshotControllerHandle,
    ) -> Result<AppendClaim, LongshotIpcError> {
        if !label.starts_with(CONTROLLER_PREFIX) {
            return Err(LongshotIpcError::missing());
        }
        let wire_token = handle.to_token()?;
        let mut slot = self
            .slot
            .lock()
            .map_err(|error| LongshotIpcError::internal(error.to_string()))?;
        let previous = std::mem::take(&mut *slot);
        match previous {
            Slot::Active {
                label: current,
                token,
                snapshot,
                revealed: true,
            } if current == label && token == wire_token => {
                *slot = Slot::Appending {
                    label: current,
                    token: token.clone(),
                    snapshot,
                };
                Ok(AppendClaim {
                    token,
                    old_snapshot: snapshot,
                })
            }
            Slot::Active {
                label: current,
                token,
                snapshot,
                revealed,
            } if current == label => {
                *slot = Slot::Active {
                    label: current,
                    token,
                    snapshot,
                    revealed,
                };
                if revealed {
                    Err(LongshotIpcError::superseded())
                } else {
                    Err(LongshotIpcError::missing())
                }
            }
            Slot::Appending {
                label: current,
                token,
                snapshot,
            } if current == label => {
                let error = if wire_token == token {
                    LongshotIpcError::busy()
                } else {
                    LongshotIpcError::superseded()
                };
                *slot = Slot::Appending {
                    label: current,
                    token,
                    snapshot,
                };
                Err(error)
            }
            Slot::Finishing {
                label: current,
                token,
                snapshot,
                stage,
                window_destroyed,
            } if current == label => {
                let error = if wire_token == token {
                    LongshotIpcError::busy()
                } else {
                    LongshotIpcError::superseded()
                };
                *slot = Slot::Finishing {
                    label: current,
                    token,
                    snapshot,
                    stage,
                    window_destroyed,
                };
                Err(error)
            }
            Slot::OutputPending {
                label: current,
                token,
                snapshot,
                artifact,
                retry_policy,
            } if current == label => {
                let error = if wire_token == token {
                    LongshotIpcError::busy()
                } else {
                    LongshotIpcError::superseded()
                };
                *slot = Slot::OutputPending {
                    label: current,
                    token,
                    snapshot,
                    artifact,
                    retry_policy,
                };
                Err(error)
            }
            other => {
                *slot = other;
                Err(LongshotIpcError::missing())
            }
        }
    }

    /// 只读授权 revealed Active 控制窗生成预览；不得改变 registry 阶段。
    fn authorize_preview(
        &self,
        label: &str,
        handle: &LongshotControllerHandle,
    ) -> Result<LongshotSessionToken, LongshotIpcError> {
        if !label.starts_with(CONTROLLER_PREFIX) {
            return Err(LongshotIpcError::missing());
        }
        let wire_token = handle.to_token()?;
        let slot = self
            .slot
            .lock()
            .map_err(|error| LongshotIpcError::internal(error.to_string()))?;
        match &*slot {
            Slot::Active {
                label: current,
                token,
                revealed: true,
                ..
            } if current == label && *token == wire_token => Ok(token.clone()),
            Slot::Active {
                label: current,
                revealed,
                ..
            } if current == label => {
                if *revealed {
                    Err(LongshotIpcError::superseded())
                } else {
                    Err(LongshotIpcError::missing())
                }
            }
            Slot::Appending {
                label: current,
                token,
                ..
            }
            | Slot::Finishing {
                label: current,
                token,
                ..
            }
            | Slot::OutputPending {
                label: current,
                token,
                ..
            } if current == label => {
                if *token == wire_token {
                    Err(LongshotIpcError::busy())
                } else {
                    Err(LongshotIpcError::superseded())
                }
            }
            _ => Err(LongshotIpcError::missing()),
        }
    }

    /// worker 返回后的 exact Active 核验；任何阶段变化都使旧预览失效。
    fn confirms_preview(
        &self,
        label: &str,
        token: &LongshotSessionToken,
    ) -> Result<bool, LongshotIpcError> {
        let slot = self
            .slot
            .lock()
            .map_err(|error| LongshotIpcError::internal(error.to_string()))?;
        Ok(matches!(&*slot, Slot::Active {
            label: current,
            token: current_token,
            revealed: true,
            ..
        } if current == label && current_token == token))
    }

    fn owns_append(&self, label: &str, token: &LongshotSessionToken) -> bool {
        self.slot.lock().is_ok_and(|slot| {
            matches!(&*slot, Slot::Appending {
                label: current,
                token: current_token,
                ..
            } if current == label && current_token == token)
        })
    }

    fn complete_append_visible(
        &self,
        label: &str,
        token: &LongshotSessionToken,
        snapshot: LongshotSnapshot,
    ) -> Result<LongshotSnapshotDto, LongshotIpcError> {
        let mut slot = self
            .slot
            .lock()
            .map_err(|error| LongshotIpcError::internal(error.to_string()))?;
        let previous = std::mem::take(&mut *slot);
        match previous {
            Slot::Appending {
                label: current,
                token: current_token,
                ..
            } if current == label && current_token == *token => {
                *slot = Slot::Active {
                    label: current,
                    token: current_token,
                    snapshot,
                    revealed: true,
                };
                Ok(snapshot.into())
            }
            other => {
                *slot = other;
                Err(LongshotIpcError::superseded())
            }
        }
    }

    fn claim_append_visibility_failure(
        &self,
        label: &str,
        token: &LongshotSessionToken,
    ) -> Option<LongshotSessionToken> {
        let Ok(mut slot) = self.slot.lock() else {
            return None;
        };
        let previous = std::mem::take(&mut *slot);
        match previous {
            Slot::Appending {
                label: current,
                token: current_token,
                snapshot,
            } if current == label && current_token == *token => {
                *slot = Slot::Terminating {
                    label: current,
                    token: current_token.clone(),
                    snapshot: Some(snapshot),
                    window_destroyed: true,
                    origin: TerminationOrigin::HiddenAppending(RetryVisibility::Forbidden),
                };
                Some(current_token)
            }
            other => {
                *slot = other;
                None
            }
        }
    }

    fn claim_cancel(
        &self,
        label: &str,
        handle: Option<&LongshotControllerHandle>,
    ) -> Result<CancelAction, LongshotIpcError> {
        let token_from_wire = handle.map(LongshotControllerHandle::to_token).transpose()?;
        let mut slot = self
            .slot
            .lock()
            .map_err(|error| LongshotIpcError::internal(error.to_string()))?;
        let previous = std::mem::take(&mut *slot);
        match previous {
            Slot::Building(launch) if launch.label == label => {
                if token_from_wire.is_some() {
                    *slot = Slot::Building(launch);
                    return Err(LongshotIpcError::superseded());
                }
                *slot = Slot::Empty;
                Ok(CancelAction::Close)
            }
            Slot::Pending(launch) if launch.label == label => {
                if token_from_wire.is_some() {
                    *slot = Slot::Pending(launch);
                    return Err(LongshotIpcError::superseded());
                }
                *slot = Slot::Empty;
                Ok(CancelAction::Close)
            }
            Slot::Failed {
                label: current,
                revealed,
            } if current == label => {
                if token_from_wire.is_some() {
                    *slot = Slot::Failed {
                        label: current,
                        revealed,
                    };
                    return Err(LongshotIpcError::superseded());
                }
                *slot = Slot::Empty;
                Ok(CancelAction::Close)
            }
            Slot::Activating {
                label: current,
                cancel_requested,
            } if current == label => {
                if token_from_wire.is_some() {
                    *slot = Slot::Activating {
                        label: current,
                        cancel_requested,
                    };
                    return Err(LongshotIpcError::superseded());
                }
                *slot = Slot::Activating {
                    label: current,
                    cancel_requested: true,
                };
                Ok(CancelAction::Requested)
            }
            Slot::Active {
                label: current,
                token,
                snapshot,
                revealed,
            } if current == label => {
                if token_from_wire.as_ref() != Some(&token) {
                    *slot = Slot::Active {
                        label: current,
                        token,
                        snapshot,
                        revealed,
                    };
                    return Err(LongshotIpcError::superseded());
                }
                *slot = Slot::Terminating {
                    label: label.to_string(),
                    token: token.clone(),
                    snapshot: Some(snapshot),
                    window_destroyed: false,
                    origin: TerminationOrigin::RevealedActive,
                };
                Ok(CancelAction::Terminate(token))
            }
            Slot::Appending {
                label: current,
                token,
                snapshot,
            } if current == label => {
                if token_from_wire.as_ref() != Some(&token) {
                    *slot = Slot::Appending {
                        label: current,
                        token,
                        snapshot,
                    };
                    return Err(LongshotIpcError::superseded());
                }
                *slot = Slot::Terminating {
                    label: current,
                    token: token.clone(),
                    snapshot: Some(snapshot),
                    window_destroyed: false,
                    origin: TerminationOrigin::HiddenAppending(RetryVisibility::NotClaimed),
                };
                Ok(CancelAction::Terminate(token))
            }
            Slot::Finishing {
                label: current,
                token,
                snapshot,
                stage,
                window_destroyed,
            } if current == label => {
                if token_from_wire.as_ref() != Some(&token) {
                    *slot = Slot::Finishing {
                        label: current,
                        token,
                        snapshot,
                        stage,
                        window_destroyed,
                    };
                    return Err(LongshotIpcError::superseded());
                }
                *slot = Slot::Finishing {
                    label: current,
                    token,
                    snapshot,
                    stage,
                    window_destroyed: true,
                };
                Ok(CancelAction::Close)
            }
            Slot::OutputPending {
                label: current,
                token,
                snapshot,
                artifact,
                retry_policy,
            } if current == label => {
                if token_from_wire.as_ref() != Some(&token) {
                    *slot = Slot::OutputPending {
                        label: current,
                        token,
                        snapshot,
                        artifact,
                        retry_policy,
                    };
                    return Err(LongshotIpcError::superseded());
                }
                *slot = Slot::Empty;
                Ok(CancelAction::Close)
            }
            other => {
                *slot = other;
                Err(LongshotIpcError::missing())
            }
        }
    }

    fn claim_deadline(&self, label: &str) -> DeadlineAction {
        let Ok(mut slot) = self.slot.lock() else {
            log::error!("长截图控制窗 deadline 无法取得 registry 锁");
            return DeadlineAction::None;
        };
        let previous = std::mem::take(&mut *slot);
        match previous {
            Slot::Building(launch) | Slot::Pending(launch) if launch.label == label => {
                *slot = Slot::Empty;
                DeadlineAction::Close
            }
            Slot::Failed {
                label: current,
                revealed: false,
            } if current == label => {
                *slot = Slot::Empty;
                DeadlineAction::Close
            }
            Slot::Activating { label: current, .. } if current == label => {
                *slot = Slot::Activating {
                    label: current,
                    cancel_requested: true,
                };
                DeadlineAction::Close
            }
            Slot::Active {
                label: current,
                token,
                snapshot,
                revealed: false,
            } if current == label => {
                *slot = Slot::Terminating {
                    label: current,
                    token: token.clone(),
                    snapshot: Some(snapshot),
                    // deadline 随后必定关窗，失败不能开放一个已经没有 UI 的重试态。
                    window_destroyed: true,
                    origin: TerminationOrigin::RevealedActive,
                };
                DeadlineAction::Terminate(token)
            }
            other => {
                *slot = other;
                DeadlineAction::None
            }
        }
    }

    fn claim_destroyed(&self, label: &str) -> Option<LongshotSessionToken> {
        let Ok(mut slot) = self.slot.lock() else {
            log::error!("控制窗销毁时无法取得 registry 锁");
            return None;
        };
        let previous = std::mem::take(&mut *slot);
        match previous {
            Slot::Building(launch) | Slot::Pending(launch) if launch.label == label => {
                *slot = Slot::Empty;
                None
            }
            Slot::Failed { label: current, .. } if current == label => {
                *slot = Slot::Empty;
                None
            }
            Slot::Activating { label: current, .. } if current == label => {
                *slot = Slot::Activating {
                    label: current,
                    cancel_requested: true,
                };
                None
            }
            Slot::Active {
                label: current,
                token,
                snapshot,
                ..
            } if current == label => {
                *slot = Slot::Terminating {
                    label: current,
                    token: token.clone(),
                    snapshot: Some(snapshot),
                    window_destroyed: true,
                    origin: TerminationOrigin::RevealedActive,
                };
                Some(token)
            }
            Slot::Appending {
                label: current,
                token,
                snapshot,
            } if current == label => {
                *slot = Slot::Terminating {
                    label: current,
                    token: token.clone(),
                    snapshot: Some(snapshot),
                    window_destroyed: true,
                    origin: TerminationOrigin::HiddenAppending(RetryVisibility::Forbidden),
                };
                Some(token)
            }
            Slot::Finishing {
                label: current,
                token,
                snapshot,
                stage,
                ..
            } if current == label => {
                *slot = Slot::Finishing {
                    label: current,
                    token,
                    snapshot,
                    stage,
                    window_destroyed: true,
                };
                None
            }
            Slot::OutputPending { label: current, .. } if current == label => {
                *slot = Slot::Empty;
                None
            }
            Slot::Terminating {
                label: current,
                token,
                snapshot,
                origin,
                ..
            } if current == label => {
                *slot = Slot::Terminating {
                    label: current,
                    token,
                    snapshot,
                    window_destroyed: true,
                    origin: match origin {
                        TerminationOrigin::HiddenAppending(_) => {
                            TerminationOrigin::HiddenAppending(RetryVisibility::Forbidden)
                        }
                        other => other,
                    },
                };
                None
            }
            other => {
                *slot = other;
                None
            }
        }
    }

    fn complete_cancel_success(&self, label: &str, token: &LongshotSessionToken) {
        let Ok(mut slot) = self.slot.lock() else {
            log::error!("长截图取消成功后无法清空 registry");
            return;
        };
        if matches!(&*slot, Slot::Terminating { label: current, token: current_token, .. }
            if current == label && current_token == token)
        {
            *slot = Slot::Empty;
        }
    }

    fn begin_cancel_failure_recovery(
        &self,
        label: &str,
        token: &LongshotSessionToken,
    ) -> CancelFailureRecovery {
        let Ok(mut slot) = self.slot.lock() else {
            return CancelFailureRecovery::CleanupFailed;
        };
        let previous = std::mem::take(&mut *slot);
        match previous {
            Slot::Terminating {
                label: current,
                token: current_token,
                snapshot,
                window_destroyed: false,
                origin: TerminationOrigin::HiddenAppending(RetryVisibility::NotClaimed),
            } if current == label && current_token == *token => {
                *slot = Slot::Terminating {
                    label: current,
                    token: current_token,
                    snapshot,
                    window_destroyed: false,
                    origin: TerminationOrigin::HiddenAppending(RetryVisibility::InProgress),
                };
                CancelFailureRecovery::Hidden
            }
            Slot::Terminating {
                label: current,
                token: current_token,
                snapshot,
                window_destroyed,
                origin: TerminationOrigin::RevealedActive,
            } if current == label && current_token == *token => {
                *slot = Slot::Terminating {
                    label: current,
                    token: current_token,
                    snapshot,
                    window_destroyed,
                    origin: TerminationOrigin::RevealedActive,
                };
                CancelFailureRecovery::Revealed
            }
            Slot::Terminating {
                label: current,
                token: current_token,
                origin: TerminationOrigin::HiddenAppending(_),
                ..
            } if current == label && current_token == *token => {
                *slot = Slot::CleanupFailed {
                    label: current,
                    _token: Some(current_token),
                    revealed: false,
                };
                CancelFailureRecovery::CleanupFailed
            }
            other => {
                *slot = other;
                CancelFailureRecovery::CleanupFailed
            }
        }
    }

    fn owns_hidden_cancel_reveal(&self, label: &str, token: &LongshotSessionToken) -> bool {
        self.slot.lock().is_ok_and(|slot| {
            matches!(&*slot, Slot::Terminating {
                label: current,
                token: current_token,
                window_destroyed: false,
                origin: TerminationOrigin::HiddenAppending(RetryVisibility::InProgress),
                ..
            } if current == label && current_token == token)
        })
    }

    fn complete_cancel_failure(
        &self,
        label: &str,
        token: &LongshotSessionToken,
        retryable: bool,
    ) -> CancelFailureRecovery {
        let Ok(mut slot) = self.slot.lock() else {
            log::error!("长截图取消失败后无法保留 registry");
            return CancelFailureRecovery::CleanupFailed;
        };
        let previous = std::mem::take(&mut *slot);
        let (next, recovery) = match previous {
            Slot::Terminating {
                label: current,
                token: current_token,
                snapshot,
                window_destroyed,
                origin,
            } if current == label && current_token == *token => {
                match (retryable, window_destroyed, snapshot, origin) {
                    (true, false, Some(snapshot), TerminationOrigin::RevealedActive) => (
                        Slot::Active {
                            label: current,
                            token: current_token,
                            snapshot,
                            revealed: true,
                        },
                        CancelFailureRecovery::Revealed,
                    ),
                    (
                        true,
                        false,
                        snapshot,
                        TerminationOrigin::HiddenAppending(RetryVisibility::NotClaimed),
                    ) => (
                        Slot::Terminating {
                            label: current,
                            token: current_token,
                            snapshot,
                            window_destroyed: false,
                            origin: TerminationOrigin::HiddenAppending(RetryVisibility::InProgress),
                        },
                        CancelFailureRecovery::Hidden,
                    ),
                    (_, _, _, _) => (
                        Slot::CleanupFailed {
                            label: current,
                            _token: Some(current_token),
                            revealed: false,
                        },
                        CancelFailureRecovery::CleanupFailed,
                    ),
                }
            }
            other => (other, CancelFailureRecovery::CleanupFailed),
        };
        *slot = next;
        recovery
    }

    fn complete_hidden_cancel_reveal(&self, label: &str, token: &LongshotSessionToken) -> bool {
        let Ok(mut slot) = self.slot.lock() else {
            return false;
        };
        let previous = std::mem::take(&mut *slot);
        match previous {
            Slot::Terminating {
                label: current,
                token: current_token,
                snapshot: Some(snapshot),
                window_destroyed: false,
                origin: TerminationOrigin::HiddenAppending(RetryVisibility::InProgress),
            } if current == label && current_token == *token => {
                *slot = Slot::Active {
                    label: current,
                    token: current_token,
                    snapshot,
                    revealed: true,
                };
                true
            }
            other => {
                *slot = other;
                false
            }
        }
    }

    fn fail_hidden_cancel_reveal(&self, label: &str, token: &LongshotSessionToken) {
        let Ok(mut slot) = self.slot.lock() else {
            return;
        };
        if matches!(&*slot, Slot::Terminating {
            label: current,
            token: current_token,
            origin: TerminationOrigin::HiddenAppending(_),
            ..
        } if current == label && current_token == token)
        {
            *slot = Slot::CleanupFailed {
                label: label.to_string(),
                _token: Some(token.clone()),
                revealed: false,
            };
        }
    }

    /// show 已失败，控制窗马上会关闭；只有从 Active 成功认领的线程可以执行 cancel。
    fn claim_forced_termination(
        &self,
        label: &str,
        token: &LongshotSessionToken,
    ) -> Option<LongshotSessionToken> {
        let Ok(mut slot) = self.slot.lock() else {
            log::error!("控制窗显示失败后无法取得 registry 锁");
            return None;
        };
        let previous = std::mem::take(&mut *slot);
        match previous {
            Slot::Active {
                label: current,
                token: current_token,
                snapshot,
                ..
            } if current == label && current_token == *token => {
                *slot = Slot::Terminating {
                    label: current,
                    token: current_token.clone(),
                    snapshot: Some(snapshot),
                    window_destroyed: true,
                    origin: TerminationOrigin::RevealedActive,
                };
                Some(current_token)
            }
            other => {
                *slot = other;
                None
            }
        }
    }

    fn mark_cleanup_failed(&self, label: &str, token: Option<LongshotSessionToken>) -> bool {
        let Ok(mut slot) = self.slot.lock() else {
            log::error!("无法记录长截图控制窗清理失败");
            return false;
        };
        let matches_label = match &*slot {
            Slot::Activating { label: current, .. } | Slot::Terminating { label: current, .. } => {
                current == label
            }
            _ => false,
        };
        if matches_label {
            *slot = Slot::CleanupFailed {
                label: label.to_string(),
                _token: token,
                revealed: false,
            };
            true
        } else {
            false
        }
    }

    /// 强制终结 worker 返回失败后的 exact 收敛；允许已由真实 cancel worker 提前写入。
    fn settle_termination_cleanup_failed(&self, label: &str, token: &LongshotSessionToken) -> bool {
        let Ok(mut slot) = self.slot.lock() else {
            return false;
        };
        let previous = std::mem::take(&mut *slot);
        match previous {
            Slot::Terminating {
                label: current,
                token: current_token,
                ..
            } if current == label && current_token == *token => {
                *slot = Slot::CleanupFailed {
                    label: current,
                    _token: Some(current_token),
                    revealed: false,
                };
                true
            }
            Slot::CleanupFailed {
                label: current,
                _token: Some(current_token),
                revealed,
            } if current == label && current_token == *token => {
                *slot = Slot::CleanupFailed {
                    label: current,
                    _token: Some(current_token),
                    revealed,
                };
                true
            }
            other => {
                *slot = other;
                false
            }
        }
    }

    fn rollback_cleanup_reveal(&self, label: &str) {
        let Ok(mut slot) = self.slot.lock() else {
            return;
        };
        if let Slot::CleanupFailed {
            label: current,
            revealed,
            ..
        } = &mut *slot
        {
            if current == label {
                *revealed = false;
            }
        }
    }

    fn mark_cleanup_revealed(&self, label: &str) {
        let Ok(mut slot) = self.slot.lock() else {
            return;
        };
        if let Slot::CleanupFailed {
            label: current,
            revealed,
            ..
        } = &mut *slot
        {
            if current == label {
                *revealed = true;
            }
        }
    }

    /// begin 已成功但 registry 提交失败后的最后收敛点。
    /// 补偿成功才允许清槽；补偿失败必须保留 exact token 阻止第二个会话。
    fn settle_emergency_cleanup(
        &self,
        label: &str,
        token: &LongshotSessionToken,
        cleanup_succeeded: bool,
    ) -> bool {
        let Ok(mut slot) = self.slot.lock() else {
            return false;
        };
        let previous = std::mem::take(&mut *slot);
        let exact = match &previous {
            Slot::Activating { label: current, .. } => current == label,
            Slot::Terminating {
                label: current,
                token: current_token,
                ..
            } => current == label && current_token == token,
            Slot::CleanupFailed {
                label: current,
                _token: Some(current_token),
                ..
            } => current == label && current_token == token,
            _ => false,
        };
        if !exact {
            *slot = previous;
            return false;
        }
        if cleanup_succeeded {
            *slot = Slot::Empty;
        } else {
            *slot = Slot::CleanupFailed {
                label: label.to_string(),
                _token: Some(token.clone()),
                revealed: false,
            };
        }
        true
    }
}

async fn execute_cancel_action<A, C, F>(
    action: CancelAction,
    label: &str,
    windows: &A,
    cancel: C,
) -> Result<(), LongshotIpcError>
where
    A: ControlWindowActions,
    C: FnOnce(LongshotSessionToken) -> F,
    F: std::future::Future<Output = Result<(), LongshotIpcError>>,
{
    match action {
        CancelAction::Close | CancelAction::Requested => {
            windows.destroy(label);
            Ok(())
        }
        CancelAction::Terminate(token) => {
            let result = cancel(token).await;
            if result.is_ok() {
                windows.destroy(label);
            }
            result
        }
    }
}

async fn execute_deadline_action<A, C, F>(
    action: DeadlineAction,
    label: &str,
    windows: &A,
    cancel: C,
) where
    A: ControlWindowActions,
    C: FnOnce(LongshotSessionToken) -> F,
    F: std::future::Future<Output = Result<(), LongshotIpcError>>,
{
    if let Some(token) = execute_deadline_window_action(action, label, windows) {
        let _ = cancel(token).await;
        windows.destroy(label);
    }
}

fn execute_deadline_window_action<A: ControlWindowActions>(
    action: DeadlineAction,
    label: &str,
    windows: &A,
) -> Option<LongshotSessionToken> {
    match action {
        DeadlineAction::None => None,
        DeadlineAction::Close => {
            windows.destroy(label);
            None
        }
        DeadlineAction::Terminate(token) => Some(token),
    }
}

async fn execute_token_cleanup<C, F>(token: Option<LongshotSessionToken>, cancel: C)
where
    C: FnOnce(LongshotSessionToken) -> F,
    F: std::future::Future<Output = Result<(), LongshotIpcError>>,
{
    if let Some(token) = token {
        let _ = cancel(token).await;
    }
}

async fn execute_ready_action<A, C, F>(
    registry: &LongshotControllerRegistry,
    action: ReadyAction,
    label: &str,
    windows: &A,
    cancel: C,
) -> Result<(), LongshotIpcError>
where
    A: ControlWindowActions,
    C: FnOnce(LongshotSessionToken) -> F,
    F: std::future::Future<Output = Result<(), LongshotIpcError>>,
{
    let (token, cleanup) = match action {
        ReadyAction::None => return Ok(()),
        ReadyAction::ShowFailed => (None, false),
        ReadyAction::ShowCleanup => (None, true),
        ReadyAction::ShowActive(token) => (Some(token), false),
    };
    if let Err(error) = windows.show(label) {
        if cleanup {
            registry.rollback_cleanup_reveal(label);
            return Err(LongshotIpcError::cleanup_failed(error.message));
        }
        if let Some(token) = token {
            let claimed = registry.claim_forced_termination(label, &token);
            execute_token_cleanup(claimed, cancel).await;
        } else {
            let _ = registry.claim_cancel(label, None);
        }
        windows.destroy(label);
        return Err(error);
    }
    windows.focus(label);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppendBoundary {
    AfterClaim,
    AfterHide,
    AfterSettle,
    AfterWorker,
    AfterShow,
    BeforeCommit,
}

async fn terminate_hidden_append<A, C, F>(
    registry: &LongshotControllerRegistry,
    label: &str,
    token: &LongshotSessionToken,
    windows: &A,
    cancel: C,
    visibility_error: LongshotIpcError,
) -> LongshotIpcError
where
    A: ControlWindowActions,
    C: FnOnce(LongshotSessionToken) -> F,
    F: std::future::Future<Output = Result<(), LongshotIpcError>>,
{
    let claimed = registry.claim_append_visibility_failure(label, token);
    let cleanup = if let Some(claimed) = claimed {
        cancel(claimed).await
    } else {
        return LongshotIpcError::superseded();
    };
    windows.destroy(label);
    match cleanup {
        Ok(()) => visibility_error,
        Err(error) => {
            let _ = registry.settle_termination_cleanup_failed(label, token);
            LongshotIpcError::cleanup_failed(error.message)
        }
    }
}

// 编排 seam 显式注入窗口、等待、worker、cleanup 与边界钩子，避免测试绕开生产路径。
#[allow(clippy::too_many_arguments)]
async fn execute_append_with_ops<A, S, SF, W, WF, C, CF, H>(
    registry: &LongshotControllerRegistry,
    label: &str,
    handle: &LongshotControllerHandle,
    windows: &A,
    settle: S,
    worker: W,
    cancel: C,
    mut boundary: H,
) -> Result<LongshotSnapshotDto, LongshotIpcError>
where
    A: ControlWindowActions,
    S: FnOnce() -> SF,
    SF: std::future::Future<Output = ()>,
    W: FnOnce(LongshotSessionToken) -> WF,
    WF: std::future::Future<Output = Result<LongshotSnapshot, LongshotIpcError>>,
    C: FnOnce(LongshotSessionToken) -> CF,
    CF: std::future::Future<Output = Result<(), LongshotIpcError>>,
    H: FnMut(AppendBoundary),
{
    let claim = registry.claim_append(label, handle)?;
    boundary(AppendBoundary::AfterClaim);
    if !registry.owns_append(label, &claim.token) {
        return Err(LongshotIpcError::superseded());
    }
    if let Err(hide_error) = windows.hide(label) {
        if !registry.owns_append(label, &claim.token) {
            return Err(LongshotIpcError::superseded());
        }
        if let Err(show_error) = windows.show(label) {
            return Err(terminate_hidden_append(
                registry,
                label,
                &claim.token,
                windows,
                cancel,
                show_error,
            )
            .await);
        }
        if !registry.owns_append(label, &claim.token) {
            windows.destroy(label);
            return Err(LongshotIpcError::superseded());
        }
        windows.focus(label);
        if let Err(error) =
            registry.complete_append_visible(label, &claim.token, claim.old_snapshot)
        {
            windows.destroy(label);
            return Err(error);
        }
        return Err(LongshotIpcError::new(
            "longshot_controller_hide_failed",
            hide_error.message,
        ));
    }

    boundary(AppendBoundary::AfterHide);
    if !registry.owns_append(label, &claim.token) {
        return Err(LongshotIpcError::superseded());
    }
    settle().await;
    boundary(AppendBoundary::AfterSettle);
    if !registry.owns_append(label, &claim.token) {
        return Err(LongshotIpcError::superseded());
    }

    let worker_result = worker(claim.token.clone()).await;
    boundary(AppendBoundary::AfterWorker);
    if !registry.owns_append(label, &claim.token) {
        return Err(LongshotIpcError::superseded());
    }
    if let Err(show_error) = windows.show(label) {
        return Err(terminate_hidden_append(
            registry,
            label,
            &claim.token,
            windows,
            cancel,
            show_error,
        )
        .await);
    }
    boundary(AppendBoundary::AfterShow);
    if !registry.owns_append(label, &claim.token) {
        // show 与 ownership 核验之间若被取消，强销毁可能被迟到 show 暴露的旧窗口。
        windows.destroy(label);
        return Err(LongshotIpcError::superseded());
    }
    windows.focus(label);
    boundary(AppendBoundary::BeforeCommit);
    if !registry.owns_append(label, &claim.token) {
        windows.destroy(label);
        return Err(LongshotIpcError::superseded());
    }

    match worker_result {
        Ok(snapshot) => registry.complete_append_visible(label, &claim.token, snapshot),
        Err(error) => {
            registry.complete_append_visible(label, &claim.token, claim.old_snapshot)?;
            Err(error)
        }
    }
}

async fn run_append_worker<F>(work: F) -> Result<LongshotSnapshot, LongshotIpcError>
where
    F: FnOnce() -> Result<LongshotSnapshot, CaptureError> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(work).await {
        Ok(Ok(snapshot)) => Ok(snapshot),
        Ok(Err(error)) => Err(error.into()),
        Err(error) => Err(LongshotIpcError::internal(format!(
            "长截图追加线程异常: {error}"
        ))),
    }
}

async fn run_preview_worker<F>(work: F) -> Result<Vec<u8>, LongshotIpcError>
where
    F: FnOnce() -> Result<Vec<u8>, CaptureError> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(work).await {
        Ok(Ok(png)) => Ok(png),
        Ok(Err(error)) => Err(error.into()),
        Err(error) => Err(LongshotIpcError::internal(format!(
            "长截图预览线程异常: {error}"
        ))),
    }
}

async fn execute_preview_with_ops<W, WFut, H>(
    registry: &LongshotControllerRegistry,
    label: &str,
    handle: &LongshotControllerHandle,
    worker: W,
    after_worker: H,
) -> Result<Vec<u8>, LongshotIpcError>
where
    W: FnOnce(LongshotSessionToken) -> WFut,
    WFut: std::future::Future<Output = Result<Vec<u8>, LongshotIpcError>>,
    H: FnOnce(),
{
    let token = registry.authorize_preview(label, handle)?;
    let worker_result = worker(token.clone()).await;
    after_worker();
    if !registry.confirms_preview(label, &token)? {
        return Err(LongshotIpcError::superseded());
    }
    worker_result
}

fn recover_cancel_failure_with_ops<A, P>(
    registry: &LongshotControllerRegistry,
    label: &str,
    token: &LongshotSessionToken,
    windows: &A,
    primary: CaptureError,
    probe_exact_active: P,
) -> Result<(), LongshotIpcError>
where
    A: ControlWindowActions,
    P: FnOnce() -> bool,
{
    let recovery = registry.begin_cancel_failure_recovery(label, token);
    match recovery {
        CancelFailureRecovery::Revealed => {
            let retryable = windows.exists(label) && probe_exact_active();
            if registry.complete_cancel_failure(label, token, retryable)
                == CancelFailureRecovery::Revealed
            {
                Err(primary.into())
            } else {
                Err(LongshotIpcError::cleanup_failed(primary.to_string()))
            }
        }
        CancelFailureRecovery::Hidden => {
            let retryable = windows.exists(label) && probe_exact_active();
            if !retryable || !registry.owns_hidden_cancel_reveal(label, token) {
                registry.fail_hidden_cancel_reveal(label, token);
                return Err(LongshotIpcError::cleanup_failed(primary.to_string()));
            }
            if windows.show(label).is_err() {
                registry.fail_hidden_cancel_reveal(label, token);
                return Err(LongshotIpcError::cleanup_failed(primary.to_string()));
            }
            if !registry.owns_hidden_cancel_reveal(label, token) {
                windows.destroy(label);
                registry.fail_hidden_cancel_reveal(label, token);
                return Err(LongshotIpcError::cleanup_failed(primary.to_string()));
            }
            windows.focus(label);
            if registry.complete_hidden_cancel_reveal(label, token) {
                Err(primary.into())
            } else {
                windows.destroy(label);
                registry.fail_hidden_cancel_reveal(label, token);
                Err(LongshotIpcError::cleanup_failed(primary.to_string()))
            }
        }
        CancelFailureRecovery::CleanupFailed => {
            log::error!("长截图控制窗 {label} 清理失败且状态不可安全重试: {primary}");
            Err(LongshotIpcError::cleanup_failed(primary.to_string()))
        }
    }
}

fn complete_build_attempt<A: ControlWindowActions>(
    registry: &LongshotControllerRegistry,
    label: &str,
    build_result: Result<(), String>,
    windows: &A,
) -> Result<(), LongshotIpcError> {
    if let Err(error) = build_result {
        registry.abort_build(label);
        windows.destroy(label);
        return Err(LongshotIpcError::new(
            "longshot_controller_create_failed",
            error,
        ));
    }
    if !registry.accepts_built_window(label) {
        windows.destroy(label);
        return Err(LongshotIpcError::missing());
    }
    Ok(())
}

fn open_with_ops<A, V, D, B>(
    registry: &LongshotControllerRegistry,
    caller_label: &str,
    selection: CaptureSelection,
    windows: &A,
    validate: V,
    arm_deadline: D,
    build: B,
) -> Result<LongshotControllerLaunch, LongshotIpcError>
where
    A: ControlWindowActions,
    V: FnOnce(&str, &CaptureSelection) -> Result<(), CaptureError>,
    D: FnOnce(&str),
    B: FnOnce(&str) -> Result<(), String>,
{
    if !caller_label.starts_with("capture-overlay-") {
        return Err(LongshotIpcError::new(
            "longshot_controller_caller_invalid",
            "只有当前截图覆盖层可以创建长截图控制窗口",
        ));
    }
    validate(caller_label, &selection)?;
    let label = registry.reserve(caller_label.to_string(), selection)?;
    arm_deadline(&label);
    let build_result = build(&label);
    complete_build_attempt(registry, &label, build_result, windows)?;
    Ok(LongshotControllerLaunch { label })
}

struct ActivationWorkerJoinFailure {
    error: LongshotIpcError,
    cleanup_recorded: bool,
}

async fn run_activation_worker<F>(
    registry: &LongshotControllerRegistry,
    label: &str,
    work: F,
) -> Result<Result<super::LongshotStart, CaptureError>, ActivationWorkerJoinFailure>
where
    F: FnOnce() -> Result<super::LongshotStart, CaptureError> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(work).await {
        Ok(result) => Ok(result),
        Err(error) => {
            let cleanup_recorded = registry.mark_cleanup_failed(label, None);
            Err(ActivationWorkerJoinFailure {
                error: LongshotIpcError::cleanup_failed(format!("长截图启动线程异常: {error}")),
                cleanup_recorded,
            })
        }
    }
}

fn reveal_cleanup_fallback(app: &tauri::AppHandle, label: &str) -> bool {
    let Some(window) = app.get_webview_window(label) else {
        log::error!("长截图清理失败且控制窗 {label} 已不存在");
        return false;
    };
    if let Err(error) = window.show() {
        log::error!("长截图清理失败后兜底显示控制窗 {label} 失败: {error}");
        return false;
    }
    let _ = window.set_focus();
    true
}

/// 普通覆盖层必须收到二阶段接管结果；token/窗口标签防迟到结果解锁新的尝试。
fn notify_handoff(app: &tauri::AppHandle, state: &AppState, label: &str, accepted: bool) {
    // 普通会话已消费时，失败不能伪装成能继续编辑；原覆盖层也已由lifecycle关闭。
    if let Some((caller, result)) =
        state
            .longshot_windows
            .take_handoff(label, accepted, |session_id| {
                state.capture_manager.ensure_current(session_id).is_ok()
            })
    {
        let _ = app.emit_to(caller, "capture-longshot-handoff", result);
    }
}

fn spawn_deadline(app: tauri::AppHandle, label: String) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(LOAD_DEADLINE_MS)).await;
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let action = state.longshot_windows.claim_deadline(&label);
        let windows = TauriControlWindowActions { app: &app };
        execute_deadline_action(action, &label, &windows, |token| {
            cancel_claimed(&app, &state, &label, token)
        })
        .await;
        notify_handoff(&app, &state, &label, false);
    });
}

pub(crate) fn open(
    app: &tauri::AppHandle,
    state: &AppState,
    caller_label: &str,
    selection: CaptureSelection,
) -> Result<LongshotControllerLaunch, LongshotIpcError> {
    let registry = state.longshot_windows.clone();
    open_with_ops(
        &state.longshot_windows,
        caller_label,
        selection,
        &TauriControlWindowActions { app },
        |label, selection| {
            state
                .capture_manager
                .validate_longshot_open(label, selection)
        },
        |label| spawn_deadline(app.clone(), label.to_string()),
        |label| {
            let callback_label = label.to_string();
            let callback_registry = registry.clone();
            tauri::WebviewWindowBuilder::new(
                app,
                label,
                tauri::WebviewUrl::App("longshot-controller.html".into()),
            )
            .title("")
            .inner_size(400.0, 560.0)
            .decorations(false)
            .resizable(false)
            .skip_taskbar(true)
            .always_on_top(true)
            .focused(false)
            .visible(false)
            .center()
            .on_page_load(move |window, payload| {
                if payload.event() == tauri::webview::PageLoadEvent::Started
                    && window.label() == callback_label
                {
                    callback_registry.publish_started(&callback_label, payload.url().path());
                }
            })
            .build()
            .map(|_| ())
            .map_err(|error| error.to_string())
        },
    )
}

pub(crate) async fn activate(
    app: tauri::AppHandle,
    state: &AppState,
    caller_label: &str,
) -> Result<LongshotActivation, LongshotIpcError> {
    let result = activate_inner(app.clone(), state, caller_label).await;
    notify_handoff(&app, state, caller_label, result.is_ok());
    result
}

async fn activate_inner(
    app: tauri::AppHandle,
    state: &AppState,
    caller_label: &str,
) -> Result<LongshotActivation, LongshotIpcError> {
    let selection = state.longshot_windows.claim_activation(caller_label)?;
    let lifecycle = state.longshot_lifecycle.clone();
    let capture = state.capture_manager.clone();
    let worker_app = app.clone();
    let result = match run_activation_worker(&state.longshot_windows, caller_label, move || {
        lifecycle.begin(&capture, &selection, &worker_app)
    })
    .await
    {
        Ok(result) => result,
        Err(failure) => {
            if !failure.cleanup_recorded && reveal_cleanup_fallback(&app, caller_label) {
                state.longshot_windows.mark_cleanup_revealed(caller_label);
            }
            return Err(failure.error);
        }
    };
    let started_token = result.as_ref().ok().map(|start| start.token.clone());
    let (activation, compensation) = match state
        .longshot_windows
        .complete_activation(caller_label, result)
    {
        Ok(completed) => completed,
        Err(error) => {
            // begin 已经成功时，registry 提交失败也不能遗失唯一 token。
            let _ = state
                .longshot_windows
                .mark_cleanup_failed(caller_label, started_token.clone());
            let decision = if let Some(token) = started_token {
                let lifecycle = state.longshot_lifecycle.clone();
                let cleanup_app = app.clone();
                let cleanup_token = token.clone();
                let cleanup = tauri::async_runtime::spawn_blocking(move || {
                    let cleanup_state = cleanup_app
                        .try_state::<AppState>()
                        .ok_or_else(|| CaptureError::StateLock("AppState 已不可用".to_string()))?;
                    lifecycle.cancel(&cleanup_token, &cleanup_app, &cleanup_state)
                })
                .await;
                let cleanup_succeeded = matches!(cleanup, Ok(Ok(())));
                let settled = state.longshot_windows.settle_emergency_cleanup(
                    caller_label,
                    &token,
                    cleanup_succeeded,
                );
                if !cleanup_succeeded {
                    log::error!("长截图启动完成但 registry 提交失败，补偿清理也失败");
                }
                if !settled {
                    log::error!("长截图补偿后无法确认控制窗 registry 已收敛，只能等待进程重启");
                }
                emergency_decision(cleanup_succeeded, settled)
            } else {
                EmergencyDecision::Reveal
            };
            match decision {
                EmergencyDecision::Destroy => {
                    TauriControlWindowActions { app: &app }.destroy(caller_label)
                }
                EmergencyDecision::AwaitReady => {}
                EmergencyDecision::Reveal => {
                    if reveal_cleanup_fallback(&app, caller_label) {
                        state.longshot_windows.mark_cleanup_revealed(caller_label);
                    }
                }
            }
            return Err(LongshotIpcError::cleanup_failed(error.message));
        }
    };
    if let Some(token) = compensation {
        execute_token_cleanup(Some(token), |token| {
            cancel_claimed(&app, state, caller_label, token)
        })
        .await;
        return Err(LongshotIpcError::missing());
    }
    activation.ok_or_else(LongshotIpcError::missing)
}

pub(crate) async fn ready(
    app: tauri::AppHandle,
    state: &AppState,
    caller_label: &str,
) -> Result<(), LongshotIpcError> {
    let action = state.longshot_windows.claim_ready(caller_label)?;
    execute_ready_action(
        &state.longshot_windows,
        action,
        caller_label,
        &TauriControlWindowActions { app: &app },
        |token| cancel_claimed(&app, state, caller_label, token),
    )
    .await
}

pub(crate) async fn append(
    app: tauri::AppHandle,
    state: &AppState,
    caller_label: &str,
    handle: LongshotControllerHandle,
) -> Result<LongshotSnapshotDto, LongshotIpcError> {
    let longshot_lifecycle = state.longshot_lifecycle.clone();
    execute_append_with_ops(
        &state.longshot_windows,
        caller_label,
        &handle,
        &TauriControlWindowActions { app: &app },
        || async {
            tokio::time::sleep(Duration::from_millis(crate::capture::HIDE_SETTLE_MS)).await;
        },
        move |token| {
            let longshot_lifecycle = longshot_lifecycle.clone();
            async move {
                run_append_worker(move || {
                    let outcome = longshot_lifecycle.append(&token)?;
                    Ok(outcome.snapshot)
                })
                .await
            }
        },
        |token| cancel_claimed(&app, state, caller_label, token),
        |_| {},
    )
    .await
}

pub(crate) async fn preview(
    state: &AppState,
    caller_label: &str,
    handle: LongshotControllerHandle,
) -> Result<tauri::ipc::Response, LongshotIpcError> {
    let lifecycle = state.longshot_lifecycle.clone();
    let png =
        execute_preview_with_ops(
            &state.longshot_windows,
            caller_label,
            &handle,
            move |token| async move {
                run_preview_worker(move || lifecycle.preview_tail_png(&token)).await
            },
            || {},
        )
        .await?;
    Ok(tauri::ipc::Response::new(png))
}

pub(crate) async fn finish(
    app: tauri::AppHandle,
    state: &AppState,
    caller_label: &str,
    handle: LongshotControllerHandle,
    action: LongshotOutputAction,
) -> Result<LongshotOutputResult, LongshotIpcError> {
    let lifecycle = state.longshot_lifecycle.clone();
    let finish_app = app.clone();
    let save_target = state.save_target();
    let pin_origins = Arc::clone(&state.pin_origins);
    let output_app = app.clone();
    execute_finish_with_ops(
        &state.longshot_windows,
        caller_label,
        &handle,
        action,
        &TauriControlWindowActions { app: &app },
        FinishOperations::new(
            move |token| {
                let lifecycle = lifecycle.clone();
                async move {
                    run_finish_worker(move || {
                        let state = finish_app.try_state::<AppState>().ok_or_else(|| {
                            CaptureError::StateLock("AppState 已不可用".to_string())
                        })?;
                        lifecycle.finish_png(&token, &finish_app, &state)
                    })
                    .await
                }
            },
            |token: &LongshotSessionToken| {
                state
                    .longshot_lifecycle
                    .is_exact_active(token)
                    .unwrap_or(false)
            },
            move |requested_action, artifact: Arc<LongshotOutputArtifact>| {
                let output_app = output_app.clone();
                async move {
                    match requested_action {
                        LongshotOutputAction::Copy => {
                            run_output_worker(requested_action, move || {
                                copy_longshot_artifact(&artifact, &pin_origins, |image| {
                                    crate::clipboard_watcher::clipboard_set_image_with_retry(image)
                                })?;
                                Ok(OutputValue::None)
                            })
                            .await
                        }
                        LongshotOutputAction::Save => {
                            run_output_worker(requested_action, move || {
                                let path = crate::image_io::save_png(
                                    artifact.png.as_slice(),
                                    "clippy-screenshot",
                                    &save_target,
                                )?;
                                Ok(OutputValue::SavePath(path.to_string_lossy().into_owned()))
                            })
                            .await
                        }
                        LongshotOutputAction::Pin => {
                            let state = output_app.try_state::<AppState>().ok_or_else(|| {
                                OutputWorkerError::Business("AppState 已不可用".to_string())
                            })?;
                            pin_longshot_artifact(&artifact, |png, origin| {
                                crate::pin::commands::create_screenshot_pin_shared(
                                    png,
                                    Some(origin),
                                    &output_app,
                                    &state,
                                )
                            })
                            .map(OutputValue::PinLabel)
                        }
                    }
                }
            },
            |token| cancel_claimed(&app, state, caller_label, token),
            |_| {},
        ),
    )
    .await
}

pub(crate) async fn cancel(
    app: tauri::AppHandle,
    state: &AppState,
    caller_label: &str,
    handle: Option<LongshotControllerHandle>,
) -> Result<(), LongshotIpcError> {
    let action = state
        .longshot_windows
        .claim_cancel(caller_label, handle.as_ref())?;
    let windows = TauriControlWindowActions { app: &app };
    let result = execute_cancel_action(action, caller_label, &windows, |token| {
        cancel_claimed(&app, state, caller_label, token)
    })
    .await;
    notify_handoff(&app, state, caller_label, false);
    result
}

async fn cancel_claimed(
    app: &tauri::AppHandle,
    state: &AppState,
    label: &str,
    token: LongshotSessionToken,
) -> Result<(), LongshotIpcError> {
    let lifecycle = state.longshot_lifecycle.clone();
    let worker_app = app.clone();
    let worker_token = token.clone();
    let result = match tauri::async_runtime::spawn_blocking(move || {
        let state = worker_app
            .try_state::<AppState>()
            .ok_or_else(|| CaptureError::StateLock("AppState 已不可用".to_string()))?;
        lifecycle.cancel(&worker_token, &worker_app, &state)
    })
    .await
    {
        Ok(result) => result,
        Err(error) => {
            let _ = state
                .longshot_windows
                .mark_cleanup_failed(label, Some(token));
            return Err(LongshotIpcError::cleanup_failed(format!(
                "长截图取消线程异常: {error}"
            )));
        }
    };
    match result {
        Ok(()) => {
            state
                .longshot_windows
                .complete_cancel_success(label, &token);
            Ok(())
        }
        Err(primary) => {
            let windows = TauriControlWindowActions { app };
            recover_cancel_failure_with_ops(
                &state.longshot_windows,
                label,
                &token,
                &windows,
                primary,
                || {
                    state
                        .longshot_lifecycle
                        .is_exact_active(&token)
                        .unwrap_or(false)
                },
            )
        }
    }
}

pub(crate) fn handle_controller_destroyed(app: &tauri::AppHandle, label: &str) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let token = state.longshot_windows.claim_destroyed(label);
    notify_handoff(app, &state, label, false);
    if token.is_none() {
        return;
    }
    let app = app.clone();
    let label = label.to_string();
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        execute_token_cleanup(token, |token| cancel_claimed(&app, &state, &label, token)).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::manager::StageTimings;
    use crate::capture::{CaptureManager, CaptureMode, CaptureModeGate};
    use crate::screenshot::CapturedMonitorFrame;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    #[derive(Default)]
    struct TestWindows {
        existing: Mutex<bool>,
        named_alive: Mutex<std::collections::HashSet<String>>,
        attempts: Mutex<Vec<String>>,
        destroyed: Mutex<Vec<String>>,
        shown: Mutex<Vec<String>>,
        hidden: Mutex<Vec<String>>,
        focused: Mutex<Vec<String>>,
        trace: Mutex<Vec<String>>,
        hide_fails: AtomicBool,
        show_fails: AtomicBool,
    }

    impl TestWindows {
        fn set_existing(&self, existing: bool) {
            *self.existing.lock().expect("existing") = existing;
        }

        fn destroy_count(&self) -> usize {
            self.destroyed.lock().expect("destroyed").len()
        }

        fn attempt_count(&self) -> usize {
            self.attempts.lock().expect("attempts").len()
        }

        fn destroyed_labels(&self) -> Vec<String> {
            self.destroyed.lock().expect("destroyed").clone()
        }

        fn add_alive(&self, label: &str) {
            self.named_alive
                .lock()
                .expect("named alive")
                .insert(label.to_string());
        }

        fn is_alive(&self, label: &str) -> bool {
            self.named_alive
                .lock()
                .expect("named alive")
                .contains(label)
        }

        fn set_show_fails(&self, fails: bool) {
            self.show_fails.store(fails, Ordering::SeqCst);
        }

        fn set_hide_fails(&self, fails: bool) {
            self.hide_fails.store(fails, Ordering::SeqCst);
        }

        fn hide_count(&self) -> usize {
            self.hidden.lock().expect("hidden").len()
        }

        fn show_count(&self) -> usize {
            self.shown.lock().expect("shown").len()
        }

        fn focus_count(&self) -> usize {
            self.focused.lock().expect("focused").len()
        }

        fn trace(&self) -> Vec<String> {
            self.trace.lock().expect("trace").clone()
        }

        fn record(&self, event: impl Into<String>) {
            self.trace.lock().expect("trace").push(event.into());
        }
    }

    impl ControlWindowActions for TestWindows {
        fn destroy(&self, label: &str) {
            self.record("destroy");
            self.attempts
                .lock()
                .expect("attempts")
                .push(label.to_string());
            let mut existing = self.existing.lock().expect("existing");
            let named = self.named_alive.lock().expect("named alive").remove(label);
            if *existing || named {
                *existing = false;
                self.destroyed
                    .lock()
                    .expect("destroyed")
                    .push(label.to_string());
            }
        }

        fn hide(&self, label: &str) -> Result<(), LongshotIpcError> {
            self.record("hide");
            self.hidden.lock().expect("hidden").push(label.to_string());
            if self.hide_fails.load(Ordering::SeqCst) {
                Err(LongshotIpcError::new(
                    "longshot_controller_hide_failed",
                    "injected hide failure",
                ))
            } else {
                Ok(())
            }
        }

        fn show(&self, label: &str) -> Result<(), LongshotIpcError> {
            self.record("show");
            self.shown.lock().expect("shown").push(label.to_string());
            if self.show_fails.load(Ordering::SeqCst) {
                Err(LongshotIpcError::new(
                    "longshot_controller_show_failed",
                    "injected show failure",
                ))
            } else {
                Ok(())
            }
        }

        fn focus(&self, label: &str) {
            self.record("focus");
            self.focused
                .lock()
                .expect("focused")
                .push(label.to_string());
        }

        fn exists(&self, label: &str) -> bool {
            *self.existing.lock().expect("existing")
                || self
                    .named_alive
                    .lock()
                    .expect("named alive")
                    .contains(label)
        }
    }

    struct DestroyOnShowWindows<'a> {
        registry: &'a LongshotControllerRegistry,
        label: &'a str,
        inner: TestWindows,
    }

    impl ControlWindowActions for DestroyOnShowWindows<'_> {
        fn destroy(&self, label: &str) {
            self.inner.destroy(label);
        }

        fn hide(&self, label: &str) -> Result<(), LongshotIpcError> {
            self.inner.hide(label)
        }

        fn show(&self, label: &str) -> Result<(), LongshotIpcError> {
            self.inner.show(label)?;
            let _ = self.registry.claim_destroyed(self.label);
            Ok(())
        }

        fn focus(&self, label: &str) {
            self.inner.focus(label);
        }

        fn exists(&self, label: &str) -> bool {
            self.inner.exists(label)
        }
    }

    fn pending_registry() -> (LongshotControllerRegistry, String) {
        let registry = LongshotControllerRegistry::new();
        let label = registry
            .reserve("capture-overlay-a-7".to_string(), selection())
            .expect("reserve");
        assert!(registry.publish_started(&label, CONTROLLER_PAGE));
        (registry, label)
    }

    fn failed_registry() -> (LongshotControllerRegistry, String) {
        let (registry, label) = pending_registry();
        registry.claim_activation(&label).expect("claim");
        let _ = registry.complete_activation(&label, Err(CaptureError::SessionMissing));
        (registry, label)
    }

    fn ordinary_fixture() -> (
        CaptureManager,
        Arc<CaptureModeGate>,
        String,
        String,
        CaptureSelection,
    ) {
        let manager = CaptureManager::new();
        let gate = Arc::new(CaptureModeGate::new());
        let ownership = gate
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("Ordinary ownership");
        let frame = CapturedMonitorFrame {
            monitor_id: 7,
            x: 0,
            y: 0,
            logical_width: 100,
            logical_height: 80,
            pixel_width: 100,
            pixel_height: 80,
            scale_x: 1.0,
            scale_y: 1.0,
            rgba: Arc::from(vec![255; 100 * 80 * 4]),
        };
        let start = manager
            .begin(
                vec![frame],
                vec!["main".to_string()],
                vec!["pin-a".to_string()],
                false,
                StageTimings::default(),
                ownership,
            )
            .expect("ordinary begin");
        let caller = start.overlays[0].label.clone();
        let selection = CaptureSelection {
            session_id: start.session_id.clone(),
            monitor_id: 7,
            x: 1.0,
            y: 2.0,
            width: 30.0,
            height: 40.0,
        };
        (manager, gate, start.session_id, caller, selection)
    }

    fn assert_and_finish_ordinary(
        manager: &CaptureManager,
        gate: &CaptureModeGate,
        session_id: &str,
        caller: &str,
    ) {
        assert!(manager.payload(caller).is_ok());
        assert_eq!(
            gate.active_mode().expect("gate"),
            Some(CaptureMode::Ordinary)
        );
        let session = manager.finish(session_id).expect("ordinary intact");
        assert_eq!(session.restore_labels, vec!["main"]);
        assert_eq!(session.lowered_pins, vec!["pin-a"]);
        session.finalize_mode().expect("release");
    }

    fn start(generation: u64) -> super::super::LongshotStart {
        super::super::LongshotStart {
            token: LongshotSessionToken::from_wire_parts("longshot".to_string(), generation),
            snapshot: LongshotSnapshot {
                frame_count: 1,
                width: 30,
                frame_height: 40,
                total_height: 40,
            },
        }
    }

    fn active_registry() -> (LongshotControllerRegistry, String, LongshotSessionToken) {
        let registry = LongshotControllerRegistry::new();
        let label = registry
            .reserve("capture-overlay-a-7".to_string(), selection())
            .expect("reserve");
        assert!(registry.publish_started(&label, CONTROLLER_PAGE));
        registry.claim_activation(&label).expect("claim");
        let (activation, compensation) = registry
            .complete_activation(&label, Ok(start(9)))
            .expect("complete");
        assert!(activation.is_some());
        assert!(compensation.is_none());
        let token = LongshotSessionToken::from_wire_parts("longshot".to_string(), 9);
        (registry, label, token)
    }

    fn revealed_active_registry() -> (LongshotControllerRegistry, String, LongshotSessionToken) {
        let (registry, label, token) = active_registry();
        assert!(matches!(
            registry.claim_ready(&label),
            Ok(ReadyAction::ShowActive(_))
        ));
        (registry, label, token)
    }

    #[test]
    fn preview_authorization_is_read_only_and_requires_exact_revealed_active() {
        let (registry, label, token) = active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        assert_eq!(
            registry
                .authorize_preview(&label, &handle)
                .expect_err("未 reveal 不可预览")
                .code,
            "longshot_controller_missing"
        );
        assert!(matches!(
            &*registry.slot.lock().expect("slot"),
            Slot::Active {
                revealed: false,
                ..
            }
        ));
        assert!(matches!(
            registry.claim_ready(&label),
            Ok(ReadyAction::ShowActive(_))
        ));

        assert_eq!(
            registry
                .authorize_preview("main", &handle)
                .expect_err("伪 label 不可预览")
                .code,
            "longshot_controller_missing"
        );
        assert_eq!(
            registry
                .authorize_preview("longshot-controller-other", &handle)
                .expect_err("同前缀其他 label 不可预览")
                .code,
            "longshot_controller_missing"
        );
        let malformed = LongshotControllerHandle {
            session_id: handle.session_id.clone(),
            generation: "01".to_string(),
        };
        assert_eq!(
            registry
                .authorize_preview(&label, &malformed)
                .expect_err("畸形 generation 不可预览")
                .code,
            "longshot_controller_superseded"
        );
        let stale = LongshotControllerHandle {
            session_id: handle.session_id.clone(),
            generation: "8".to_string(),
        };
        assert_eq!(
            registry
                .authorize_preview(&label, &stale)
                .expect_err("旧 generation 不可预览")
                .code,
            "longshot_controller_superseded"
        );
        assert_eq!(registry.authorize_preview(&label, &handle).unwrap(), token);
        assert!(registry.confirms_preview(&label, &token).unwrap());
        assert!(matches!(
            &*registry.slot.lock().expect("slot"),
            Slot::Active {
                label: current,
                token: current_token,
                revealed: true,
                ..
            } if current == &label && current_token == &token
        ));
    }

    #[test]
    fn preview_authorization_reports_busy_for_every_exact_worker_phase() {
        let (appending, append_label, append_token) = revealed_active_registry();
        let append_handle = LongshotControllerHandle::from_token(&append_token);
        appending
            .claim_append(&append_label, &append_handle)
            .expect("进入 Appending");
        assert_eq!(
            appending
                .authorize_preview(&append_label, &append_handle)
                .expect_err("Appending 应 busy")
                .code,
            "longshot_controller_busy"
        );
        let stale_append_handle = LongshotControllerHandle {
            session_id: append_handle.session_id.clone(),
            generation: "8".to_string(),
        };
        assert_eq!(
            appending
                .authorize_preview(&append_label, &stale_append_handle)
                .expect_err("Appending 的旧 handle 应 superseded")
                .code,
            "longshot_controller_superseded"
        );

        let (finishing, finish_label, finish_token) = revealed_active_registry();
        let finish_handle = LongshotControllerHandle::from_token(&finish_token);
        finishing
            .claim_finish(&finish_label, &finish_handle, LongshotOutputAction::Copy)
            .expect("进入 Finishing");
        assert_eq!(
            finishing
                .authorize_preview(&finish_label, &finish_handle)
                .expect_err("Finishing 应 busy")
                .code,
            "longshot_controller_busy"
        );

        let (pending, pending_label, pending_token) = revealed_active_registry();
        let pending_handle = LongshotControllerHandle::from_token(&pending_token);
        let snapshot = start(9).snapshot;
        *pending.slot.lock().expect("slot") = Slot::OutputPending {
            label: pending_label.clone(),
            token: pending_token.clone(),
            snapshot,
            artifact: Arc::new(LongshotOutputArtifact {
                png: Arc::new(vec![137, 80, 78, 71]),
                origin: test_origin(),
            }),
            retry_policy: RetryPolicy::Any,
        };
        assert_eq!(
            pending
                .authorize_preview(&pending_label, &pending_handle)
                .expect_err("OutputPending 应 busy")
                .code,
            "longshot_controller_busy"
        );
    }

    #[tokio::test]
    async fn preview_post_check_makes_phase_change_win_over_worker_error() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let error = execute_preview_with_ops(
            &registry,
            &label,
            &handle,
            |_| async { Err(LongshotIpcError::new("codec", "旧 worker 错误")) },
            || {
                registry
                    .claim_append(&label, &handle)
                    .expect("worker 后模拟 Append 获胜");
            },
        )
        .await
        .expect_err("阶段变化必须覆盖 worker 错误");
        assert_eq!(error.code, "longshot_controller_superseded");
    }

    fn selection() -> CaptureSelection {
        CaptureSelection {
            session_id: "ordinary".to_string(),
            monitor_id: 7,
            x: 1.0,
            y: 2.0,
            width: 30.0,
            height: 40.0,
        }
    }

    fn lifecycle_artifact(png: Vec<u8>) -> LongshotArtifact {
        LongshotArtifact {
            png,
            origin: test_origin(),
        }
    }

    fn test_origin() -> PinOrigin {
        PinOrigin {
            x: -12.5,
            y: 8.25,
            width: 320.5,
            height: 640.75,
        }
    }

    fn output_artifact(png: Vec<u8>) -> Arc<LongshotOutputArtifact> {
        Arc::new(LongshotOutputArtifact::from(lifecycle_artifact(png)))
    }

    #[test]
    fn generation_parser_is_canonical_and_lossless() {
        let handle = LongshotControllerHandle {
            session_id: "session".to_string(),
            generation: u64::MAX.to_string(),
        };
        assert_eq!(handle.to_token().expect("u64 max").wire_parts().1, u64::MAX);
        for invalid in ["", " 1", "+1", "-1", "01", "18446744073709551616"] {
            let handle = LongshotControllerHandle {
                session_id: "session".to_string(),
                generation: invalid.to_string(),
            };
            assert_eq!(
                handle.to_token().expect_err("必须拒绝").code,
                "longshot_controller_superseded"
            );
        }
    }

    #[test]
    fn pin_output_wire_contract_keeps_label_separate_from_save_path() {
        let action: LongshotOutputAction = serde_json::from_str("\"pin\"").expect("Pin action");
        assert_eq!(action, LongshotOutputAction::Pin);
        assert_eq!(serde_json::to_string(&action).unwrap(), "\"pin\"");

        let value = serde_json::to_value(LongshotOutputResult {
            action,
            path: None,
            pin_label: Some("pin-image-longshot".to_string()),
        })
        .expect("Pin result");
        assert_eq!(value["action"], "pin");
        assert_eq!(value["path"], serde_json::Value::Null);
        assert_eq!(value["pinLabel"], "pin-image-longshot");
    }

    #[test]
    fn started_barrier_requires_exact_first_path_and_label() {
        let registry = LongshotControllerRegistry::new();
        let label = registry
            .reserve("capture-overlay-a-7".to_string(), selection())
            .expect("reserve");
        assert!(!registry.publish_started(&label, "/wrong.html"));
        assert_eq!(
            registry
                .claim_activation(&label)
                .expect_err("Building 绝不能 activation")
                .code,
            "longshot_controller_missing"
        );
        assert!(!registry.publish_started("longshot-controller-old", CONTROLLER_PAGE));
        assert!(registry.publish_started(&label, CONTROLLER_PAGE));
        assert!(!registry.publish_started(&label, CONTROLLER_PAGE));
        assert!(registry.claim_activation(&label).is_ok());
    }

    #[test]
    fn deadline_before_publish_revokes_exact_launch_only() {
        let registry = LongshotControllerRegistry::new();
        let label = registry
            .reserve("capture-overlay-a-7".to_string(), selection())
            .expect("reserve");
        assert!(matches!(
            registry.claim_deadline("old"),
            DeadlineAction::None
        ));
        assert!(registry.accepts_built_window(&label));
        assert!(matches!(
            registry.claim_deadline(&label),
            DeadlineAction::Close
        ));
        assert!(!registry.accepts_built_window(&label));
        assert!(registry
            .reserve("capture-overlay-b-7".to_string(), selection())
            .is_ok());
    }

    #[test]
    fn cancel_during_activation_only_records_request() {
        let registry = LongshotControllerRegistry::new();
        let label = registry
            .reserve("capture-overlay-a-7".to_string(), selection())
            .expect("reserve");
        assert!(registry.publish_started(&label, CONTROLLER_PAGE));
        registry.claim_activation(&label).expect("activation claim");
        assert!(matches!(
            registry.claim_cancel(&label, None).expect("cancel"),
            CancelAction::Requested
        ));
        let error = CaptureError::SessionMissing;
        assert_eq!(
            registry
                .complete_activation(&label, Err(error))
                .expect_err("primary error")
                .code,
            "session_missing"
        );
        assert!(registry
            .reserve("capture-overlay-b-7".to_string(), selection())
            .is_ok());
    }

    #[test]
    fn failed_ready_is_idempotent_and_cancel_never_needs_handle() {
        let registry = LongshotControllerRegistry::new();
        let label = registry
            .reserve("capture-overlay-a-7".to_string(), selection())
            .expect("reserve");
        assert!(registry.publish_started(&label, CONTROLLER_PAGE));
        registry.claim_activation(&label).expect("activation claim");
        let _ = registry.complete_activation(&label, Err(CaptureError::SessionMissing));
        assert!(matches!(
            registry.claim_ready(&label),
            Ok(ReadyAction::ShowFailed)
        ));
        assert!(matches!(
            registry.claim_ready(&label),
            Ok(ReadyAction::None)
        ));
        assert!(matches!(
            registry.claim_cancel(&label, None),
            Ok(CancelAction::Close)
        ));
    }

    #[test]
    fn serde_rejects_numeric_generation() {
        let error =
            serde_json::from_str::<LongshotControllerHandle>(r#"{"sessionId":"s","generation":1}"#)
                .expect_err("JSON number 不能进入字符串 generation");
        assert!(error.to_string().contains("string"));
    }

    #[test]
    fn ready_show_failure_then_destroyed_has_one_termination_winner() {
        let (registry, label, token) = active_registry();
        assert!(matches!(
            registry.claim_ready(&label),
            Ok(ReadyAction::ShowActive(_))
        ));
        assert_eq!(
            registry.claim_forced_termination(&label, &token),
            Some(token.clone())
        );
        assert_eq!(registry.claim_destroyed(&label), None);
        registry.complete_cancel_success(&label, &token);
        assert!(registry
            .reserve("capture-overlay-next-7".to_string(), selection())
            .is_ok());
    }

    #[test]
    fn destroyed_before_ready_show_failure_has_one_termination_winner() {
        let (registry, label, token) = active_registry();
        assert_eq!(registry.claim_destroyed(&label), Some(token.clone()));
        assert_eq!(registry.claim_forced_termination(&label, &token), None);
        registry.complete_cancel_success(&label, &token);
    }

    #[test]
    fn deadline_cleanup_failure_never_reopens_invisible_active_slot() {
        let (registry, label, token) = active_registry();
        assert!(matches!(
            registry.claim_deadline(&label),
            DeadlineAction::Terminate(ref claimed) if claimed == &token
        ));
        registry.complete_cancel_failure(&label, &token, true);
        assert_eq!(
            registry
                .reserve("capture-overlay-next-7".to_string(), selection())
                .expect_err("CleanupFailed 必须阻止新窗口")
                .code,
            "longshot_controller_busy"
        );
    }

    #[test]
    fn stale_handle_cannot_claim_active_generation() {
        let (registry, label, token) = active_registry();
        let stale = LongshotControllerHandle {
            session_id: "longshot".to_string(),
            generation: "8".to_string(),
        };
        assert_eq!(
            registry
                .claim_cancel(&label, Some(&stale))
                .expect_err("旧代次")
                .code,
            "longshot_controller_superseded"
        );
        assert_eq!(registry.claim_destroyed(&label), Some(token));
    }

    #[test]
    fn cancelled_activation_success_is_compensated_without_publishing_active() {
        let registry = LongshotControllerRegistry::new();
        let label = registry
            .reserve("capture-overlay-a-7".to_string(), selection())
            .expect("reserve");
        assert!(registry.publish_started(&label, CONTROLLER_PAGE));
        registry.claim_activation(&label).expect("claim");
        assert!(matches!(
            registry.claim_cancel(&label, None),
            Ok(CancelAction::Requested)
        ));
        let (activation, compensation) = registry
            .complete_activation(&label, Ok(start(9)))
            .expect("late begin success");
        assert!(activation.is_none(), "Active 不能发布给已销毁窗口");
        let token = compensation.expect("必须补偿 cancel");
        assert_eq!(registry.claim_destroyed(&label), None);
        registry.complete_cancel_success(&label, &token);
        assert!(registry
            .reserve("capture-overlay-next-7".to_string(), selection())
            .is_ok());
    }

    #[test]
    fn explicit_cancel_before_destroyed_keeps_single_winner_and_can_retry_live_failure() {
        let (registry, label, token) = active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        assert!(matches!(
            registry.claim_cancel(&label, Some(&handle)),
            Ok(CancelAction::Terminate(ref claimed)) if claimed == &token
        ));
        assert_eq!(registry.claim_destroyed("longshot-controller-old"), None);
        registry.complete_cancel_failure(&label, &token, true);
        assert!(matches!(
            registry.claim_cancel(&label, Some(&handle)),
            Ok(CancelAction::Terminate(_))
        ));
        assert_eq!(registry.claim_destroyed(&label), None);
        registry.complete_cancel_success(&label, &token);
    }

    #[test]
    fn revealed_active_is_immune_to_deadline_but_destroyed_still_cleans_it() {
        let (registry, label, token) = active_registry();
        assert!(matches!(
            registry.claim_ready(&label),
            Ok(ReadyAction::ShowActive(_))
        ));
        assert!(matches!(
            registry.claim_deadline(&label),
            DeadlineAction::None
        ));
        assert_eq!(registry.claim_destroyed(&label), Some(token));
    }

    #[test]
    fn old_label_events_never_touch_new_reservation() {
        let registry = LongshotControllerRegistry::new();
        let old = registry
            .reserve("capture-overlay-a-7".to_string(), selection())
            .expect("old reserve");
        assert!(matches!(
            registry.claim_deadline(&old),
            DeadlineAction::Close
        ));
        let new = registry
            .reserve("capture-overlay-b-7".to_string(), selection())
            .expect("new reserve");
        assert_eq!(registry.claim_destroyed(&old), None);
        assert!(registry.accepts_built_window(&new));
    }

    #[test]
    fn every_nonempty_representative_blocks_duplicate_open() {
        let registry = LongshotControllerRegistry::new();
        let label = registry
            .reserve("capture-overlay-a-7".to_string(), selection())
            .expect("Building");
        assert_eq!(
            registry
                .reserve("capture-overlay-b-7".to_string(), selection())
                .expect_err("Building busy")
                .code,
            "longshot_controller_busy"
        );
        assert!(registry.publish_started(&label, CONTROLLER_PAGE));
        assert_eq!(
            registry
                .reserve("capture-overlay-b-7".to_string(), selection())
                .expect_err("Pending busy")
                .code,
            "longshot_controller_busy"
        );
        registry.claim_activation(&label).expect("Activating");
        assert_eq!(
            registry
                .reserve("capture-overlay-b-7".to_string(), selection())
                .expect_err("Activating busy")
                .code,
            "longshot_controller_busy"
        );
        let _ = registry.complete_activation(&label, Ok(start(9)));
        assert_eq!(
            registry
                .reserve("capture-overlay-b-7".to_string(), selection())
                .expect_err("Active busy")
                .code,
            "longshot_controller_busy"
        );
    }

    #[test]
    fn failed_terminating_and_cleanup_failed_all_block_duplicate_open() {
        let registry = LongshotControllerRegistry::new();
        let token = LongshotSessionToken::from_wire_parts("longshot".to_string(), 3);
        for slot in [
            Slot::Failed {
                label: "longshot-controller-failed".to_string(),
                revealed: false,
            },
            Slot::Terminating {
                label: "longshot-controller-terminating".to_string(),
                token: token.clone(),
                snapshot: Some(start(3).snapshot),
                window_destroyed: false,
                origin: TerminationOrigin::RevealedActive,
            },
            Slot::CleanupFailed {
                label: "longshot-controller-cleanup".to_string(),
                _token: Some(token.clone()),
                revealed: false,
            },
        ] {
            *registry.slot.lock().expect("test slot") = slot;
            assert_eq!(
                registry
                    .reserve("capture-overlay-b-7".to_string(), selection())
                    .expect_err("非空状态必须 Busy")
                    .code,
                "longshot_controller_busy"
            );
        }
    }

    #[test]
    fn max_generation_survives_json_round_trip_as_string() {
        let original = LongshotControllerHandle {
            session_id: "session".to_string(),
            generation: u64::MAX.to_string(),
        };
        let json = serde_json::to_string(&original).expect("serialize");
        assert!(json.contains(r#""generation":"18446744073709551615""#));
        let decoded: LongshotControllerHandle = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, original);
        assert_eq!(decoded.to_token().expect("parse").wire_parts().1, u64::MAX);
    }

    #[test]
    fn activation_worker_failure_is_revealable_but_never_closeable_or_reopenable() {
        let registry = LongshotControllerRegistry::new();
        let label = registry
            .reserve("capture-overlay-a-7".to_string(), selection())
            .expect("reserve");
        assert!(registry.publish_started(&label, CONTROLLER_PAGE));
        registry.claim_activation(&label).expect("Activating");
        assert!(registry.mark_cleanup_failed(&label, None));
        assert!(matches!(
            registry.claim_ready(&label),
            Ok(ReadyAction::ShowCleanup)
        ));
        assert!(matches!(
            registry.claim_ready(&label),
            Ok(ReadyAction::None)
        ));
        registry.rollback_cleanup_reveal(&label);
        assert!(matches!(
            registry.claim_ready(&label),
            Ok(ReadyAction::ShowCleanup)
        ));
        assert_eq!(
            registry
                .claim_cancel(&label, None)
                .expect_err("CleanupFailed 不能被普通 Close 清槽")
                .code,
            "longshot_controller_missing"
        );
        assert_eq!(registry.claim_destroyed(&label), None);
        assert_eq!(
            registry
                .reserve("capture-overlay-next-7".to_string(), selection())
                .expect_err("CleanupFailed 必须持续阻止新会话")
                .code,
            "longshot_controller_busy"
        );
    }

    #[test]
    fn emergency_cleanup_decision_matches_resource_certainty() {
        assert_eq!(emergency_decision(true, true), EmergencyDecision::Destroy);
        assert_eq!(
            emergency_decision(false, true),
            EmergencyDecision::AwaitReady
        );
        assert_eq!(emergency_decision(true, false), EmergencyDecision::Reveal);
        assert_eq!(emergency_decision(false, false), EmergencyDecision::Reveal);
    }

    #[test]
    fn emergency_cleanup_success_clears_exact_slot_but_failure_stays_revealable() {
        let registry = LongshotControllerRegistry::new();
        let label = registry
            .reserve("capture-overlay-a-7".to_string(), selection())
            .expect("reserve");
        assert!(registry.publish_started(&label, CONTROLLER_PAGE));
        registry.claim_activation(&label).expect("Activating");
        let token = LongshotSessionToken::from_wire_parts("longshot".to_string(), 9);
        assert!(registry.mark_cleanup_failed(&label, Some(token.clone())));
        assert!(registry.settle_emergency_cleanup(&label, &token, true));
        assert!(registry
            .reserve("capture-overlay-next-7".to_string(), selection())
            .is_ok());

        let failed = LongshotControllerRegistry::new();
        let failed_label = failed
            .reserve("capture-overlay-b-7".to_string(), selection())
            .expect("reserve failed path");
        assert!(failed.publish_started(&failed_label, CONTROLLER_PAGE));
        failed.claim_activation(&failed_label).expect("Activating");
        assert!(failed.mark_cleanup_failed(&failed_label, Some(token.clone())));
        assert!(failed.settle_emergency_cleanup(&failed_label, &token, false));
        assert!(matches!(
            failed.claim_ready(&failed_label),
            Ok(ReadyAction::ShowCleanup)
        ));
        assert_eq!(
            failed
                .reserve("capture-overlay-next-7".to_string(), selection())
                .expect_err("清理失败不能开放新会话")
                .code,
            "longshot_controller_busy"
        );
    }

    #[tokio::test]
    async fn pending_cancel_and_destroyed_both_orders_destroy_at_most_once() {
        let (registry, label) = pending_registry();
        let windows = TestWindows::default();
        windows.set_existing(true);
        let cancel_calls = AtomicUsize::new(0);
        let action = registry.claim_cancel(&label, None).expect("cancel first");
        execute_cancel_action(action, &label, &windows, |_| async {
            cancel_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await
        .expect("execute");
        assert_eq!(registry.claim_destroyed(&label), None);
        assert!(registry.claim_cancel(&label, None).is_err());
        assert_eq!(windows.destroy_count(), 1);
        assert_eq!(windows.attempt_count(), 1);
        assert_eq!(windows.destroyed_labels(), vec![label.clone()]);
        assert_eq!(cancel_calls.load(Ordering::SeqCst), 0);

        let (registry, label) = pending_registry();
        let windows = TestWindows::default();
        windows.set_existing(false); // external Destroyed 已经移除窗口
        assert_eq!(registry.claim_destroyed(&label), None);
        assert!(registry.claim_cancel(&label, None).is_err());
        assert_eq!(windows.destroy_count(), 0);
        assert_eq!(windows.attempt_count(), 0);
    }

    #[tokio::test]
    async fn failed_cancel_and_destroyed_both_orders_destroy_at_most_once() {
        let (registry, label) = failed_registry();
        let windows = TestWindows::default();
        windows.set_existing(true);
        let action = registry.claim_cancel(&label, None).expect("cancel first");
        execute_cancel_action(action, &label, &windows, |_| async { Ok(()) })
            .await
            .expect("execute");
        assert_eq!(registry.claim_destroyed(&label), None);
        assert!(registry.claim_cancel(&label, None).is_err());
        assert_eq!(windows.destroy_count(), 1);
        assert_eq!(windows.attempt_count(), 1);
        assert_eq!(windows.destroyed_labels(), vec![label.clone()]);

        let (registry, label) = failed_registry();
        let windows = TestWindows::default();
        assert_eq!(registry.claim_destroyed(&label), None);
        assert!(registry.claim_cancel(&label, None).is_err());
        assert_eq!(windows.destroy_count(), 0);
        assert_eq!(windows.attempt_count(), 0);
    }

    #[tokio::test]
    async fn activating_cancel_and_destroyed_orders_compensate_success_once() {
        for explicit_first in [true, false] {
            let (registry, label) = pending_registry();
            registry.claim_activation(&label).expect("Activating");
            let windows = TestWindows::default();
            windows.set_existing(explicit_first);
            if explicit_first {
                let action = registry.claim_cancel(&label, None).expect("cancel");
                execute_cancel_action(action, &label, &windows, |_| async { Ok(()) })
                    .await
                    .expect("destroy");
                assert_eq!(registry.claim_destroyed(&label), None);
            } else {
                assert_eq!(registry.claim_destroyed(&label), None);
                let late = registry
                    .claim_cancel(&label, None)
                    .expect("late cancel 幂等");
                execute_cancel_action(late, &label, &windows, |_| async { Ok(()) })
                    .await
                    .expect("late cancel executor");
            }
            let (activation, compensation) = registry
                .complete_activation(&label, Ok(start(11)))
                .expect("late success");
            assert!(activation.is_none());
            let token = compensation.expect("compensation token");
            let cancel_calls = AtomicUsize::new(0);
            execute_token_cleanup(Some(token), |claimed| {
                cancel_calls.fetch_add(1, Ordering::SeqCst);
                registry.complete_cancel_success(&label, &claimed);
                std::future::ready(Ok(()))
            })
            .await;
            assert_eq!(cancel_calls.load(Ordering::SeqCst), 1);
            assert_eq!(windows.destroy_count(), usize::from(explicit_first));
            assert_eq!(windows.attempt_count(), 1);
        }
    }

    #[tokio::test]
    async fn activating_cancel_and_destroyed_orders_late_failure_never_cancel_lifecycle() {
        for explicit_first in [true, false] {
            let (registry, label) = pending_registry();
            registry.claim_activation(&label).expect("Activating");
            let windows = TestWindows::default();
            windows.set_existing(explicit_first);
            if explicit_first {
                let action = registry.claim_cancel(&label, None).expect("cancel");
                execute_cancel_action(action, &label, &windows, |_| async { Ok(()) })
                    .await
                    .expect("destroy");
                assert_eq!(registry.claim_destroyed(&label), None);
            } else {
                assert_eq!(registry.claim_destroyed(&label), None);
                let late = registry
                    .claim_cancel(&label, None)
                    .expect("late cancel 幂等");
                execute_cancel_action(late, &label, &windows, |_| async { Ok(()) })
                    .await
                    .expect("late cancel executor");
            }
            let error = registry
                .complete_activation(&label, Err(CaptureError::SessionMissing))
                .expect_err("late begin failure");
            assert_eq!(error.code, "session_missing");
            assert_eq!(windows.destroy_count(), usize::from(explicit_first));
            assert_eq!(windows.attempt_count(), 1);
            assert!(registry
                .reserve("capture-overlay-next-7".to_string(), selection())
                .is_ok());
        }
    }

    #[tokio::test]
    async fn active_cancel_and_destroyed_both_orders_cancel_and_destroy_at_most_once() {
        let (registry, label, token) = active_registry();
        let windows = TestWindows::default();
        windows.set_existing(true);
        let calls = AtomicUsize::new(0);
        let action = registry
            .claim_cancel(&label, Some(&LongshotControllerHandle::from_token(&token)))
            .expect("cancel first");
        execute_cancel_action(action, &label, &windows, |claimed| {
            assert_eq!(claimed, token);
            calls.fetch_add(1, Ordering::SeqCst);
            registry.complete_cancel_success(&label, &claimed);
            std::future::ready(Ok(()))
        })
        .await
        .expect("execute");
        assert_eq!(registry.claim_destroyed(&label), None);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(windows.destroy_count(), 1);
        assert_eq!(windows.attempt_count(), 1);
        assert_eq!(windows.destroyed_labels(), vec![label.clone()]);

        let (registry, label, token) = active_registry();
        let windows = TestWindows::default();
        let claimed = registry.claim_destroyed(&label).expect("Destroyed wins");
        assert_eq!(claimed, token);
        let calls = AtomicUsize::new(0);
        execute_token_cleanup(Some(claimed), |claimed| {
            calls.fetch_add(1, Ordering::SeqCst);
            registry.complete_cancel_success(&label, &claimed);
            std::future::ready(Ok(()))
        })
        .await;
        assert!(registry
            .claim_cancel(&label, Some(&LongshotControllerHandle::from_token(&token)))
            .is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(windows.destroy_count(), 0);
        assert_eq!(windows.attempt_count(), 0);
    }

    #[tokio::test]
    async fn pending_and_activating_deadlines_execute_exact_destroy_once() {
        let (pending, pending_label) = pending_registry();
        let pending_windows = TestWindows::default();
        pending_windows.set_existing(true);
        execute_deadline_action(
            pending.claim_deadline(&pending_label),
            &pending_label,
            &pending_windows,
            |_| async { Ok(()) },
        )
        .await;
        assert_eq!(pending_windows.destroy_count(), 1);
        assert_eq!(pending_windows.attempt_count(), 1);
        assert_eq!(
            pending_windows.destroyed_labels(),
            vec![pending_label.clone()]
        );
        assert!(!pending.accepts_built_window(&pending_label));

        let (activating, activating_label) = pending_registry();
        activating
            .claim_activation(&activating_label)
            .expect("Activating");
        let activating_windows = TestWindows::default();
        activating_windows.set_existing(true);
        execute_deadline_action(
            activating.claim_deadline(&activating_label),
            &activating_label,
            &activating_windows,
            |_| async { Ok(()) },
        )
        .await;
        assert_eq!(activating_windows.destroy_count(), 1);
        assert_eq!(activating_windows.attempt_count(), 1);
        let (_, compensation) = activating
            .complete_activation(&activating_label, Ok(start(12)))
            .expect("late success");
        let activating_cancels = AtomicUsize::new(0);
        execute_token_cleanup(compensation, |claimed| {
            activating_cancels.fetch_add(1, Ordering::SeqCst);
            activating.complete_cancel_success(&activating_label, &claimed);
            std::future::ready(Ok(()))
        })
        .await;
        assert_eq!(activating_cancels.load(Ordering::SeqCst), 1);
        assert!(activating
            .reserve("capture-overlay-next-7".to_string(), selection())
            .is_ok());

        let (failing, failing_label) = pending_registry();
        failing
            .claim_activation(&failing_label)
            .expect("Activating failure");
        let failing_windows = TestWindows::default();
        failing_windows.set_existing(true);
        execute_deadline_action(
            failing.claim_deadline(&failing_label),
            &failing_label,
            &failing_windows,
            |_| async { panic!("late begin failure 不能 cancel lifecycle") },
        )
        .await;
        let error = failing
            .complete_activation(&failing_label, Err(CaptureError::SessionMissing))
            .expect_err("late failure");
        assert_eq!(error.code, "session_missing");
        assert_eq!(failing_windows.destroy_count(), 1);
        assert!(failing
            .reserve("capture-overlay-next-7".to_string(), selection())
            .is_ok());
    }

    #[tokio::test]
    async fn active_revealed_unrevealed_and_old_label_deadlines_execute_contract() {
        let (registry, label, token) = active_registry();
        let windows = TestWindows::default();
        windows.set_existing(true);
        let cancels = AtomicUsize::new(0);
        execute_deadline_action(
            registry.claim_deadline(&label),
            &label,
            &windows,
            |claimed| {
                assert_eq!(claimed, token);
                cancels.fetch_add(1, Ordering::SeqCst);
                registry.complete_cancel_success(&label, &claimed);
                std::future::ready(Ok(()))
            },
        )
        .await;
        assert_eq!(cancels.load(Ordering::SeqCst), 1);
        assert_eq!(windows.attempt_count(), 1);
        assert_eq!(windows.destroy_count(), 1);
        assert_eq!(windows.destroyed_labels(), vec![label]);

        let (revealed, revealed_label, _) = active_registry();
        assert!(matches!(
            revealed.claim_ready(&revealed_label),
            Ok(ReadyAction::ShowActive(_))
        ));
        let revealed_windows = TestWindows::default();
        revealed_windows.set_existing(true);
        let revealed_cancels = AtomicUsize::new(0);
        execute_deadline_action(
            revealed.claim_deadline(&revealed_label),
            &revealed_label,
            &revealed_windows,
            |_| {
                revealed_cancels.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(()))
            },
        )
        .await;
        assert_eq!(revealed_cancels.load(Ordering::SeqCst), 0);
        assert_eq!(revealed_windows.attempt_count(), 0);
        assert_eq!(revealed_windows.destroy_count(), 0);

        let (current, current_label) = pending_registry();
        let old_windows = TestWindows::default();
        old_windows.set_existing(true);
        execute_deadline_action(
            current.claim_deadline("longshot-controller-old"),
            "longshot-controller-old",
            &old_windows,
            |_| async { Ok(()) },
        )
        .await;
        assert_eq!(old_windows.attempt_count(), 0);
        assert!(current.accepts_built_window(&current_label));
    }

    #[tokio::test]
    async fn late_and_partial_build_outcomes_destroy_the_actual_exact_attempt_once() {
        let registry = LongshotControllerRegistry::new();
        let label = registry
            .reserve("capture-overlay-a-7".to_string(), selection())
            .expect("reserve");
        let windows = TestWindows::default();
        windows.add_alive("longshot-controller-decoy");
        execute_deadline_action(
            registry.claim_deadline(&label),
            &label,
            &windows,
            |_| async { Ok(()) },
        )
        .await;
        assert_eq!(windows.destroy_count(), 0, "deadline 时窗口尚未出现");
        assert_eq!(windows.attempt_count(), 1);
        windows.set_existing(true); // builder 迟到后才真正创建 attempted window
        windows.add_alive(&label);
        assert!(complete_build_attempt(&registry, &label, Ok(()), &windows).is_err());
        assert_eq!(windows.destroy_count(), 1);
        assert_eq!(windows.attempt_count(), 2);
        assert_eq!(windows.destroyed_labels(), vec![label.clone()]);
        assert!(windows.is_alive("longshot-controller-decoy"));

        let partial = LongshotControllerRegistry::new();
        let partial_label = partial
            .reserve("capture-overlay-b-7".to_string(), selection())
            .expect("reserve partial");
        let partial_windows = TestWindows::default();
        partial_windows.set_existing(true);
        assert!(complete_build_attempt(
            &partial,
            &partial_label,
            Err("partial build".to_string()),
            &partial_windows,
        )
        .is_err());
        assert_eq!(partial_windows.destroy_count(), 1);
        assert_eq!(partial_windows.attempt_count(), 1);
        assert_eq!(
            partial_windows.destroyed_labels(),
            vec![partial_label.clone()]
        );
        assert!(!partial.accepts_built_window(&partial_label));
        assert!(partial
            .reserve("capture-overlay-next-7".to_string(), selection())
            .is_ok());

        let absent = LongshotControllerRegistry::new();
        let absent_windows = TestWindows::default();
        let armed = AtomicBool::new(false);
        let error = open_with_ops(
            &absent,
            "capture-overlay-c-7",
            selection(),
            &absent_windows,
            |_, _| Ok(()),
            |_| armed.store(true, Ordering::SeqCst),
            |_| {
                assert!(armed.load(Ordering::SeqCst));
                Err("builder failed before window".to_string())
            },
        )
        .expect_err("builder error");
        assert_eq!(error.code, "longshot_controller_create_failed");
        assert_eq!(absent_windows.attempt_count(), 1);
        assert_eq!(absent_windows.destroy_count(), 0);
        assert!(absent
            .reserve("capture-overlay-next-7".to_string(), selection())
            .is_ok());
    }

    #[test]
    fn open_builder_failure_uses_real_validation_and_preserves_ordinary_resources() {
        let manager = CaptureManager::new();
        let gate = Arc::new(CaptureModeGate::new());
        let ownership = gate
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("Ordinary ownership");
        let frame = CapturedMonitorFrame {
            monitor_id: 7,
            x: 0,
            y: 0,
            logical_width: 100,
            logical_height: 80,
            pixel_width: 100,
            pixel_height: 80,
            scale_x: 1.0,
            scale_y: 1.0,
            rgba: Arc::from(vec![255; 100 * 80 * 4]),
        };
        let start = manager
            .begin(
                vec![frame],
                vec!["main".to_string()],
                vec!["pin-a".to_string()],
                false,
                StageTimings::default(),
                ownership,
            )
            .expect("ordinary begin");
        let caller = start.overlays[0].label.clone();
        let chosen = CaptureSelection {
            session_id: start.session_id.clone(),
            monitor_id: 7,
            x: 1.0,
            y: 2.0,
            width: 30.0,
            height: 40.0,
        };
        let registry = LongshotControllerRegistry::new();
        let windows = TestWindows::default();
        windows.set_existing(true); // 模拟 builder 已部分创建 exact attempted window
        let armed = AtomicBool::new(false);
        let result = open_with_ops(
            &registry,
            &caller,
            chosen,
            &windows,
            |label, selection| manager.validate_longshot_open(label, selection),
            |_| armed.store(true, Ordering::SeqCst),
            |_| {
                assert!(armed.load(Ordering::SeqCst), "deadline 必须先于 builder");
                Err("partial build".to_string())
            },
        );
        assert_eq!(
            result.expect_err("build 必须失败").code,
            "longshot_controller_create_failed"
        );
        assert_eq!(windows.destroy_count(), 1);
        assert_eq!(windows.attempt_count(), 1);
        assert!(manager.payload(&caller).is_ok());
        assert_eq!(
            gate.active_mode().expect("gate"),
            Some(CaptureMode::Ordinary)
        );
        let session = manager.finish(&start.session_id).expect("ordinary intact");
        assert_eq!(session.restore_labels, vec!["main"]);
        assert_eq!(session.lowered_pins, vec!["pin-a"]);
        session.finalize_mode().expect("release");
        assert!(registry
            .reserve("capture-overlay-next-7".to_string(), selection())
            .is_ok());
    }

    #[test]
    fn open_before_window_error_and_late_success_preserve_real_ordinary_resources() {
        let (manager, gate, session_id, caller, chosen) = ordinary_fixture();
        let registry = LongshotControllerRegistry::new();
        let windows = TestWindows::default();
        let result = open_with_ops(
            &registry,
            &caller,
            chosen,
            &windows,
            |label, selection| manager.validate_longshot_open(label, selection),
            |_| {},
            |_| Err("builder failed before window".to_string()),
        );
        assert_eq!(
            result.expect_err("build error").code,
            "longshot_controller_create_failed"
        );
        assert_eq!(windows.attempt_count(), 1);
        assert_eq!(windows.destroy_count(), 0);
        assert_and_finish_ordinary(&manager, &gate, &session_id, &caller);

        let (manager, gate, session_id, caller, chosen) = ordinary_fixture();
        let registry = LongshotControllerRegistry::new();
        let windows = TestWindows::default();
        let result = open_with_ops(
            &registry,
            &caller,
            chosen,
            &windows,
            |label, selection| manager.validate_longshot_open(label, selection),
            |label| {
                let action = registry.claim_deadline(label);
                assert!(matches!(action, DeadlineAction::Close));
                assert_eq!(
                    execute_deadline_window_action(action, label, &windows),
                    None
                );
            },
            |_| {
                windows.set_existing(true); // deadline 后 builder 才迟到成功
                Ok(())
            },
        );
        let error = result.expect_err("late build must be rejected");
        assert_eq!(error.code, "longshot_controller_missing");
        assert_eq!(windows.attempt_count(), 2);
        assert_eq!(windows.destroy_count(), 1);
        let destroyed = windows.destroyed_labels();
        assert_eq!(destroyed.len(), 1);
        assert!(destroyed[0].starts_with(CONTROLLER_PREFIX));
        assert_and_finish_ordinary(&manager, &gate, &session_id, &caller);
    }

    #[test]
    fn normal_open_with_ops_success_returns_launch_and_preserves_ordinary() {
        let (manager, gate, session_id, caller, chosen) = ordinary_fixture();
        let registry = LongshotControllerRegistry::new();
        let windows = TestWindows::default();
        let armed = AtomicBool::new(false);
        let built = AtomicBool::new(false);
        let launch = open_with_ops(
            &registry,
            &caller,
            chosen,
            &windows,
            |label, selection| manager.validate_longshot_open(label, selection),
            |_| armed.store(true, Ordering::SeqCst),
            |_| {
                assert!(armed.load(Ordering::SeqCst));
                built.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .expect("normal build");
        assert!(launch.label.starts_with(CONTROLLER_PREFIX));
        assert!(built.load(Ordering::SeqCst));
        assert!(registry.accepts_built_window(&launch.label));
        assert_eq!(windows.attempt_count(), 0);
        assert_and_finish_ordinary(&manager, &gate, &session_id, &caller);
    }

    #[tokio::test]
    async fn ready_success_shows_and_focuses_exact_window_once() {
        let (registry, label, _) = active_registry();
        let windows = TestWindows::default();
        let first = registry.claim_ready(&label).expect("first ready");
        execute_ready_action(&registry, first, &label, &windows, |_| async {
            panic!("ready success must not cancel")
        })
        .await
        .expect("show success");
        let repeated = registry.claim_ready(&label).expect("repeated ready");
        execute_ready_action(&registry, repeated, &label, &windows, |_| async {
            panic!("repeated ready must not cancel")
        })
        .await
        .expect("idempotent");
        assert_eq!(windows.show_count(), 1);
        assert_eq!(windows.focus_count(), 1);
        assert_eq!(windows.shown.lock().expect("shown").as_slice(), &[label]);
    }

    #[tokio::test]
    async fn ready_show_failures_apply_active_failed_and_cleanup_contracts() {
        let (active, active_label, token) = active_registry();
        let active_windows = TestWindows::default();
        active_windows.set_existing(true);
        active_windows.set_show_fails(true);
        let active_cancels = AtomicUsize::new(0);
        let active_result = execute_ready_action(
            &active,
            active.claim_ready(&active_label).expect("active ready"),
            &active_label,
            &active_windows,
            |claimed| {
                assert_eq!(claimed, token);
                active_cancels.fetch_add(1, Ordering::SeqCst);
                active.complete_cancel_success(&active_label, &claimed);
                std::future::ready(Ok(()))
            },
        )
        .await;
        assert_eq!(
            active_result.expect_err("show failure").code,
            "longshot_controller_show_failed"
        );
        assert_eq!(active_cancels.load(Ordering::SeqCst), 1);
        assert_eq!(active_windows.destroy_count(), 1);

        let (manager, gate, session_id, caller, _) = ordinary_fixture();
        let (failed, failed_label) = failed_registry();
        let failed_windows = TestWindows::default();
        failed_windows.set_existing(true);
        failed_windows.set_show_fails(true);
        let failed_cancels = AtomicUsize::new(0);
        let _ = execute_ready_action(
            &failed,
            failed.claim_ready(&failed_label).expect("failed ready"),
            &failed_label,
            &failed_windows,
            |_| {
                failed_cancels.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(()))
            },
        )
        .await;
        assert_eq!(failed_cancels.load(Ordering::SeqCst), 0);
        assert_eq!(failed_windows.destroy_count(), 1);
        assert_and_finish_ordinary(&manager, &gate, &session_id, &caller);

        let (cleanup, cleanup_label) = pending_registry();
        cleanup
            .claim_activation(&cleanup_label)
            .expect("Activating");
        assert!(cleanup.mark_cleanup_failed(&cleanup_label, None));
        let cleanup_windows = TestWindows::default();
        cleanup_windows.set_existing(true);
        cleanup_windows.set_show_fails(true);
        let cleanup_result = execute_ready_action(
            &cleanup,
            cleanup.claim_ready(&cleanup_label).expect("cleanup ready"),
            &cleanup_label,
            &cleanup_windows,
            |_| async { panic!("CleanupFailed show failure must not cancel") },
        )
        .await;
        assert_eq!(
            cleanup_result.expect_err("cleanup show failure").code,
            "longshot_controller_cleanup_failed"
        );
        assert_eq!(cleanup_windows.destroy_count(), 0);
        assert!(matches!(
            cleanup.claim_ready(&cleanup_label),
            Ok(ReadyAction::ShowCleanup)
        ));
    }

    #[tokio::test]
    async fn activation_spawn_blocking_panic_becomes_revealable_cleanup_failed() {
        let (registry, label) = pending_registry();
        registry.claim_activation(&label).expect("Activating");
        let failure = run_activation_worker(&registry, &label, || {
            panic!("injected activation worker panic")
        })
        .await
        .expect_err("JoinError must be structured");
        assert!(failure.cleanup_recorded);
        assert_eq!(failure.error.code, "longshot_controller_cleanup_failed");
        assert!(matches!(
            registry.claim_ready(&label),
            Ok(ReadyAction::ShowCleanup)
        ));
        assert_eq!(
            registry
                .reserve("capture-overlay-next-7".to_string(), selection())
                .expect_err("cleanup failure blocks reopen")
                .code,
            "longshot_controller_busy"
        );
    }

    #[test]
    fn append_claim_accepts_only_exact_revealed_active() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let claim = registry
            .claim_append(&label, &handle)
            .expect("exact append");
        assert_eq!(claim.token, token);
        assert_eq!(claim.old_snapshot, start(9).snapshot);
        let stale = LongshotControllerHandle {
            session_id: token.wire_parts().0.to_string(),
            generation: "8".to_string(),
        };
        assert_eq!(
            registry
                .claim_append(&label, &stale)
                .expect_err("valid stale token is superseded while appending")
                .code,
            "longshot_controller_superseded"
        );
        assert_eq!(
            registry
                .claim_append(&label, &handle)
                .expect_err("second append is busy")
                .code,
            "longshot_controller_busy"
        );

        let (unrevealed, unrevealed_label, unrevealed_token) = active_registry();
        assert_eq!(
            unrevealed
                .claim_append(
                    &unrevealed_label,
                    &LongshotControllerHandle::from_token(&unrevealed_token),
                )
                .expect_err("unrevealed")
                .code,
            "longshot_controller_missing"
        );
    }

    #[test]
    fn append_claim_rejects_stale_malformed_and_unrevealed_before_actions() {
        let (registry, label, token) = revealed_active_registry();
        let windows = TestWindows::default();
        for generation in ["", "01", "18446744073709551616", "8"] {
            let error = registry
                .claim_append(
                    &label,
                    &LongshotControllerHandle {
                        session_id: token.wire_parts().0.to_string(),
                        generation: generation.to_string(),
                    },
                )
                .expect_err("invalid or stale");
            assert_eq!(error.code, "longshot_controller_superseded");
        }
        assert_eq!(
            registry
                .claim_append(
                    "capture-overlay-not-controller",
                    &LongshotControllerHandle::from_token(&token),
                )
                .expect_err("bad caller")
                .code,
            "longshot_controller_missing"
        );
        let exact = LongshotControllerHandle::from_token(&token);
        registry.claim_append(&label, &exact).expect("exact claim");
        let stale = LongshotControllerHandle {
            session_id: token.wire_parts().0.to_string(),
            generation: "8".to_string(),
        };
        assert_eq!(
            registry
                .claim_append(&label, &stale)
                .expect_err("stale token while Appending")
                .code,
            "longshot_controller_superseded"
        );
        assert!(registry.owns_append(&label, &token));
        assert_eq!(
            registry
                .claim_append(&label, &exact)
                .expect_err("exact duplicate remains busy")
                .code,
            "longshot_controller_busy"
        );
        assert_eq!(windows.hide_count(), 0);
        assert_eq!(windows.show_count(), 0);
        assert_eq!(windows.attempt_count(), 0);
    }

    #[test]
    fn append_completion_requires_exact_owner_and_visible_commit() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        registry.claim_append(&label, &handle).expect("claim");
        let next = LongshotSnapshot {
            frame_count: 2,
            width: 30,
            frame_height: 40,
            total_height: 70,
        };
        assert_eq!(
            registry
                .complete_append_visible("longshot-controller-old", &token, next)
                .expect_err("old label")
                .code,
            "longshot_controller_superseded"
        );
        let stale = LongshotSessionToken::from_wire_parts("longshot".to_string(), 8);
        assert_eq!(
            registry
                .complete_append_visible(&label, &stale, next)
                .expect_err("stale token")
                .code,
            "longshot_controller_superseded"
        );
        let dto = registry
            .complete_append_visible(&label, &token, next)
            .expect("exact visible commit");
        assert_eq!(dto.frame_count, 2);
        assert_eq!(dto.total_height, 70);
        assert_eq!(
            registry
                .complete_append_visible(&label, &token, start(9).snapshot)
                .expect_err("commit once")
                .code,
            "longshot_controller_superseded"
        );
    }

    #[tokio::test]
    async fn append_executor_orders_hide_settle_worker_show_focus_commit() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let windows = TestWindows::default();
        windows.add_alive(&label);
        let next = LongshotSnapshot {
            frame_count: 2,
            width: 30,
            frame_height: 40,
            total_height: 72,
        };
        let result = execute_append_with_ops(
            &registry,
            &label,
            &handle,
            &windows,
            || async { windows.record(format!("settle:{}", crate::capture::HIDE_SETTLE_MS)) },
            |_| async {
                windows.record("append");
                Ok(next)
            },
            |_| async { panic!("success must not cancel") },
            |boundary| {
                if boundary == AppendBoundary::BeforeCommit {
                    windows.record("commit-attempt");
                }
            },
        )
        .await
        .expect("append success");
        assert_eq!(result.frame_count, 2);
        assert_eq!(result.total_height, 72);
        assert_eq!(
            windows.trace(),
            vec![
                "hide",
                "settle:140",
                "append",
                "show",
                "focus",
                "commit-attempt"
            ]
        );
        assert!(matches!(
            &*registry.slot.lock().expect("slot after append"),
            Slot::Active {
                label: current,
                token: current_token,
                snapshot,
                revealed: true,
            } if current == &label && current_token == &token && snapshot == &next
        ));
        registry
            .claim_append(&label, &handle)
            .expect("actual visible commit permits the next exact append");
    }

    #[tokio::test]
    async fn append_hide_and_business_failures_restore_visible_old_snapshot() {
        let (hidden, hidden_label, hidden_token) = revealed_active_registry();
        let hidden_handle = LongshotControllerHandle::from_token(&hidden_token);
        let hidden_windows = TestWindows::default();
        hidden_windows.add_alive(&hidden_label);
        hidden_windows.set_hide_fails(true);
        let worker_calls = AtomicUsize::new(0);
        let error = execute_append_with_ops(
            &hidden,
            &hidden_label,
            &hidden_handle,
            &hidden_windows,
            || async {},
            |_| {
                worker_calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(start(10).snapshot))
            },
            |_| async { Ok(()) },
            |_| {},
        )
        .await
        .expect_err("hide failure");
        assert_eq!(error.code, "longshot_controller_hide_failed");
        assert_eq!(worker_calls.load(Ordering::SeqCst), 0);
        assert_eq!(hidden_windows.show_count(), 1);
        assert_eq!(hidden_windows.focus_count(), 1);
        hidden
            .claim_append(&hidden_label, &hidden_handle)
            .expect("hide failure is retryable");

        let (business, business_label, business_token) = revealed_active_registry();
        let business_handle = LongshotControllerHandle::from_token(&business_token);
        let business_windows = TestWindows::default();
        business_windows.add_alive(&business_label);
        let error = execute_append_with_ops(
            &business,
            &business_label,
            &business_handle,
            &business_windows,
            || async {},
            |_| async {
                Err(LongshotIpcError::from(
                    CaptureError::LongshotEstimateLowTexture,
                ))
            },
            |_| async { Ok(()) },
            |_| {},
        )
        .await
        .expect_err("business failure");
        assert_eq!(error.code, "longshot_estimate_low_texture");
        assert_eq!(business_windows.show_count(), 1);
        business
            .claim_append(&business_label, &business_handle)
            .expect("business failure is retryable");

        let (raced, raced_label, raced_token) = revealed_active_registry();
        let raced_handle = LongshotControllerHandle::from_token(&raced_token);
        let race_windows = DestroyOnShowWindows {
            registry: &raced,
            label: &raced_label,
            inner: TestWindows::default(),
        };
        race_windows.inner.add_alive(&raced_label);
        race_windows.inner.set_hide_fails(true);
        let error = execute_append_with_ops(
            &raced,
            &raced_label,
            &raced_handle,
            &race_windows,
            || async {},
            |_| async { panic!("hide failure must not append") },
            |_| async { Ok(()) },
            |_| {},
        )
        .await
        .expect_err("Destroyed during hide recovery show wins");
        assert_eq!(error.code, "longshot_controller_superseded");
        assert_eq!(race_windows.inner.destroy_count(), 1);
    }

    #[tokio::test]
    async fn append_spawn_blocking_panic_is_internal_and_retryable() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let windows = TestWindows::default();
        windows.add_alive(&label);
        let error = execute_append_with_ops(
            &registry,
            &label,
            &handle,
            &windows,
            || async {},
            |_| {
                run_append_worker(|| -> Result<LongshotSnapshot, CaptureError> {
                    panic!("injected append panic")
                })
            },
            |_| async { Ok(()) },
            |_| {},
        )
        .await
        .expect_err("panic is structured");
        assert_eq!(error.code, "longshot_controller_internal");
        assert_eq!(windows.show_count(), 1);
        registry
            .claim_append(&label, &handle)
            .expect("join failure is retryable");
    }

    #[tokio::test]
    async fn append_cancel_wins_at_every_boundary_without_resurrection() {
        for target in [
            AppendBoundary::AfterClaim,
            AppendBoundary::AfterHide,
            AppendBoundary::AfterSettle,
            AppendBoundary::AfterWorker,
            AppendBoundary::AfterShow,
            AppendBoundary::BeforeCommit,
        ] {
            let (registry, label, token) = revealed_active_registry();
            let handle = LongshotControllerHandle::from_token(&token);
            let windows = TestWindows::default();
            windows.add_alive(&label);
            let cancels = AtomicUsize::new(0);
            let result = execute_append_with_ops(
                &registry,
                &label,
                &handle,
                &windows,
                || async {},
                |_| async { Ok(start(10).snapshot) },
                |_| async { panic!("boundary winner owns cleanup") },
                |boundary| {
                    if boundary == target {
                        let action = registry
                            .claim_cancel(&label, Some(&handle))
                            .expect("cancel winner");
                        let CancelAction::Terminate(claimed) = action else {
                            panic!("active append cancellation must terminate")
                        };
                        cancels.fetch_add(1, Ordering::SeqCst);
                        registry.complete_cancel_success(&label, &claimed);
                    }
                },
            )
            .await;
            assert_eq!(
                result.expect_err("cancel supersedes append").code,
                "longshot_controller_superseded"
            );
            assert_eq!(cancels.load(Ordering::SeqCst), 1);
            assert!(registry
                .reserve("capture-overlay-next-7".to_string(), selection())
                .is_ok());
        }
    }

    #[tokio::test]
    async fn append_destroyed_wins_at_every_boundary_without_second_cleanup() {
        for target in [
            AppendBoundary::AfterClaim,
            AppendBoundary::AfterHide,
            AppendBoundary::AfterSettle,
            AppendBoundary::AfterWorker,
            AppendBoundary::AfterShow,
            AppendBoundary::BeforeCommit,
        ] {
            let (registry, label, token) = revealed_active_registry();
            let handle = LongshotControllerHandle::from_token(&token);
            let windows = TestWindows::default();
            windows.add_alive(&label);
            let cancels = AtomicUsize::new(0);
            let result = execute_append_with_ops(
                &registry,
                &label,
                &handle,
                &windows,
                || async {},
                |_| async { Ok(start(10).snapshot) },
                |_| async { panic!("Destroyed winner owns cleanup") },
                |boundary| {
                    if boundary == target {
                        let claimed = registry.claim_destroyed(&label).expect("Destroyed winner");
                        cancels.fetch_add(1, Ordering::SeqCst);
                        registry.complete_cancel_success(&label, &claimed);
                    }
                },
            )
            .await;
            assert_eq!(
                result.expect_err("Destroyed supersedes append").code,
                "longshot_controller_superseded"
            );
            assert_eq!(cancels.load(Ordering::SeqCst), 1);
            assert!(registry
                .reserve("capture-overlay-next-7".to_string(), selection())
                .is_ok());
        }
    }

    #[test]
    fn appending_cancel_failure_reveals_before_retry_or_enters_cleanup_failed() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        registry.claim_append(&label, &handle).expect("append");
        assert!(matches!(
            registry.claim_cancel(&label, Some(&handle)),
            Ok(CancelAction::Terminate(_))
        ));
        let windows = TestWindows::default();
        windows.add_alive(&label);
        let result = recover_cancel_failure_with_ops(
            &registry,
            &label,
            &token,
            &windows,
            CaptureError::LongshotSessionMissing,
            || {
                windows.record("probe");
                true
            },
        );
        assert_eq!(
            result.expect_err("primary remains").code,
            "longshot_session_missing"
        );
        assert_eq!(windows.trace(), vec!["probe", "show", "focus"]);
        registry
            .claim_append(&label, &handle)
            .expect("visible Active retry");

        for mode in [0, 1, 2] {
            let (failed, failed_label, failed_token) = revealed_active_registry();
            let failed_handle = LongshotControllerHandle::from_token(&failed_token);
            failed
                .claim_append(&failed_label, &failed_handle)
                .expect("append");
            failed
                .claim_cancel(&failed_label, Some(&failed_handle))
                .expect("cancel");
            let failed_windows = TestWindows::default();
            if mode != 0 {
                failed_windows.add_alive(&failed_label);
            }
            if mode == 2 {
                failed_windows.set_show_fails(true);
            }
            let error = recover_cancel_failure_with_ops(
                &failed,
                &failed_label,
                &failed_token,
                &failed_windows,
                CaptureError::LongshotSessionMissing,
                || mode != 1,
            )
            .expect_err("unsafe retry must fail cleanup");
            assert_eq!(error.code, "longshot_controller_cleanup_failed");
            assert_eq!(
                failed
                    .reserve("capture-overlay-next-7".to_string(), selection())
                    .expect_err("CleanupFailed blocks open")
                    .code,
                "longshot_controller_busy"
            );
        }

        let (before_show, before_label, before_token) = revealed_active_registry();
        let before_handle = LongshotControllerHandle::from_token(&before_token);
        before_show
            .claim_append(&before_label, &before_handle)
            .expect("append");
        before_show
            .claim_cancel(&before_label, Some(&before_handle))
            .expect("cancel");
        assert_eq!(before_show.claim_destroyed(&before_label), None);
        let before_windows = TestWindows::default();
        let error = recover_cancel_failure_with_ops(
            &before_show,
            &before_label,
            &before_token,
            &before_windows,
            CaptureError::LongshotSessionMissing,
            || panic!("Forbidden retry visibility must not probe lifecycle"),
        )
        .expect_err("Destroyed before recovery becomes CleanupFailed");
        assert_eq!(error.code, "longshot_controller_cleanup_failed");
        assert!(matches!(
            before_show.claim_ready(&before_label),
            Ok(ReadyAction::ShowCleanup)
        ));
        assert_eq!(
            before_show
                .reserve("capture-overlay-next-7".to_string(), selection())
                .expect_err("Destroyed during retry reveal is CleanupFailed")
                .code,
            "longshot_controller_busy"
        );

        let (after_show, after_label, after_token) = revealed_active_registry();
        let after_handle = LongshotControllerHandle::from_token(&after_token);
        after_show
            .claim_append(&after_label, &after_handle)
            .expect("append");
        after_show
            .claim_cancel(&after_label, Some(&after_handle))
            .expect("cancel");
        let race_windows = DestroyOnShowWindows {
            registry: &after_show,
            label: &after_label,
            inner: TestWindows::default(),
        };
        race_windows.inner.add_alive(&after_label);
        let error = recover_cancel_failure_with_ops(
            &after_show,
            &after_label,
            &after_token,
            &race_windows,
            CaptureError::LongshotSessionMissing,
            || true,
        )
        .expect_err("Destroyed after show prevents Active commit");
        assert_eq!(error.code, "longshot_controller_cleanup_failed");
        assert_eq!(race_windows.inner.destroy_count(), 1);
        assert_eq!(
            after_show
                .reserve("capture-overlay-next-7".to_string(), selection())
                .expect_err("race remains CleanupFailed")
                .code,
            "longshot_controller_busy"
        );
    }

    #[test]
    fn appending_deadline_and_old_aba_events_are_noops() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        registry.claim_append(&label, &handle).expect("append");
        assert!(matches!(
            registry.claim_deadline(&label),
            DeadlineAction::None
        ));
        assert_eq!(registry.claim_destroyed("longshot-controller-old"), None);
        let stale = LongshotSessionToken::from_wire_parts("longshot".to_string(), 8);
        assert_eq!(
            registry
                .complete_append_visible(&label, &stale, start(11).snapshot)
                .expect_err("stale worker")
                .code,
            "longshot_controller_superseded"
        );
        let dto = registry
            .complete_append_visible(&label, &token, start(10).snapshot)
            .expect("current commit");
        assert_eq!(dto.frame_count, 1);
    }

    #[tokio::test]
    async fn append_show_failure_prioritizes_visibility_cleanup_for_all_worker_results() {
        for worker_kind in 0..3 {
            let (registry, label, token) = revealed_active_registry();
            let handle = LongshotControllerHandle::from_token(&token);
            let windows = TestWindows::default();
            windows.add_alive(&label);
            windows.set_show_fails(true);
            let cancels = AtomicUsize::new(0);
            let result = execute_append_with_ops(
                &registry,
                &label,
                &handle,
                &windows,
                || async {},
                |_| async move {
                    match worker_kind {
                        0 => Ok(start(10).snapshot),
                        1 => Err(LongshotIpcError::from(
                            CaptureError::LongshotEstimateLowTexture,
                        )),
                        _ => Err(LongshotIpcError::internal("join failure")),
                    }
                },
                |claimed| {
                    cancels.fetch_add(1, Ordering::SeqCst);
                    registry.complete_cancel_success(&label, &claimed);
                    std::future::ready(Ok(()))
                },
                |_| {},
            )
            .await;
            assert_eq!(
                result.expect_err("show outranks worker").code,
                "longshot_controller_show_failed"
            );
            assert_eq!(cancels.load(Ordering::SeqCst), 1);
            assert_eq!(windows.destroy_count(), 1);
            assert!(registry
                .reserve("capture-overlay-next-7".to_string(), selection())
                .is_ok());
        }

        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let windows = TestWindows::default();
        windows.add_alive(&label);
        windows.set_show_fails(true);
        let result = execute_append_with_ops(
            &registry,
            &label,
            &handle,
            &windows,
            || async {},
            |_| async { Ok(start(10).snapshot) },
            |_| {
                std::future::ready(Err(LongshotIpcError::cleanup_failed(
                    "injected direct cleanup failure",
                )))
            },
            |_| {},
        )
        .await;
        assert_eq!(
            result.expect_err("cleanup outranks show").code,
            "longshot_controller_cleanup_failed"
        );
        assert!(matches!(
            registry.claim_ready(&label),
            Ok(ReadyAction::ShowCleanup)
        ));
    }

    #[tokio::test]
    async fn finish_copy_success_orders_effects_and_commits_once() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let windows = TestWindows::default();
        windows.add_alive(&label);
        let trace = Arc::new(Mutex::new(Vec::<String>::new()));
        let finish_trace = Arc::clone(&trace);
        let copy_trace = Arc::clone(&trace);
        let png = vec![1, 2, 3, 4, 5];

        let result = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Copy,
            &windows,
            FinishOperations::new(
                move |claimed| {
                    assert_eq!(claimed, token);
                    finish_trace.lock().expect("trace").push("finish".into());
                    std::future::ready(Ok(lifecycle_artifact(png)))
                },
                |_| false,
                move |requested_action, artifact| {
                    assert_eq!(requested_action, LongshotOutputAction::Copy);
                    assert_eq!(artifact.png.as_slice(), &[1, 2, 3, 4, 5]);
                    assert_eq!(artifact.origin, test_origin());
                    copy_trace.lock().expect("trace").push("copy".into());
                    std::future::ready(Ok(OutputValue::None))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect("finish copy");

        assert_eq!(result.action, LongshotOutputAction::Copy);
        assert_eq!(result.path, None);
        assert_eq!(result.pin_label, None);
        assert_eq!(*trace.lock().expect("trace"), vec!["finish", "copy"]);
        assert_eq!(windows.destroy_count(), 1);
        assert!(registry
            .reserve("capture-overlay-next-7".to_string(), selection())
            .is_ok());
    }

    #[tokio::test]
    async fn finish_copy_failure_retries_same_arc_without_reencoding() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let windows = TestWindows::default();
        windows.add_alive(&label);
        let finishes = AtomicUsize::new(0);
        let first_seen = Arc::new(Mutex::new(None::<Arc<LongshotOutputArtifact>>));
        let first_slot = Arc::clone(&first_seen);

        let first = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Copy,
            &windows,
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(vec![9, 8, 7])))
                },
                |_| false,
                move |_, artifact| {
                    *first_slot.lock().expect("first artifact") = Some(Arc::clone(&artifact));
                    std::future::ready(Err(finish::OutputWorkerError::Business(
                        "injected copy failure".to_string(),
                    )))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect_err("first copy fails");
        assert_eq!(first.code, "longshot_controller_copy_failed");

        let retained = first_seen
            .lock()
            .expect("first artifact")
            .as_ref()
            .expect("recorded artifact")
            .clone();
        match &*registry.slot.lock().expect("slot") {
            Slot::OutputPending { artifact, .. } => {
                assert!(Arc::ptr_eq(artifact, &retained));
                assert_eq!(artifact.origin, retained.origin);
            }
            other => panic!("expected OutputPending, got {other:?}"),
        }

        let retry_seen = Arc::new(Mutex::new(None::<Arc<LongshotOutputArtifact>>));
        let retry_slot = Arc::clone(&retry_seen);
        execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Copy,
            &windows,
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(Vec::new())))
                },
                |_| false,
                move |_, artifact| {
                    *retry_slot.lock().expect("retry artifact") = Some(Arc::clone(&artifact));
                    std::future::ready(Ok(OutputValue::None))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect("retry copy");

        assert_eq!(finishes.load(Ordering::SeqCst), 1);
        let retried = retry_seen
            .lock()
            .expect("retry artifact")
            .as_ref()
            .expect("retry recorded")
            .clone();
        assert!(Arc::ptr_eq(&retained, &retried));
        assert!(Arc::ptr_eq(&retained.png, &retried.png));
        assert_eq!(retained.origin, retried.origin);
        assert_eq!(windows.destroy_count(), 1);
    }

    #[tokio::test]
    async fn finish_save_success_returns_path_without_copying() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let windows = TestWindows::default();
        windows.add_alive(&label);
        let finishes = AtomicUsize::new(0);
        let saves = AtomicUsize::new(0);

        let result = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Save,
            &windows,
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(vec![8, 6, 7, 5])))
                },
                |_| false,
                |action, artifact| {
                    assert_eq!(action, LongshotOutputAction::Save);
                    assert_eq!(artifact.png.as_slice(), &[8, 6, 7, 5]);
                    saves.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(OutputValue::SavePath("/tmp/截图-完成.png".to_string())))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect("finish save");

        assert_eq!(result.action, LongshotOutputAction::Save);
        assert_eq!(result.path.as_deref(), Some("/tmp/截图-完成.png"));
        assert_eq!(result.pin_label, None);
        assert_eq!(finishes.load(Ordering::SeqCst), 1);
        assert_eq!(saves.load(Ordering::SeqCst), 1);
        assert_eq!(windows.destroy_count(), 1);
        assert!(registry
            .reserve("capture-overlay-next-save".to_string(), selection())
            .is_ok());
    }

    #[tokio::test]
    async fn finish_pin_success_returns_label_and_commits_once() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let windows = TestWindows::default();
        windows.add_alive(&label);
        let finishes = AtomicUsize::new(0);
        let pins = AtomicUsize::new(0);

        let result = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Pin,
            &windows,
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(vec![3, 5, 8, 9])))
                },
                |_| false,
                |action, artifact| {
                    assert_eq!(action, LongshotOutputAction::Pin);
                    assert_eq!(artifact.png.as_slice(), &[3, 5, 8, 9]);
                    assert_eq!(artifact.origin, test_origin());
                    pins.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(OutputValue::PinLabel("pin-image-longshot".to_string())))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect("finish pin");

        assert_eq!(result.action, LongshotOutputAction::Pin);
        assert_eq!(result.path, None);
        assert_eq!(result.pin_label.as_deref(), Some("pin-image-longshot"));
        assert_eq!(finishes.load(Ordering::SeqCst), 1);
        assert_eq!(pins.load(Ordering::SeqCst), 1);
        assert_eq!(windows.destroy_count(), 1);
        assert!(registry
            .reserve("capture-overlay-next-pin".to_string(), selection())
            .is_ok());
    }

    #[tokio::test]
    async fn finish_pin_not_created_retries_the_same_artifact_without_reencoding() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let finishes = AtomicUsize::new(0);
        let retained = Arc::new(Mutex::new(None::<Arc<LongshotOutputArtifact>>));
        let first_seen = Arc::clone(&retained);

        let error = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Pin,
            &TestWindows::default(),
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(vec![2, 3, 5, 7])))
                },
                |_| false,
                move |action, artifact| {
                    assert_eq!(action, LongshotOutputAction::Pin);
                    *first_seen.lock().expect("first") = Some(Arc::clone(&artifact));
                    std::future::ready(Err(finish::OutputWorkerError::Business(
                        "Pin manager unavailable".to_string(),
                    )))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect_err("confirmed not-created Pin fails");
        assert_eq!(error.code, "longshot_controller_pin_failed");

        let original = retained
            .lock()
            .expect("first")
            .as_ref()
            .expect("recorded")
            .clone();
        assert!(matches!(
            &*registry.slot.lock().expect("slot"),
            Slot::OutputPending {
                artifact,
                retry_policy: RetryPolicy::Any,
                ..
            } if Arc::ptr_eq(artifact, &original)
        ));

        let result = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Pin,
            &TestWindows::default(),
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(Vec::new())))
                },
                |_| false,
                move |action, artifact| {
                    assert_eq!(action, LongshotOutputAction::Pin);
                    assert!(Arc::ptr_eq(&artifact, &original));
                    assert!(Arc::ptr_eq(&artifact.png, &original.png));
                    assert_eq!(artifact.origin, original.origin);
                    std::future::ready(Ok(OutputValue::PinLabel("pin-image-retry".to_string())))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect("Pin retry");
        assert_eq!(result.pin_label.as_deref(), Some("pin-image-retry"));
        assert_eq!(result.path, None);
        assert_eq!(finishes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn finish_save_failure_retries_copy_with_same_arc_without_reencoding() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let finishes = AtomicUsize::new(0);
        let retained = Arc::new(Mutex::new(None::<Arc<LongshotOutputArtifact>>));
        let first_artifact = Arc::clone(&retained);

        let error = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Save,
            &TestWindows::default(),
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(vec![2, 7, 1, 8])))
                },
                |_| false,
                move |action, artifact| {
                    assert_eq!(action, LongshotOutputAction::Save);
                    *first_artifact.lock().expect("artifact") = Some(Arc::clone(&artifact));
                    std::future::ready(Err(finish::OutputWorkerError::Business(
                        "disk full".to_string(),
                    )))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect_err("save failure");
        assert_eq!(error.code, "longshot_controller_save_failed");
        let original = retained
            .lock()
            .expect("artifact")
            .as_ref()
            .expect("recorded")
            .clone();
        assert!(matches!(
            &*registry.slot.lock().expect("slot"),
            Slot::OutputPending { artifact, retry_policy: RetryPolicy::Any, .. }
                if Arc::ptr_eq(artifact, &original)
        ));

        let retry_artifact = Arc::new(Mutex::new(None::<Arc<LongshotOutputArtifact>>));
        let retry_seen = Arc::clone(&retry_artifact);
        let result = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Copy,
            &TestWindows::default(),
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(Vec::new())))
                },
                |_| false,
                move |action, artifact| {
                    assert_eq!(action, LongshotOutputAction::Copy);
                    *retry_seen.lock().expect("retry") = Some(Arc::clone(&artifact));
                    std::future::ready(Ok(OutputValue::None))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect("retry copy");
        assert_eq!(result.action, LongshotOutputAction::Copy);
        assert_eq!(result.path, None);
        assert_eq!(result.pin_label, None);
        assert_eq!(finishes.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(
            &original,
            retry_artifact
                .lock()
                .expect("retry")
                .as_ref()
                .expect("recorded retry")
        ));
        let retried = retry_artifact
            .lock()
            .expect("retry")
            .as_ref()
            .expect("recorded retry")
            .clone();
        assert!(Arc::ptr_eq(&original.png, &retried.png));
        assert_eq!(original.origin, retried.origin);
    }

    #[tokio::test]
    async fn finish_copy_failure_can_retry_save_with_same_arc() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let finishes = AtomicUsize::new(0);
        let first = Arc::new(Mutex::new(None::<Arc<LongshotOutputArtifact>>));
        let first_seen = Arc::clone(&first);

        execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Copy,
            &TestWindows::default(),
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(vec![3, 1, 4, 1])))
                },
                |_| false,
                move |_, artifact| {
                    *first_seen.lock().expect("first") = Some(Arc::clone(&artifact));
                    std::future::ready(Err(finish::OutputWorkerError::Business(
                        "clipboard busy".to_string(),
                    )))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect_err("copy failure");
        let original = first
            .lock()
            .expect("first")
            .as_ref()
            .expect("recorded")
            .clone();

        let result = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Save,
            &TestWindows::default(),
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(Vec::new())))
                },
                |_| false,
                move |action, artifact| {
                    assert_eq!(action, LongshotOutputAction::Save);
                    assert!(Arc::ptr_eq(&artifact, &original));
                    assert!(Arc::ptr_eq(&artifact.png, &original.png));
                    assert_eq!(artifact.origin, original.origin);
                    std::future::ready(Ok(OutputValue::SavePath("/tmp/recovered.png".to_string())))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect("retry save");
        assert_eq!(result.action, LongshotOutputAction::Save);
        assert_eq!(result.path.as_deref(), Some("/tmp/recovered.png"));
        assert_eq!(result.pin_label, None);
        assert_eq!(finishes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn finish_business_error_restores_revealed_snapshot_for_retry() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let old_snapshot = start(9).snapshot;
        let result = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Copy,
            &TestWindows::default(),
            FinishOperations::new(
                |_| {
                    std::future::ready(Err(FinishWorkerError::Business(
                        CaptureError::LongshotEstimateLowTexture,
                    )))
                },
                |_| true,
                |_, _| std::future::ready(Ok(OutputValue::None)),
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect_err("domain error");
        assert_eq!(result.code, "longshot_estimate_low_texture");
        match &*registry.slot.lock().expect("slot") {
            Slot::Active {
                snapshot,
                revealed: true,
                ..
            } => assert_eq!(*snapshot, old_snapshot),
            other => panic!("expected revealed Active, got {other:?}"),
        }
        assert!(registry.claim_append(&label, &handle).is_ok());
    }

    #[tokio::test]
    async fn finish_destroyed_during_encoding_compensates_exact_active_once() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let cancels = AtomicUsize::new(0);
        let result = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Copy,
            &TestWindows::default(),
            FinishOperations::new(
                |_| {
                    std::future::ready(Err(FinishWorkerError::Business(
                        CaptureError::LongshotEstimateLowTexture,
                    )))
                },
                |_| true,
                |_, _| std::future::ready(Ok(OutputValue::None)),
                |claimed| {
                    assert_eq!(claimed, token);
                    cancels.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(()))
                },
                |boundary| {
                    if boundary == FinishBoundary::AfterClaim {
                        assert_eq!(registry.claim_destroyed(&label), None);
                    }
                },
            ),
        )
        .await
        .expect_err("primary preserved");
        assert_eq!(result.code, "longshot_estimate_low_texture");
        assert_eq!(cancels.load(Ordering::SeqCst), 1);
        assert!(registry
            .reserve("capture-overlay-next-7".to_string(), selection())
            .is_ok());
    }

    #[tokio::test]
    async fn finish_destroyed_copy_failure_drops_artifact_without_cancel() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let cancels = AtomicUsize::new(0);
        let result = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Copy,
            &TestWindows::default(),
            FinishOperations::new(
                |_| std::future::ready(Ok(lifecycle_artifact(vec![4, 3, 2, 1]))),
                |_| false,
                |_, _| {
                    std::future::ready(Err(finish::OutputWorkerError::Business(
                        "clipboard unavailable".to_string(),
                    )))
                },
                |_| {
                    cancels.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(()))
                },
                |boundary| {
                    if boundary == FinishBoundary::BeforeOutput {
                        assert_eq!(registry.claim_destroyed(&label), None);
                    }
                },
            ),
        )
        .await
        .expect_err("copy fails");
        assert_eq!(result.code, "longshot_controller_copy_failed");
        assert_eq!(cancels.load(Ordering::SeqCst), 0);
        assert!(registry
            .reserve("capture-overlay-next-7".to_string(), selection())
            .is_ok());
    }

    #[tokio::test]
    async fn finish_join_failure_is_conservative_cleanup_failed() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let result = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Copy,
            &TestWindows::default(),
            FinishOperations::new(
                |_| std::future::ready(Err(FinishWorkerError::Join("panic".into()))),
                |_| true,
                |_, _| std::future::ready(Ok(OutputValue::None)),
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect_err("join failure");
        assert_eq!(result.code, "longshot_controller_cleanup_failed");
        assert!(matches!(
            registry.claim_ready(&label),
            Ok(ReadyAction::ShowCleanup)
        ));
    }

    #[test]
    fn finish_claim_rejects_invalid_or_busy_states_before_side_effects() {
        let (registry, label, token) = active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        assert_eq!(
            registry
                .claim_finish(&label, &handle, LongshotOutputAction::Copy)
                .expect_err("unrevealed")
                .code,
            "longshot_controller_missing"
        );
        assert!(matches!(
            registry.claim_ready(&label),
            Ok(ReadyAction::ShowActive(_))
        ));
        let stale = LongshotControllerHandle {
            session_id: "longshot".into(),
            generation: "8".into(),
        };
        assert_eq!(
            registry
                .claim_finish(&label, &stale, LongshotOutputAction::Copy)
                .expect_err("stale")
                .code,
            "longshot_controller_superseded"
        );
        assert!(matches!(
            registry.claim_finish(&label, &handle, LongshotOutputAction::Copy),
            Ok(FinishClaim::Encoding(_))
        ));
        assert_eq!(
            registry
                .claim_finish(&label, &handle, LongshotOutputAction::Copy)
                .expect_err("duplicate")
                .code,
            "longshot_controller_busy"
        );
        assert_eq!(
            registry
                .claim_append(&label, &handle)
                .expect_err("append while finishing")
                .code,
            "longshot_controller_busy"
        );
        assert_eq!(
            registry
                .claim_finish(
                    "capture-overlay-not-controller",
                    &handle,
                    LongshotOutputAction::Copy,
                )
                .expect_err("wrong caller")
                .code,
            "longshot_controller_missing"
        );
    }

    #[test]
    fn finish_output_transitions_require_exact_action_and_arc() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        assert!(matches!(
            registry.claim_finish(&label, &handle, LongshotOutputAction::Save),
            Ok(FinishClaim::Encoding(_))
        ));
        let artifact = output_artifact(vec![5, 8, 9, 7]);
        assert_eq!(
            registry
                .publish_outputting(
                    &label,
                    &token,
                    LongshotOutputAction::Copy,
                    Arc::clone(&artifact),
                )
                .expect_err("wrong publish action")
                .code,
            "longshot_controller_cleanup_failed"
        );
        registry
            .publish_outputting(
                &label,
                &token,
                LongshotOutputAction::Save,
                Arc::clone(&artifact),
            )
            .expect("exact publish");

        let fake_same_bytes = output_artifact(artifact.png.as_ref().clone());
        assert!(!registry.complete_output_success(
            &label,
            &token,
            LongshotOutputAction::Save,
            &fake_same_bytes,
        ));
        let fake_outer = Arc::new(LongshotOutputArtifact {
            png: Arc::clone(&artifact.png),
            origin: artifact.origin,
        });
        assert!(!registry.complete_output_success(
            &label,
            &token,
            LongshotOutputAction::Save,
            &fake_outer,
        ));
        assert!(!registry.complete_output_success(
            &label,
            &token,
            LongshotOutputAction::Pin,
            &artifact,
        ));

        assert_eq!(
            registry.complete_output_failure(
                &label,
                &token,
                LongshotOutputAction::Copy,
                &artifact,
                false,
            ),
            OutputFailureAction::OwnershipLost
        );
        assert_eq!(
            registry.complete_output_failure(
                &label,
                &token,
                LongshotOutputAction::Save,
                &fake_same_bytes,
                false,
            ),
            OutputFailureAction::OwnershipLost
        );
        assert_eq!(
            registry.complete_output_failure(
                &label,
                &token,
                LongshotOutputAction::Save,
                &artifact,
                false,
            ),
            OutputFailureAction::Pending
        );
    }

    #[tokio::test]
    async fn finish_copy_worker_join_retains_output_pending_for_retry() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let result = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Copy,
            &TestWindows::default(),
            FinishOperations::new(
                |_| std::future::ready(Ok(lifecycle_artifact(vec![6, 5, 4]))),
                |_| false,
                |action, _| async move {
                    run_output_worker(action, || panic!("injected copy panic")).await
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect_err("copy join failure");
        assert_eq!(result.code, "longshot_controller_copy_failed");
        assert!(matches!(
            &*registry.slot.lock().expect("slot"),
            Slot::OutputPending { artifact, .. } if artifact.png.as_slice() == [6, 5, 4]
        ));
    }

    #[tokio::test]
    async fn pin_is_rejected_before_the_blocking_output_worker_runs() {
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_calls = Arc::clone(&calls);
        let error = run_output_worker(LongshotOutputAction::Pin, move || {
            worker_calls.fetch_add(1, Ordering::SeqCst);
            Ok(OutputValue::PinLabel("must-not-run".to_string()))
        })
        .await
        .expect_err("Pin must stay on the Tauri control path");

        assert!(matches!(
            error,
            finish::OutputWorkerError::Business(message)
                if message.contains("Tauri 控制路径")
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn finish_save_join_allows_copy_and_pin_and_policy_never_upgrades() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let finishes = AtomicUsize::new(0);

        let error = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Save,
            &TestWindows::default(),
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(vec![1, 6, 1, 8])))
                },
                |_| false,
                |action, _| async move {
                    run_output_worker(action, || panic!("injected save panic")).await
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect_err("save join");
        assert_eq!(error.code, "longshot_controller_save_uncertain");
        assert!(matches!(
            &*registry.slot.lock().expect("slot"),
            Slot::OutputPending {
                retry_policy: RetryPolicy::CopyPin,
                ..
            }
        ));

        let forbidden_finishes = AtomicUsize::new(0);
        let forbidden_outputs = AtomicUsize::new(0);
        let forbidden = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Save,
            &TestWindows::default(),
            FinishOperations::new(
                |_| {
                    forbidden_finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(Vec::new())))
                },
                |_| false,
                |_, _| {
                    forbidden_outputs.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(OutputValue::SavePath("must-not-save.png".to_string())))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect_err("retry save forbidden");
        assert_eq!(forbidden.code, "longshot_controller_save_uncertain");
        assert_eq!(forbidden_finishes.load(Ordering::SeqCst), 0);
        assert_eq!(forbidden_outputs.load(Ordering::SeqCst), 0);

        let pin_outputs = AtomicUsize::new(0);
        let pin_error = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Pin,
            &TestWindows::default(),
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(Vec::new())))
                },
                |_| false,
                |action, _| {
                    assert_eq!(action, LongshotOutputAction::Pin);
                    pin_outputs.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Err(finish::OutputWorkerError::Business(
                        "pin temporarily unavailable".to_string(),
                    )))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect_err("Save 不确定后仍可尝试 Pin");
        assert_eq!(pin_error.code, "longshot_controller_pin_failed");
        assert_eq!(pin_outputs.load(Ordering::SeqCst), 1);
        assert_eq!(finishes.load(Ordering::SeqCst), 1);

        let copy_error = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Copy,
            &TestWindows::default(),
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(Vec::new())))
                },
                |_| false,
                |action, _| {
                    assert_eq!(action, LongshotOutputAction::Copy);
                    std::future::ready(Err(finish::OutputWorkerError::Business(
                        "clipboard still busy".to_string(),
                    )))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect_err("copy retry failure");
        assert_eq!(copy_error.code, "longshot_controller_copy_failed");
        assert_eq!(finishes.load(Ordering::SeqCst), 1);
        assert!(matches!(
            &*registry.slot.lock().expect("slot"),
            Slot::OutputPending {
                retry_policy: RetryPolicy::CopyPin,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn finish_pin_uncertain_forbids_pin_but_keeps_copy_and_save() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let windows = TestWindows::default();
        windows.add_alive(&label);
        let finishes = AtomicUsize::new(0);
        let retained = Arc::new(Mutex::new(None::<Arc<LongshotOutputArtifact>>));
        let first_seen = Arc::clone(&retained);

        let error = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Pin,
            &windows,
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(vec![1, 4, 1, 4])))
                },
                |_| false,
                move |action, artifact| {
                    assert_eq!(action, LongshotOutputAction::Pin);
                    *first_seen.lock().expect("first") = Some(Arc::clone(&artifact));
                    std::future::ready(Err(finish::OutputWorkerError::Uncertain(
                        "native Pin completion unknown".to_string(),
                    )))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect_err("Pin outcome is uncertain");
        assert_eq!(error.code, "longshot_controller_pin_uncertain");

        let original = retained
            .lock()
            .expect("first")
            .as_ref()
            .expect("recorded")
            .clone();
        assert!(matches!(
            &*registry.slot.lock().expect("slot"),
            Slot::OutputPending {
                artifact,
                retry_policy: RetryPolicy::CopySave,
                ..
            } if Arc::ptr_eq(artifact, &original)
        ));

        let forbidden_finishes = AtomicUsize::new(0);
        let forbidden_outputs = AtomicUsize::new(0);
        let forbidden = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Pin,
            &windows,
            FinishOperations::new(
                |_| {
                    forbidden_finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(Vec::new())))
                },
                |_| false,
                |_, _| {
                    forbidden_outputs.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(OutputValue::PinLabel("must-not-pin".to_string())))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect_err("uncertain Pin retry must be rejected before side effects");
        assert_eq!(forbidden.code, "longshot_controller_pin_uncertain");
        assert_eq!(forbidden_finishes.load(Ordering::SeqCst), 0);
        assert_eq!(forbidden_outputs.load(Ordering::SeqCst), 0);

        let save_original = Arc::clone(&original);
        let saves = AtomicUsize::new(0);
        let save_error = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Save,
            &windows,
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(Vec::new())))
                },
                |_| false,
                |action, artifact| {
                    assert_eq!(action, LongshotOutputAction::Save);
                    assert!(Arc::ptr_eq(&artifact, &save_original));
                    saves.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Err(finish::OutputWorkerError::Business(
                        "disk temporarily unavailable".to_string(),
                    )))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect_err("Save remains allowed");
        assert_eq!(save_error.code, "longshot_controller_save_failed");
        assert_eq!(saves.load(Ordering::SeqCst), 1);

        let result = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Copy,
            &windows,
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(Vec::new())))
                },
                |_| false,
                move |action, artifact| {
                    assert_eq!(action, LongshotOutputAction::Copy);
                    assert!(Arc::ptr_eq(&artifact, &original));
                    std::future::ready(Ok(OutputValue::None))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect("Copy remains allowed");
        assert_eq!(result.action, LongshotOutputAction::Copy);
        assert_eq!(finishes.load(Ordering::SeqCst), 1);
        assert_eq!(windows.destroy_count(), 1);
    }

    #[tokio::test]
    async fn save_and_pin_uncertainty_compose_to_copy_only_without_reencoding() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let finishes = AtomicUsize::new(0);
        let retained = Arc::new(Mutex::new(None::<Arc<LongshotOutputArtifact>>));
        let first_seen = Arc::clone(&retained);

        let save_error = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Save,
            &TestWindows::default(),
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(vec![2, 7, 1, 8])))
                },
                |_| false,
                move |_, artifact| {
                    *first_seen.lock().expect("first") = Some(Arc::clone(&artifact));
                    std::future::ready(Err(finish::OutputWorkerError::Join(
                        "save completion unknown".to_string(),
                    )))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect_err("Save uncertain");
        assert_eq!(save_error.code, "longshot_controller_save_uncertain");
        let original = retained
            .lock()
            .expect("first")
            .as_ref()
            .expect("recorded")
            .clone();

        let pin_original = Arc::clone(&original);
        let pin_error = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Pin,
            &TestWindows::default(),
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(Vec::new())))
                },
                |_| false,
                move |_, artifact| {
                    assert!(Arc::ptr_eq(&artifact, &pin_original));
                    std::future::ready(Err(finish::OutputWorkerError::Uncertain(
                        "Pin completion unknown".to_string(),
                    )))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect_err("Pin uncertain");
        assert_eq!(pin_error.code, "longshot_controller_pin_uncertain");
        assert_eq!(finishes.load(Ordering::SeqCst), 1);
        assert!(matches!(
            &*registry.slot.lock().expect("slot"),
            Slot::OutputPending {
                artifact,
                retry_policy: RetryPolicy::CopyOnly,
                ..
            } if Arc::ptr_eq(artifact, &original)
        ));

        assert_eq!(
            registry
                .claim_finish(&label, &handle, LongshotOutputAction::Save)
                .expect_err("Save remains forbidden")
                .code,
            "longshot_controller_save_uncertain"
        );
        assert_eq!(
            registry
                .claim_finish(&label, &handle, LongshotOutputAction::Pin)
                .expect_err("Pin remains forbidden")
                .code,
            "longshot_controller_pin_uncertain"
        );

        let result = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Copy,
            &TestWindows::default(),
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(Vec::new())))
                },
                |_| false,
                move |action, artifact| {
                    assert_eq!(action, LongshotOutputAction::Copy);
                    assert!(Arc::ptr_eq(&artifact, &original));
                    std::future::ready(Ok(OutputValue::None))
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect("Copy remains allowed");
        assert_eq!(result.action, LongshotOutputAction::Copy);
        assert_eq!(finishes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn finish_destroyed_save_uncertain_drops_artifact() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let error = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Save,
            &TestWindows::default(),
            FinishOperations::new(
                |_| std::future::ready(Ok(lifecycle_artifact(vec![2, 0, 2, 6]))),
                |_| false,
                |_, _| {
                    std::future::ready(Err(finish::OutputWorkerError::Join(
                        "save completion unknown".to_string(),
                    )))
                },
                |_| std::future::ready(Ok(())),
                |boundary| {
                    if boundary == FinishBoundary::BeforeOutput {
                        assert_eq!(registry.claim_destroyed(&label), None);
                    }
                },
            ),
        )
        .await
        .expect_err("destroyed save remains uncertain");
        assert_eq!(error.code, "longshot_controller_save_uncertain");
        assert!(registry
            .reserve("capture-overlay-after-save".to_string(), selection())
            .is_ok());
    }

    #[test]
    fn finish_output_pending_discard_drops_artifact_without_lifecycle_cancel() {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let artifact = output_artifact(vec![3, 1, 4]);
        assert!(matches!(
            registry.claim_finish(&label, &handle, LongshotOutputAction::Copy),
            Ok(FinishClaim::Encoding(_))
        ));
        registry
            .publish_outputting(
                &label,
                &token,
                LongshotOutputAction::Copy,
                Arc::clone(&artifact),
            )
            .expect("publish outputting");
        assert_eq!(
            registry.complete_output_failure(
                &label,
                &token,
                LongshotOutputAction::Copy,
                &artifact,
                false,
            ),
            OutputFailureAction::Pending
        );
        assert_eq!(
            registry
                .claim_append(&label, &handle)
                .expect_err("pending blocks append")
                .code,
            "longshot_controller_busy"
        );
        assert!(matches!(
            registry.claim_cancel(&label, Some(&handle)),
            Ok(CancelAction::Close)
        ));
        assert!(registry
            .reserve("capture-overlay-next-7".to_string(), selection())
            .is_ok());
    }

    #[tokio::test]
    async fn finish_destroyed_at_every_success_boundary_never_cancels_or_resurrects() {
        for injected in [
            FinishBoundary::AfterClaim,
            FinishBoundary::AfterFinish,
            FinishBoundary::BeforeOutput,
            FinishBoundary::AfterOutput,
            FinishBoundary::BeforeCommit,
        ] {
            let (registry, label, token) = revealed_active_registry();
            let handle = LongshotControllerHandle::from_token(&token);
            let finishes = AtomicUsize::new(0);
            let copies = AtomicUsize::new(0);
            let cancels = AtomicUsize::new(0);
            let result = execute_finish_with_ops(
                &registry,
                &label,
                &handle,
                LongshotOutputAction::Copy,
                &TestWindows::default(),
                FinishOperations::new(
                    |_| {
                        finishes.fetch_add(1, Ordering::SeqCst);
                        std::future::ready(Ok(lifecycle_artifact(vec![1, 9, 9, 8])))
                    },
                    |_| false,
                    |_, _| {
                        copies.fetch_add(1, Ordering::SeqCst);
                        std::future::ready(Ok(OutputValue::None))
                    },
                    |_| {
                        cancels.fetch_add(1, Ordering::SeqCst);
                        std::future::ready(Ok(()))
                    },
                    |boundary| {
                        if boundary == injected {
                            assert_eq!(registry.claim_destroyed(&label), None);
                        }
                    },
                ),
            )
            .await
            .expect("finish remains authorized after click");
            assert_eq!(result.action, LongshotOutputAction::Copy);
            assert_eq!(finishes.load(Ordering::SeqCst), 1);
            assert_eq!(copies.load(Ordering::SeqCst), 1);
            assert_eq!(cancels.load(Ordering::SeqCst), 0);
            assert!(registry
                .reserve("capture-overlay-next-7".to_string(), selection())
                .is_ok());
        }
    }

    #[tokio::test]
    async fn finish_pin_success_survives_destroyed_at_every_commit_boundary() {
        for injected in [
            FinishBoundary::AfterClaim,
            FinishBoundary::AfterFinish,
            FinishBoundary::BeforeOutput,
            FinishBoundary::AfterOutput,
            FinishBoundary::BeforeCommit,
        ] {
            let (registry, label, token) = revealed_active_registry();
            let handle = LongshotControllerHandle::from_token(&token);
            let finishes = AtomicUsize::new(0);
            let pins = AtomicUsize::new(0);
            let result = execute_finish_with_ops(
                &registry,
                &label,
                &handle,
                LongshotOutputAction::Pin,
                &TestWindows::default(),
                FinishOperations::new(
                    |_| {
                        finishes.fetch_add(1, Ordering::SeqCst);
                        std::future::ready(Ok(lifecycle_artifact(vec![2, 0, 2, 6])))
                    },
                    |_| false,
                    |action, artifact| {
                        assert_eq!(action, LongshotOutputAction::Pin);
                        assert_eq!(artifact.origin, test_origin());
                        pins.fetch_add(1, Ordering::SeqCst);
                        std::future::ready(Ok(OutputValue::PinLabel(
                            "pin-image-boundary".to_string(),
                        )))
                    },
                    |_| std::future::ready(Ok(())),
                    |boundary| {
                        if boundary == injected {
                            assert_eq!(registry.claim_destroyed(&label), None);
                        }
                    },
                ),
            )
            .await
            .expect("已领取的 Pin 输出保持授权");

            assert_eq!(result.action, LongshotOutputAction::Pin);
            assert_eq!(result.pin_label.as_deref(), Some("pin-image-boundary"));
            assert_eq!(finishes.load(Ordering::SeqCst), 1);
            assert_eq!(pins.load(Ordering::SeqCst), 1);
            assert!(registry
                .reserve("capture-overlay-after-pin".to_string(), selection())
                .is_ok());
        }
    }
    #[test]
    fn handoff_failure_keeps_the_exact_ordinary_origin_after_activation_consumes_launch() {
        let registry = LongshotControllerRegistry::new();
        let source = selection();
        let label = registry
            .reserve("capture-overlay-original".into(), source.clone())
            .unwrap();
        registry.publish_started(&label, CONTROLLER_PAGE);
        registry.claim_activation(&label).unwrap();
        let (caller, result) = registry
            .take_handoff(&label, false, |id| id == source.session_id)
            .unwrap();
        assert_eq!(caller, "capture-overlay-original");
        let json = serde_json::to_value(result).unwrap();
        assert_eq!(json["controllerLabel"], label);
        assert_eq!(json["sessionId"], source.session_id);
        assert_eq!(json["accepted"], false);
        assert!(registry.take_handoff(&label, false, |_| true).is_none());
    }

    #[test]
    fn controller_deadline_or_destroy_preserves_one_failure_notification() {
        for destroyed in [true, false] {
            let registry = LongshotControllerRegistry::new();
            let label = registry
                .reserve("capture-overlay-original".into(), selection())
                .unwrap();
            if destroyed {
                registry.claim_destroyed(&label);
            } else {
                registry.claim_deadline(&label);
            }
            assert!(registry.take_handoff(&label, false, |_| true).is_some());
            assert!(registry.take_handoff(&label, false, |_| true).is_none());
        }
    }

    #[test]
    fn stale_handoff_cannot_unlock_a_replaced_ordinary_capture() {
        let registry = LongshotControllerRegistry::new();
        let label = registry
            .reserve("capture-overlay-old".into(), selection())
            .unwrap();
        assert!(registry.take_handoff(&label, false, |_| false).is_none());
        assert!(registry
            .take_handoff("another-controller", false, |_| true)
            .is_none());
    }
}
