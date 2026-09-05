//! 长截图完成、复制与待重试输出的状态转换。

use super::{
    ControlWindowActions, LongshotControllerHandle, LongshotControllerRegistry, LongshotIpcError,
    LongshotOutputAction, LongshotOutputArtifact, LongshotOutputResult, Slot, TerminationOrigin,
    CONTROLLER_PREFIX,
};
use crate::capture::longshot::{LongshotArtifact, LongshotSessionToken};
use crate::capture::CaptureError;
use std::sync::Arc;

#[derive(Debug)]
pub(super) enum FinishStage {
    Encoding(LongshotOutputAction),
    Outputting {
        action: LongshotOutputAction,
        artifact: Arc<LongshotOutputArtifact>,
        retry_policy: RetryPolicy,
    },
}

#[derive(Debug)]
pub(super) enum FinishClaim {
    Encoding(LongshotSessionToken),
    Outputting(LongshotSessionToken, Arc<LongshotOutputArtifact>),
}

/// 尚可安全重试的输出动作集合。
///
/// 对未知完成结果的动作只会移除权限，绝不会把已移除的权限加回来。
/// 虽然当前复制永远可重试，仍把全部三种动作编码出来，避免后续状态迁移
/// 把「只允许复制」错误地当作「允许所有动作」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RetryPolicy {
    None,
    CopyOnly,
    SaveOnly,
    PinOnly,
    CopySave,
    CopyPin,
    SavePin,
    Any,
}

impl RetryPolicy {
    fn allows(self, action: LongshotOutputAction) -> bool {
        !matches!(
            (self, action),
            (Self::None, _)
                | (
                    Self::CopyOnly,
                    LongshotOutputAction::Save | LongshotOutputAction::Pin
                )
                | (
                    Self::SaveOnly,
                    LongshotOutputAction::Copy | LongshotOutputAction::Pin
                )
                | (
                    Self::PinOnly,
                    LongshotOutputAction::Copy | LongshotOutputAction::Save
                )
                | (Self::CopySave, LongshotOutputAction::Pin)
                | (Self::CopyPin, LongshotOutputAction::Save)
                | (Self::SavePin, LongshotOutputAction::Copy)
        )
    }

