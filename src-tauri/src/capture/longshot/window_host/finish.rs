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
    Encoding(LongshotOutputAction),
    Outputting {
        action: LongshotOutputAction,
        png: Arc<Vec<u8>>,
        retry_policy: RetryPolicy,
    },
}

#[derive(Debug)]
pub(super) enum FinishClaim {
    Encoding(LongshotSessionToken),
    Outputting(LongshotSessionToken, Arc<Vec<u8>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RetryPolicy {
    Any,
    CopyOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FinishFailureAction {
    Restored,
    Compensate(LongshotSessionToken),
    CleanupFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OutputFailureAction {
    Pending,
    Dropped,
    OwnershipLost,
}

impl FinishClaim {
    fn token(&self) -> &LongshotSessionToken {
        match self {
            Self::Encoding(token) | Self::Outputting(token, _) => token,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FinishBoundary {
    AfterClaim,
    AfterFinish,
    BeforeOutput,
    AfterOutput,
    BeforeCommit,
}

pub(super) enum FinishWorkerError {
    Business(CaptureError),
    Join(String),
}

pub(super) enum OutputWorkerError {
    Business(String),
    Join(String),
}

pub(super) struct FinishOperations<W, P, O, X, H> {
    finish_worker: W,
    probe_exact_active: P,
    output_worker: O,
    compensate: X,
    boundary: H,
}

impl<W, P, O, X, H> FinishOperations<W, P, O, X, H> {
    pub(super) fn new<WF, OF, XF>(
        finish_worker: W,
        probe_exact_active: P,
        output_worker: O,
        compensate: X,
        boundary: H,
    ) -> Self
    where
        W: FnOnce(LongshotSessionToken) -> WF,
        P: FnOnce(&LongshotSessionToken) -> bool,
        O: FnOnce(LongshotOutputAction, Arc<Vec<u8>>) -> OF,
        X: FnOnce(LongshotSessionToken) -> XF,
        H: FnMut(FinishBoundary),
    {
        Self {
            finish_worker,
            probe_exact_active,
            output_worker,
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
        action: LongshotOutputAction,
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
                    stage: FinishStage::Encoding(action),
                    window_destroyed: false,
                };
                Ok(FinishClaim::Encoding(token))
            }
            Slot::OutputPending {
                label: current,
                token,
                snapshot,
                png,
                retry_policy,
            } if current == label && token == wire_token => {
                if retry_policy == RetryPolicy::CopyOnly && action == LongshotOutputAction::Save {
                    *slot = Slot::OutputPending {
                        label: current,
                        token,
                        snapshot,
                        png,
                        retry_policy,
                    };
                    return Err(LongshotIpcError::save_uncertain(
                        "上次保存结果不确定，禁止再次保存以免生成重复文件",
                    ));
                }
                *slot = Slot::Finishing {
                    label: current,
                    token: token.clone(),
                    snapshot,
                    stage: FinishStage::Outputting {
                        action,
                        png: Arc::clone(&png),
                        retry_policy,
                    },
                    window_destroyed: false,
                };
                Ok(FinishClaim::Outputting(token, png))
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
                retry_policy,
            } if current == label => {
                *slot = Slot::OutputPending {
                    label: current,
                    token,
                    snapshot,
                    png,
                    retry_policy,
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

    pub(super) fn publish_outputting(
        &self,
        label: &str,
        token: &LongshotSessionToken,
        action: LongshotOutputAction,
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
                stage: FinishStage::Encoding(current_action),
                window_destroyed,
            } if current == label && current_token == *token && current_action == action => {
                *slot = Slot::Finishing {
                    label: current,
                    token: current_token,
                    snapshot,
                    stage: FinishStage::Outputting {
                        action,
                        png,
                        retry_policy: RetryPolicy::Any,
                    },
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
        action: LongshotOutputAction,
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
                stage: FinishStage::Encoding(current_action),
                window_destroyed,
            } if current == label && current_token == *token && current_action == action => {
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

    fn fail_finish_uncertain(
        &self,
        label: &str,
        token: &LongshotSessionToken,
        action: LongshotOutputAction,
    ) {
        let Ok(mut slot) = self.slot.lock() else {
            return;
        };
        if matches!(&*slot, Slot::Finishing {
            label: current,
            token: current_token,
            stage: FinishStage::Encoding(current_action),
            ..
        } if current == label && current_token == token && *current_action == action)
        {
            *slot = Slot::CleanupFailed {
                label: label.to_string(),
                _token: Some(token.clone()),
                revealed: false,
            };
        }
    }

    pub(super) fn complete_output_failure(
        &self,
        label: &str,
        token: &LongshotSessionToken,
        action: LongshotOutputAction,
        png: &Arc<Vec<u8>>,
        uncertain: bool,
    ) -> OutputFailureAction {
        let Ok(mut slot) = self.slot.lock() else {
            return OutputFailureAction::OwnershipLost;
        };
        let previous = std::mem::take(&mut *slot);
        match previous {
            Slot::Finishing {
                label: current,
                token: current_token,
                snapshot,
                stage:
                    FinishStage::Outputting {
                        action: current_action,
                        png: current_png,
                        retry_policy,
                    },
                window_destroyed,
            } if current == label
                && current_token == *token
                && current_action == action
                && Arc::ptr_eq(&current_png, png) =>
            {
                if window_destroyed {
                    *slot = Slot::Empty;
                    OutputFailureAction::Dropped
                } else {
                    let retry_policy = if uncertain && action == LongshotOutputAction::Save {
                        RetryPolicy::CopyOnly
                    } else {
                        retry_policy
                    };
                    *slot = Slot::OutputPending {
                        label: current,
                        token: current_token,
                        snapshot,
                        png: current_png,
                        retry_policy,
                    };
                    OutputFailureAction::Pending
                }
            }
            other => {
                *slot = other;
                OutputFailureAction::OwnershipLost
            }
        }
    }

    fn complete_output_success(
        &self,
        label: &str,
        token: &LongshotSessionToken,
        action: LongshotOutputAction,
        png: &Arc<Vec<u8>>,
    ) -> bool {
        let Ok(mut slot) = self.slot.lock() else {
            return false;
        };
        let exact = matches!(&*slot, Slot::Finishing {
            label: current,
            token: current_token,
            stage: FinishStage::Outputting {
                action: current_action,
                png: current_png,
                ..
            },
            ..
        } if current == label
            && current_token == token
            && *current_action == action
            && Arc::ptr_eq(current_png, png));
        if exact {
            *slot = Slot::Empty;
        }
        exact
    }
}

pub(super) async fn execute_finish_with_ops<A, W, WF, P, O, OF, X, XF, H>(
    registry: &LongshotControllerRegistry,
    label: &str,
    handle: &LongshotControllerHandle,
    action: LongshotOutputAction,
    windows: &A,
    operations: FinishOperations<W, P, O, X, H>,
) -> Result<LongshotOutputResult, LongshotIpcError>
where
    A: ControlWindowActions,
    W: FnOnce(LongshotSessionToken) -> WF,
    WF: std::future::Future<Output = Result<Vec<u8>, FinishWorkerError>>,
    P: FnOnce(&LongshotSessionToken) -> bool,
    O: FnOnce(LongshotOutputAction, Arc<Vec<u8>>) -> OF,
    OF: std::future::Future<Output = Result<Option<String>, OutputWorkerError>>,
    X: FnOnce(LongshotSessionToken) -> XF,
    XF: std::future::Future<Output = Result<(), LongshotIpcError>>,
    H: FnMut(FinishBoundary),
{
    let FinishOperations {
        finish_worker,
        probe_exact_active,
        output_worker,
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
                registry.publish_outputting(label, &token, action, Arc::clone(&png))?;
                png
            }
            Err(FinishWorkerError::Business(primary)) => {
                let exact_active = probe_exact_active(&token);
                return match registry.complete_finish_failure(label, &token, action, exact_active) {
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
                registry.fail_finish_uncertain(label, &token, action);
                return Err(LongshotIpcError::cleanup_failed(message));
            }
        },
        FinishClaim::Outputting(_, png) => png,
    };

    boundary(FinishBoundary::BeforeOutput);
    let output_result = match (action, output_worker(action, Arc::clone(&png)).await) {
        (LongshotOutputAction::Copy, Ok(_)) => Ok(None),
        (LongshotOutputAction::Save, Ok(Some(path))) if !path.is_empty() => Ok(Some(path)),
        (LongshotOutputAction::Save, Ok(_)) => Err(OutputWorkerError::Business(
            "长截图保存成功但未返回有效路径".to_string(),
        )),
        (_, Err(error)) => Err(error),
    };
    boundary(FinishBoundary::AfterOutput);
    let path = match output_result {
        Ok(path) => path,
        Err(error) => {
            let uncertain =
                matches!(error, OutputWorkerError::Join(_)) && action == LongshotOutputAction::Save;
            let message = match error {
                OutputWorkerError::Business(message) | OutputWorkerError::Join(message) => message,
            };
            return match registry.complete_output_failure(label, &token, action, &png, uncertain) {
                OutputFailureAction::Pending | OutputFailureAction::Dropped => Err(match action {
                    LongshotOutputAction::Copy => LongshotIpcError::copy_failed(message),
                    LongshotOutputAction::Save if uncertain => {
                        LongshotIpcError::save_uncertain(message)
                    }
                    LongshotOutputAction::Save => LongshotIpcError::save_failed(message),
                }),
                OutputFailureAction::OwnershipLost => Err(LongshotIpcError::cleanup_failed(
                    "长截图输出失败后控制权已经丢失",
                )),
            };
        }
    };

    boundary(FinishBoundary::BeforeCommit);
    if !registry.complete_output_success(label, &token, action, &png) {
        return Err(LongshotIpcError::cleanup_failed(
            "长截图输出成功后控制权已经丢失",
        ));
    }
    windows.destroy(label);
    Ok(LongshotOutputResult { action, path })
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

pub(super) async fn run_output_worker<F>(
    action: LongshotOutputAction,
    work: F,
) -> Result<Option<String>, OutputWorkerError>
where
    F: FnOnce() -> Result<Option<String>, String> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(work).await {
        Ok(Ok(path)) => Ok(path),
        Ok(Err(error)) => Err(OutputWorkerError::Business(error)),
        Err(error) => Err(OutputWorkerError::Join(match action {
            LongshotOutputAction::Copy => format!("长截图复制线程异常: {error}"),
            LongshotOutputAction::Save => format!("长截图保存线程异常: {error}"),
        })),
    }
}
