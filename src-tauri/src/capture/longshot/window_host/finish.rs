//! 长截图完成、复制与待重试输出的状态转换。

use super::{
    ControlWindowActions, LongshotControllerHandle, LongshotControllerRegistry, LongshotIpcError,
    LongshotOutputAction, LongshotOutputResult, Slot, TerminationOrigin, CONTROLLER_PREFIX,
};
use crate::capture::longshot::LongshotSessionToken;
use crate::capture::CaptureError;
use std::sync::Arc;

#[derive(Debug)]
pub(super) enum FinishStage {
    Encoding,
    Copying(Arc<Vec<u8>>),
}

#[derive(Debug)]
pub(super) enum FinishClaim {
    Encoding(LongshotSessionToken),
    Copying(LongshotSessionToken, Arc<Vec<u8>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FinishFailureAction {
    Restored,
    Compensate(LongshotSessionToken),
    CleanupFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CopyFailureAction {
    Pending,
    Dropped,
    OwnershipLost,
}

impl FinishClaim {
    fn token(&self) -> &LongshotSessionToken {
        match self {
            Self::Encoding(token) | Self::Copying(token, _) => token,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FinishBoundary {
    AfterClaim,
    AfterFinish,
    BeforeCopy,
    AfterCopy,
    BeforeCommit,
}

pub(super) enum FinishWorkerError {
    Business(CaptureError),
    Join(String),
}

pub(super) struct FinishOperations<W, P, C, X, H> {
    finish_worker: W,
    probe_exact_active: P,
    copy_worker: C,
    compensate: X,
    boundary: H,
}

impl<W, P, C, X, H> FinishOperations<W, P, C, X, H> {
    pub(super) fn new<WF, CF, XF>(
        finish_worker: W,
        probe_exact_active: P,
        copy_worker: C,
        compensate: X,
        boundary: H,
    ) -> Self
    where
        W: FnOnce(LongshotSessionToken) -> WF,
        P: FnOnce(&LongshotSessionToken) -> bool,
        C: FnOnce(Arc<Vec<u8>>) -> CF,
        X: FnOnce(LongshotSessionToken) -> XF,
        H: FnMut(FinishBoundary),
    {
        Self {
            finish_worker,
            probe_exact_active,
            copy_worker,
            compensate,
            boundary,
        }
    }
}

impl LongshotControllerRegistry {
    pub(super) fn claim_finish(
        &self,
        label: &str,
        handle: &LongshotControllerHandle,
        _action: LongshotOutputAction,
    ) -> Result<FinishClaim, LongshotIpcError> {
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
                *slot = Slot::Finishing {
                    label: current,
                    token: token.clone(),
                    snapshot,
                    stage: FinishStage::Encoding,
                    window_destroyed: false,
                };
                Ok(FinishClaim::Encoding(token))
            }
            Slot::OutputPending {
                label: current,
                token,
                snapshot,
                png,
            } if current == label && token == wire_token => {
                *slot = Slot::Finishing {
                    label: current,
                    token: token.clone(),
                    snapshot,
                    stage: FinishStage::Copying(Arc::clone(&png)),
                    window_destroyed: false,
                };
                Ok(FinishClaim::Copying(token, png))
            }
            Slot::Active {
                label: current,
                token,
                snapshot,
                revealed,
            } if current == label => {
                let error = if revealed {
                    LongshotIpcError::superseded()
                } else {
                    LongshotIpcError::missing()
                };
                *slot = Slot::Active {
                    label: current,
                    token,
                    snapshot,
                    revealed,
                };
                Err(error)
            }
            Slot::Appending {
                label: current,
                token,
                snapshot,
            } if current == label => {
                let error = if token == wire_token {
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
                let error = if token == wire_token {
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
                png,
            } if current == label => {
                *slot = Slot::OutputPending {
                    label: current,
                    token,
                    snapshot,
                    png,
                };
                Err(LongshotIpcError::superseded())
            }
            Slot::Terminating {
                label: current,
                token,
                snapshot,
                window_destroyed,
                origin,
            } if current == label => {
                let error = if token == wire_token {
                    LongshotIpcError::busy()
                } else {
                    LongshotIpcError::superseded()
                };
                *slot = Slot::Terminating {
                    label: current,
                    token,
                    snapshot,
                    window_destroyed,
                    origin,
                };
                Err(error)
            }
            other => {
                *slot = other;
                Err(LongshotIpcError::missing())
            }
        }
    }

    pub(super) fn publish_copying(
        &self,
        label: &str,
        token: &LongshotSessionToken,
        png: Arc<Vec<u8>>,
    ) -> Result<(), LongshotIpcError> {
        let mut slot = self
            .slot
            .lock()
            .map_err(|error| LongshotIpcError::internal(error.to_string()))?;
        let previous = std::mem::take(&mut *slot);
        match previous {
            Slot::Finishing {
                label: current,
                token: current_token,
                snapshot,
                stage: FinishStage::Encoding,
                window_destroyed,
            } if current == label && current_token == *token => {
                *slot = Slot::Finishing {
                    label: current,
                    token: current_token,
                    snapshot,
                    stage: FinishStage::Copying(png),
                    window_destroyed,
                };
                Ok(())
            }
            other => {
                *slot = other;
                Err(LongshotIpcError::cleanup_failed(
                    "长截图编码完成后控制权已经丢失",
                ))
            }
        }
    }

    fn complete_finish_failure(
        &self,
        label: &str,
        token: &LongshotSessionToken,
        exact_active: bool,
    ) -> FinishFailureAction {
        let Ok(mut slot) = self.slot.lock() else {
            return FinishFailureAction::CleanupFailed;
        };
        let previous = std::mem::take(&mut *slot);
        match previous {
            Slot::Finishing {
                label: current,
                token: current_token,
                snapshot,
                stage: FinishStage::Encoding,
                window_destroyed,
            } if current == label && current_token == *token => {
                if !exact_active {
                    *slot = Slot::CleanupFailed {
                        label: current,
                        _token: Some(current_token),
                        revealed: false,
                    };
                    FinishFailureAction::CleanupFailed
                } else if window_destroyed {
                    *slot = Slot::Terminating {
                        label: current,
                        token: current_token.clone(),
                        snapshot: Some(snapshot),
                        window_destroyed: true,
                        origin: TerminationOrigin::RevealedActive,
                    };
                    FinishFailureAction::Compensate(current_token)
                } else {
                    *slot = Slot::Active {
                        label: current,
                        token: current_token,
                        snapshot,
                        revealed: true,
                    };
                    FinishFailureAction::Restored
                }
            }
            other => {
                *slot = other;
                FinishFailureAction::CleanupFailed
            }
        }
    }

    fn fail_finish_uncertain(&self, label: &str, token: &LongshotSessionToken) {
        let Ok(mut slot) = self.slot.lock() else {
            return;
        };
        if matches!(&*slot, Slot::Finishing {
            label: current,
            token: current_token,
            stage: FinishStage::Encoding,
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

    pub(super) fn complete_copy_failure(
        &self,
        label: &str,
        token: &LongshotSessionToken,
        png: &Arc<Vec<u8>>,
    ) -> CopyFailureAction {
        let Ok(mut slot) = self.slot.lock() else {
            return CopyFailureAction::OwnershipLost;
        };
        let previous = std::mem::take(&mut *slot);
        match previous {
            Slot::Finishing {
                label: current,
                token: current_token,
                snapshot,
                stage: FinishStage::Copying(current_png),
                window_destroyed,
            } if current == label && current_token == *token && Arc::ptr_eq(&current_png, png) => {
                if window_destroyed {
                    *slot = Slot::Empty;
                    CopyFailureAction::Dropped
                } else {
                    *slot = Slot::OutputPending {
                        label: current,
                        token: current_token,
                        snapshot,
                        png: current_png,
                    };
                    CopyFailureAction::Pending
                }
            }
            other => {
                *slot = other;
                CopyFailureAction::OwnershipLost
            }
        }
    }

    fn complete_copy_success(
        &self,
        label: &str,
        token: &LongshotSessionToken,
        png: &Arc<Vec<u8>>,
    ) -> bool {
        let Ok(mut slot) = self.slot.lock() else {
            return false;
        };
        let exact = matches!(&*slot, Slot::Finishing {
            label: current,
            token: current_token,
            stage: FinishStage::Copying(current_png),
            ..
        } if current == label && current_token == token && Arc::ptr_eq(current_png, png));
        if exact {
            *slot = Slot::Empty;
        }
        exact
    }
}

pub(super) async fn execute_finish_with_ops<A, W, WF, P, C, CF, X, XF, H>(
    registry: &LongshotControllerRegistry,
    label: &str,
    handle: &LongshotControllerHandle,
    action: LongshotOutputAction,
    windows: &A,
    operations: FinishOperations<W, P, C, X, H>,
) -> Result<LongshotOutputResult, LongshotIpcError>
where
    A: ControlWindowActions,
    W: FnOnce(LongshotSessionToken) -> WF,
    WF: std::future::Future<Output = Result<Vec<u8>, FinishWorkerError>>,
    P: FnOnce(&LongshotSessionToken) -> bool,
    C: FnOnce(Arc<Vec<u8>>) -> CF,
    CF: std::future::Future<Output = Result<(), String>>,
    X: FnOnce(LongshotSessionToken) -> XF,
    XF: std::future::Future<Output = Result<(), LongshotIpcError>>,
    H: FnMut(FinishBoundary),
{
    let FinishOperations {
        finish_worker,
        probe_exact_active,
        copy_worker,
        compensate,
        mut boundary,
    } = operations;
    let claim = registry.claim_finish(label, handle, action)?;
    let token = claim.token().clone();
    boundary(FinishBoundary::AfterClaim);

    let png = match claim {
        FinishClaim::Encoding(token) => match finish_worker(token.clone()).await {
            Ok(bytes) => {
                boundary(FinishBoundary::AfterFinish);
                let png = Arc::new(bytes);
                registry.publish_copying(label, &token, Arc::clone(&png))?;
                png
            }
            Err(FinishWorkerError::Business(primary)) => {
                let exact_active = probe_exact_active(&token);
                return match registry.complete_finish_failure(label, &token, exact_active) {
                    FinishFailureAction::Restored => Err(primary.into()),
                    FinishFailureAction::Compensate(claimed) => {
                        match compensate(claimed.clone()).await {
                            Ok(()) => {
                                registry.complete_cancel_success(label, &claimed);
                                Err(primary.into())
                            }
                            Err(error) => {
                                let _ = registry.settle_termination_cleanup_failed(label, &claimed);
                                Err(LongshotIpcError::cleanup_failed(error.message))
                            }
                        }
                    }
                    FinishFailureAction::CleanupFailed => Err(LongshotIpcError::cleanup_failed(
                        "长截图编码失败后无法确认会话状态",
                    )),
                };
            }
            Err(FinishWorkerError::Join(message)) => {
                registry.fail_finish_uncertain(label, &token);
                return Err(LongshotIpcError::cleanup_failed(message));
            }
        },
        FinishClaim::Copying(_, png) => png,
    };

    boundary(FinishBoundary::BeforeCopy);
    let copy_result = copy_worker(Arc::clone(&png)).await;
    boundary(FinishBoundary::AfterCopy);
    if let Err(message) = copy_result {
        return match registry.complete_copy_failure(label, &token, &png) {
            CopyFailureAction::Pending | CopyFailureAction::Dropped => {
                Err(LongshotIpcError::copy_failed(message))
            }
            CopyFailureAction::OwnershipLost => Err(LongshotIpcError::cleanup_failed(
                "长截图复制失败后控制权已经丢失",
            )),
        };
    }

    boundary(FinishBoundary::BeforeCommit);
    if !registry.complete_copy_success(label, &token, &png) {
        return Err(LongshotIpcError::cleanup_failed(
            "长截图复制成功后控制权已经丢失",
        ));
    }
    windows.destroy(label);
    Ok(LongshotOutputResult { action })
}

pub(super) async fn run_finish_worker<F>(work: F) -> Result<Vec<u8>, FinishWorkerError>
where
    F: FnOnce() -> Result<Vec<u8>, CaptureError> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(work).await {
        Ok(Ok(png)) => Ok(png),
        Ok(Err(error)) => Err(FinishWorkerError::Business(error)),
        Err(error) => Err(FinishWorkerError::Join(format!(
            "长截图完成线程异常: {error}"
        ))),
    }
}

pub(super) async fn run_copy_worker<F>(work: F) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(work).await {
        Ok(result) => result,
        Err(error) => Err(format!("长截图复制线程异常: {error}")),
    }
}
