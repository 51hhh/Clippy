//! 长截图控制窗 registry 的精确 claim/commit/rollback 转移。

use super::model::{
    AppendClaim, CancelAction, CancelFailureRecovery, DeadlineAction, HandoffResult, Launch,
    LongshotActivation, LongshotControllerHandle, LongshotControllerRegistry, LongshotIpcError,
    LongshotSnapshotDto, ReadyAction, RetryVisibility, Slot, TerminationOrigin,
};
use super::{CONTROLLER_PAGE, CONTROLLER_PREFIX};
use crate::capture::longshot::{LongshotSessionToken, LongshotSnapshot, LongshotStart};
use crate::capture::{CaptureError, CaptureSelection};

impl LongshotControllerRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(super) fn reserve(
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

    pub(super) fn take_handoff(
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

    pub(super) fn publish_started(&self, label: &str, path: &str) -> bool {
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

    pub(super) fn abort_build(&self, label: &str) {
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

    pub(super) fn accepts_built_window(&self, label: &str) -> bool {
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

    pub(super) fn claim_activation(
        &self,
        label: &str,
    ) -> Result<CaptureSelection, LongshotIpcError> {
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

    pub(super) fn complete_activation(
        &self,
        label: &str,
        result: Result<LongshotStart, CaptureError>,
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

    pub(super) fn claim_ready(&self, label: &str) -> Result<ReadyAction, LongshotIpcError> {
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

    pub(super) fn claim_append(
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
    pub(super) fn authorize_preview(
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
    pub(super) fn confirms_preview(
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

    pub(super) fn owns_append(&self, label: &str, token: &LongshotSessionToken) -> bool {
        self.slot.lock().is_ok_and(|slot| {
            matches!(&*slot, Slot::Appending {
                label: current,
                token: current_token,
                ..
            } if current == label && current_token == token)
        })
    }

    pub(super) fn complete_append_visible(
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

    pub(super) fn claim_append_visibility_failure(
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

    pub(super) fn claim_cancel(
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

    pub(super) fn claim_deadline(&self, label: &str) -> DeadlineAction {
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

    pub(super) fn claim_destroyed(&self, label: &str) -> Option<LongshotSessionToken> {
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

    pub(super) fn complete_cancel_success(&self, label: &str, token: &LongshotSessionToken) {
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

    pub(super) fn begin_cancel_failure_recovery(
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

    pub(super) fn owns_hidden_cancel_reveal(
        &self,
        label: &str,
        token: &LongshotSessionToken,
    ) -> bool {
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

    pub(super) fn complete_cancel_failure(
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

    pub(super) fn complete_hidden_cancel_reveal(
        &self,
        label: &str,
        token: &LongshotSessionToken,
    ) -> bool {
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

    pub(super) fn fail_hidden_cancel_reveal(&self, label: &str, token: &LongshotSessionToken) {
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
    pub(super) fn claim_forced_termination(
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

    pub(super) fn mark_cleanup_failed(
        &self,
        label: &str,
        token: Option<LongshotSessionToken>,
    ) -> bool {
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
    pub(super) fn settle_termination_cleanup_failed(
        &self,
        label: &str,
        token: &LongshotSessionToken,
    ) -> bool {
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

    pub(super) fn rollback_cleanup_reveal(&self, label: &str) {
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

    pub(super) fn mark_cleanup_revealed(&self, label: &str) {
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
    pub(super) fn settle_emergency_cleanup(
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