    /// 从许可集合中移除一个动作。返回值始终是 `self` 的子集。
    fn without(self, action: LongshotOutputAction) -> Self {
        match (self, action) {
            (Self::None, _) => Self::None,
            (Self::CopyOnly, LongshotOutputAction::Copy) => Self::None,
            (Self::SaveOnly, LongshotOutputAction::Save) => Self::None,
            (Self::PinOnly, LongshotOutputAction::Pin) => Self::None,
            (Self::CopySave, LongshotOutputAction::Copy) => Self::SaveOnly,
            (Self::CopySave, LongshotOutputAction::Save) => Self::CopyOnly,
            (Self::CopyPin, LongshotOutputAction::Copy) => Self::PinOnly,
            (Self::CopyPin, LongshotOutputAction::Pin) => Self::CopyOnly,
            (Self::SavePin, LongshotOutputAction::Save) => Self::PinOnly,
            (Self::SavePin, LongshotOutputAction::Pin) => Self::SaveOnly,
            (Self::Any, LongshotOutputAction::Copy) => Self::SavePin,
            (Self::Any, LongshotOutputAction::Save) => Self::CopyPin,
            (Self::Any, LongshotOutputAction::Pin) => Self::CopySave,
            (policy, _) => policy,
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OutputWorkerError {
    Business(String),
    /// 业务线程已返回，但动作是否已经生效无法确定。
    Uncertain(String),
    Join(String),
}

/// 输出动作的成功值。`SavePath` 与 `PinLabel` 不能共用 `Option<String>`，
/// 否则重试状态机无法证明一个返回值确实属于所请求的动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OutputValue {
    None,
    SavePath(String),
    PinLabel(String),
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
        O: FnOnce(LongshotOutputAction, Arc<LongshotOutputArtifact>) -> OF,
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
                artifact,
                retry_policy,
            } if current == label && token == wire_token => {
                if !retry_policy.allows(action) {
                    *slot = Slot::OutputPending {
                        label: current,
                        token,
                        snapshot,
                        artifact,
                        retry_policy,
                    };
                    return Err(match action {
                        LongshotOutputAction::Copy => {
                            LongshotIpcError::cleanup_failed("长截图复制重试权限已经丢失")
                        }
                        LongshotOutputAction::Save => LongshotIpcError::save_uncertain(
                            "上次保存结果不确定，禁止再次保存以免生成重复文件",
                        ),
                        LongshotOutputAction::Pin => LongshotIpcError::pin_uncertain(
                            "上次贴图结果不确定，禁止再次贴图以免创建重复窗口",
                        ),
                    });
                }
                *slot = Slot::Finishing {
                    label: current,
                    token: token.clone(),
                    snapshot,
                    stage: FinishStage::Outputting {
                        action,
                        artifact: Arc::clone(&artifact),
                        retry_policy,
                    },
                    window_destroyed: false,
                };
                Ok(FinishClaim::Outputting(token, artifact))
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
                artifact,
                retry_policy,
            } if current == label => {
                *slot = Slot::OutputPending {
                    label: current,
                    token,
                    snapshot,
                    artifact,
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
        artifact: Arc<LongshotOutputArtifact>,
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
                        artifact,
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
        artifact: &Arc<LongshotOutputArtifact>,
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
                        artifact: current_artifact,
                        retry_policy,
                    },
                window_destroyed,
            } if current == label
                && current_token == *token
                && current_action == action
                && Arc::ptr_eq(&current_artifact, artifact) =>
            {
                if window_destroyed {
                    *slot = Slot::Empty;
                    OutputFailureAction::Dropped
                } else {
                    let retry_policy = if uncertain
                        && matches!(
                            action,
                            LongshotOutputAction::Save | LongshotOutputAction::Pin
                        ) {
                        retry_policy.without(action)
                    } else {
                        retry_policy
                    };
                    *slot = Slot::OutputPending {
                        label: current,
                        token: current_token,
                        snapshot,
                        artifact: current_artifact,
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

    pub(super) fn complete_output_success(
        &self,
        label: &str,
        token: &LongshotSessionToken,
        action: LongshotOutputAction,
        artifact: &Arc<LongshotOutputArtifact>,
    ) -> bool {
        let Ok(mut slot) = self.slot.lock() else {
            return false;
        };
        let exact = matches!(&*slot, Slot::Finishing {
            label: current,
            token: current_token,
            stage: FinishStage::Outputting {
                action: current_action,
                artifact: current_artifact,
                ..
            },
            ..
        } if current == label
            && current_token == token
            && *current_action == action
            && Arc::ptr_eq(current_artifact, artifact));
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
    WF: std::future::Future<Output = Result<LongshotArtifact, FinishWorkerError>>,
    P: FnOnce(&LongshotSessionToken) -> bool,
    O: FnOnce(LongshotOutputAction, Arc<LongshotOutputArtifact>) -> OF,
    OF: std::future::Future<Output = Result<OutputValue, OutputWorkerError>>,
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

    let artifact = match claim {
        FinishClaim::Encoding(token) => match finish_worker(token.clone()).await {
            Ok(lifecycle_artifact) => {
                boundary(FinishBoundary::AfterFinish);
                let artifact = Arc::new(LongshotOutputArtifact::from(lifecycle_artifact));
                registry.publish_outputting(label, &token, action, Arc::clone(&artifact))?;
                artifact
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
        FinishClaim::Outputting(_, artifact) => artifact,
    };

    boundary(FinishBoundary::BeforeOutput);
    let output_result = match (action, output_worker(action, Arc::clone(&artifact)).await) {
        (LongshotOutputAction::Copy, Ok(OutputValue::None)) => Ok((None, None)),
        (LongshotOutputAction::Save, Ok(OutputValue::SavePath(path))) if !path.is_empty() => {
            Ok((Some(path), None))
        }
        (LongshotOutputAction::Pin, Ok(OutputValue::PinLabel(label))) if !label.is_empty() => {
            Ok((None, Some(label)))
        }
        (LongshotOutputAction::Copy, Ok(_)) => Err(OutputWorkerError::Business(
            "长截图复制成功却返回了不匹配的输出值".to_string(),
        )),
        (LongshotOutputAction::Save, Ok(_)) => Err(OutputWorkerError::Business(
            "长截图保存成功但未返回有效路径".to_string(),
        )),
        (LongshotOutputAction::Pin, Ok(_)) => Err(OutputWorkerError::Business(
            "长截图贴图成功但未返回有效窗口标签".to_string(),
        )),
        (_, Err(error)) => Err(error),
    };
    boundary(FinishBoundary::AfterOutput);
    let (path, pin_label) = match output_result {
        Ok(value) => value,
        Err(error) => {
            let uncertain = matches!(
                error,
                OutputWorkerError::Uncertain(_) | OutputWorkerError::Join(_)
            ) && matches!(
                action,
                LongshotOutputAction::Save | LongshotOutputAction::Pin
            );
            let message = match error {
                OutputWorkerError::Business(message)
                | OutputWorkerError::Uncertain(message)
                | OutputWorkerError::Join(message) => message,
            };
            return match registry
                .complete_output_failure(label, &token, action, &artifact, uncertain)
            {
                OutputFailureAction::Pending | OutputFailureAction::Dropped => Err(match action {
                    LongshotOutputAction::Copy => LongshotIpcError::copy_failed(message),
                    LongshotOutputAction::Save if uncertain => {
                        LongshotIpcError::save_uncertain(message)
                    }
                    LongshotOutputAction::Save => LongshotIpcError::save_failed(message),
                    LongshotOutputAction::Pin if uncertain => {
                        LongshotIpcError::pin_uncertain(message)
                    }
                    LongshotOutputAction::Pin => LongshotIpcError::pin_failed(message),
                }),
                OutputFailureAction::OwnershipLost => Err(LongshotIpcError::cleanup_failed(
                    "长截图输出失败后控制权已经丢失",
                )),
            };
        }
    };

    boundary(FinishBoundary::BeforeCommit);
    if !registry.complete_output_success(label, &token, action, &artifact) {
        return Err(LongshotIpcError::cleanup_failed(
            "长截图输出成功后控制权已经丢失",
        ));
    }
    windows.destroy(label);
    Ok(LongshotOutputResult {
        action,
        path,
        pin_label,
    })
}

pub(super) async fn run_finish_worker<F>(work: F) -> Result<LongshotArtifact, FinishWorkerError>
where
    F: FnOnce() -> Result<LongshotArtifact, CaptureError> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(work).await {
        Ok(Ok(artifact)) => Ok(artifact),
        Ok(Err(error)) => Err(FinishWorkerError::Business(error)),
        Err(error) => Err(FinishWorkerError::Join(format!(
            "长截图完成线程异常: {error}"
        ))),
    }
}

pub(super) async fn run_output_worker<F>(
    action: LongshotOutputAction,
    work: F,
) -> Result<OutputValue, OutputWorkerError>
where
    F: FnOnce() -> Result<OutputValue, String> + Send + 'static,
{
    if action == LongshotOutputAction::Pin {
        return Err(OutputWorkerError::Business(
            "长截图贴图必须在 Tauri 控制路径执行".to_string(),
        ));
    }
    match tauri::async_runtime::spawn_blocking(work).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(OutputWorkerError::Business(error)),
        Err(error) => Err(OutputWorkerError::Join(match action {
            LongshotOutputAction::Copy => format!("长截图复制线程异常: {error}"),
            LongshotOutputAction::Save => format!("长截图保存线程异常: {error}"),
            LongshotOutputAction::Pin => unreachable!("贴图不会进入输出工作线程"),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIONS: [LongshotOutputAction; 3] = [
        LongshotOutputAction::Copy,
        LongshotOutputAction::Save,
        LongshotOutputAction::Pin,
    ];

    #[test]
    fn removing_an_action_never_restores_another_permission() {
        let policies = [
            RetryPolicy::None,
            RetryPolicy::CopyOnly,
            RetryPolicy::SaveOnly,
            RetryPolicy::PinOnly,
            RetryPolicy::CopySave,
            RetryPolicy::CopyPin,
            RetryPolicy::SavePin,
            RetryPolicy::Any,
        ];
        for policy in policies {
            for removed in ACTIONS {
                let narrowed = policy.without(removed);
                assert!(!narrowed.allows(removed));
                for action in ACTIONS {
                    assert!(
                        !narrowed.allows(action) || policy.allows(action),
                        "{policy:?} - {removed:?} unexpectedly restored {action:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn save_and_pin_uncertainty_commute_to_copy_only() {
        let save_then_pin = RetryPolicy::Any
            .without(LongshotOutputAction::Save)
            .without(LongshotOutputAction::Pin);
        let pin_then_save = RetryPolicy::Any
            .without(LongshotOutputAction::Pin)
            .without(LongshotOutputAction::Save);
        assert_eq!(save_then_pin, RetryPolicy::CopyOnly);
        assert_eq!(pin_then_save, RetryPolicy::CopyOnly);
    }
}
