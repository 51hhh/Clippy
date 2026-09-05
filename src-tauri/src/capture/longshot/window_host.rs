//! 独立长截图控制窗的两阶段接管与补偿状态机。

use super::{LongshotSessionToken, LongshotSnapshot};
use crate::capture::{CaptureError, CaptureSelection};
use crate::commands::AppState;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::Duration;
use tauri::Manager;

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
    Terminating {
        label: String,
        token: LongshotSessionToken,
        snapshot: Option<LongshotSnapshot>,
        window_destroyed: bool,
    },
    CleanupFailed {
        label: String,
        _token: Option<LongshotSessionToken>,
        revealed: bool,
    },
}

#[derive(Debug)]
struct Launch {
    label: String,
    selection: CaptureSelection,
    caller_label: String,
}

#[derive(Default)]
pub(crate) struct LongshotControllerRegistry {
    slot: Mutex<Slot>,
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
    fn show(&self, label: &str) -> Result<(), LongshotIpcError>;
    fn focus(&self, label: &str);
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
        *slot = Slot::Building(Launch {
            label: label.clone(),
            selection,
            caller_label,
        });
        Ok(label)
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
                };
                Ok(CancelAction::Terminate(token))
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
                };
                Some(token)
            }
            Slot::Terminating {
                label: current,
                token,
                snapshot,
                ..
            } if current == label => {
                *slot = Slot::Terminating {
                    label: current,
                    token,
                    snapshot,
                    window_destroyed: true,
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

    fn complete_cancel_failure(&self, label: &str, token: &LongshotSessionToken, retryable: bool) {
        let Ok(mut slot) = self.slot.lock() else {
            log::error!("长截图取消失败后无法保留 registry");
            return;
        };
        let previous = std::mem::take(&mut *slot);
        *slot = match previous {
            Slot::Terminating {
                label: current,
                token: current_token,
                snapshot,
                window_destroyed,
            } if current == label && current_token == *token => {
                if let (true, false, Some(snapshot)) = (retryable, window_destroyed, snapshot) {
                    Slot::Active {
                        label: current,
                        token: current_token,
                        snapshot,
                        revealed: true,
                    }
                } else {
                    Slot::CleanupFailed {
                        label: current,
                        _token: Some(current_token),
                        revealed: false,
                    }
                }
            }
            other => other,
        };
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
            .inner_size(360.0, 180.0)
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
    execute_cancel_action(action, caller_label, &windows, |token| {
        cancel_claimed(&app, state, caller_label, token)
    })
    .await
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
            let window_alive = app.get_webview_window(label).is_some();
            let retryable = window_alive
                && state
                    .longshot_lifecycle
                    .is_exact_active(&token)
                    .unwrap_or(false);
            state
                .longshot_windows
                .complete_cancel_failure(label, &token, retryable);
            if retryable {
                Err(primary.into())
            } else {
                log::error!("长截图控制窗 {label} 清理失败且状态不可安全重试: {primary}");
                Err(LongshotIpcError::cleanup_failed(primary.to_string()))
            }
        }
    }
}

pub(crate) fn handle_controller_destroyed(app: &tauri::AppHandle, label: &str) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let token = state.longshot_windows.claim_destroyed(label);
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
        focused: Mutex<Vec<String>>,
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

        fn show_count(&self) -> usize {
            self.shown.lock().expect("shown").len()
        }

        fn focus_count(&self) -> usize {
            self.focused.lock().expect("focused").len()
        }
    }

    impl ControlWindowActions for TestWindows {
        fn destroy(&self, label: &str) {
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

        fn show(&self, label: &str) -> Result<(), LongshotIpcError> {
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
            self.focused
                .lock()
                .expect("focused")
                .push(label.to_string());
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
}
