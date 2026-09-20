//! 录屏控制窗身份与会话 generation token 的单槽绑定。
//!
//! 控制窗只携带后端生成的窗口标签；暂停、继续、停止和销毁处理都从这里取得 exact token。前端不
//! 能提交 session ID 或 generation，从而避免迟到窗口控制随后建立的新录屏。

use super::manager::RecordingToken;
use std::sync::Mutex;
use thiserror::Error;

const CONTROL_PREFIX: &str = "recording-control-";

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub(super) enum RecordingControlRegistryError {
    #[error("已有录屏控制窗正在创建、使用或关闭")]
    Busy,
    #[error("录屏控制窗不存在")]
    Missing,
    #[error("录屏控制窗已经更新")]
    Superseded,
    #[error("录屏控制窗 registry 锁已损坏")]
    Poisoned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordingControlClose {
    pub label: String,
    pub token: Option<RecordingToken>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordingControlBinding {
    pub label: String,
    /// 页面 ready 先于 token 到达时，由 bind 一次性认领 reveal。
    pub reveal: bool,
}

#[derive(Default)]
pub(super) struct RecordingControlRegistry {
    slot: Mutex<ControlSlot>,
}

#[derive(Default)]
enum ControlSlot {
    #[default]
    Empty,
    Preparing {
        label: String,
        session_id: String,
        ready: bool,
    },
    Bound {
        label: String,
        token: RecordingToken,
        ready: bool,
        revealed: bool,
    },
    Closing {
        label: String,
        token: Option<RecordingToken>,
    },
    TerminalFailed,
}

impl RecordingControlRegistry {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn reserve(
        &self,
        session_id: &str,
    ) -> Result<String, RecordingControlRegistryError> {
        let mut slot = self
            .slot
            .lock()
            .map_err(|_| RecordingControlRegistryError::Poisoned)?;
        if !matches!(*slot, ControlSlot::Empty) {
            return Err(RecordingControlRegistryError::Busy);
        }
        let label = format!("{CONTROL_PREFIX}{}", crate::image_io::unique_image_id());
        *slot = ControlSlot::Preparing {
            label: label.clone(),
            session_id: session_id.to_string(),
            ready: false,
        };
        Ok(label)
    }

    /// 控制窗已安全创建后，把 manager 返回的不可复用 token 原子绑定到该窗。
    pub(super) fn bind(
        &self,
        token: &RecordingToken,
    ) -> Result<RecordingControlBinding, RecordingControlRegistryError> {
        let mut slot = self
            .slot
            .lock()
            .map_err(|_| RecordingControlRegistryError::Poisoned)?;
        let previous = std::mem::take(&mut *slot);
        match previous {
            ControlSlot::Preparing {
                label,
                session_id,
                ready,
            } if session_id == token.session_id => {
                *slot = ControlSlot::Bound {
                    label: label.clone(),
                    token: token.clone(),
                    ready,
                    revealed: ready,
                };
                Ok(RecordingControlBinding {
                    label,
                    reveal: ready,
                })
            }
            other => {
                *slot = other;
                Err(RecordingControlRegistryError::Superseded)
            }
        }
    }

    /// 页面和 token 任一方都可以先到；只有第二个条件到达的调用取得一次 reveal 责任。
    pub(super) fn mark_ready(
        &self,
        caller_label: &str,
    ) -> Result<bool, RecordingControlRegistryError> {
        if !caller_label.starts_with(CONTROL_PREFIX) {
            return Err(RecordingControlRegistryError::Missing);
        }
        let mut slot = self
            .slot
            .lock()
            .map_err(|_| RecordingControlRegistryError::Poisoned)?;
        match &mut *slot {
            ControlSlot::Preparing { label, ready, .. } if label == caller_label => {
                *ready = true;
                Ok(false)
            }
            ControlSlot::Bound {
                label,
                ready,
                revealed,
                ..
            } if label == caller_label => {
                *ready = true;
                if *revealed {
                    Ok(false)
                } else {
                    *revealed = true;
                    Ok(true)
                }
            }
            ControlSlot::Closing { label, .. } if label == caller_label => {
                Err(RecordingControlRegistryError::Busy)
            }
            ControlSlot::Empty => Err(RecordingControlRegistryError::Missing),
            _ => Err(RecordingControlRegistryError::Superseded),
        }
    }

    /// IPC command 只用注入的 caller label 查询，不接受前端提供的 token。
    pub(super) fn token_for_caller(
        &self,
        caller_label: &str,
    ) -> Result<RecordingToken, RecordingControlRegistryError> {
        if !caller_label.starts_with(CONTROL_PREFIX) {
            return Err(RecordingControlRegistryError::Missing);
        }
        let slot = self
            .slot
            .lock()
            .map_err(|_| RecordingControlRegistryError::Poisoned)?;
        match &*slot {
            ControlSlot::Bound { label, token, .. } if label == caller_label => Ok(token.clone()),
            ControlSlot::Preparing { label, .. } | ControlSlot::Closing { label, .. }
                if label == caller_label =>
            {
                Err(RecordingControlRegistryError::Busy)
            }
            ControlSlot::Empty => Err(RecordingControlRegistryError::Missing),
            _ => Err(RecordingControlRegistryError::Superseded),
        }
    }

    /// 生命周期主动结束时先 claim 关闭责任，再销毁窗口，最后 settle。
    pub(super) fn begin_close(
        &self,
        session_id: &str,
    ) -> Result<RecordingControlClose, RecordingControlRegistryError> {
        let mut slot = self
            .slot
            .lock()
            .map_err(|_| RecordingControlRegistryError::Poisoned)?;
        let previous = std::mem::take(&mut *slot);
        let close = match previous {
            ControlSlot::Preparing {
                label,
                session_id: current,
                ..
            } if current == session_id => RecordingControlClose { label, token: None },
            ControlSlot::Bound { label, token, .. } if token.session_id == session_id => {
                RecordingControlClose {
                    label,
                    token: Some(token),
                }
            }
            other => {
                *slot = other;
                return Err(RecordingControlRegistryError::Superseded);
            }
        };
        *slot = ControlSlot::Closing {
            label: close.label.clone(),
            token: close.token.clone(),
        };
        Ok(close)
    }

    /// 原生窗口先被用户或系统销毁时取得唯一清理责任；caller label 必须精确匹配。
    pub(super) fn claim_destroyed(
        &self,
        caller_label: &str,
    ) -> Result<RecordingControlClose, RecordingControlRegistryError> {
        let mut slot = self
            .slot
            .lock()
            .map_err(|_| RecordingControlRegistryError::Poisoned)?;
        let previous = std::mem::take(&mut *slot);
        let close = match previous {
            ControlSlot::Preparing { label, .. } if label == caller_label => {
                RecordingControlClose { label, token: None }
            }
            ControlSlot::Bound { label, token, .. } if label == caller_label => {
                RecordingControlClose {
                    label,
                    token: Some(token),
                }
            }
            ControlSlot::Closing { label, token } if label == caller_label => {
                *slot = ControlSlot::Closing { label, token };
                return Err(RecordingControlRegistryError::Busy);
            }
            other => {
                *slot = other;
                return Err(RecordingControlRegistryError::Superseded);
            }
        };
        *slot = ControlSlot::Closing {
            label: close.label.clone(),
            token: close.token.clone(),
        };
        Ok(close)
    }

    pub(super) fn settle_close(
        &self,
        label: &str,
        succeeded: bool,
    ) -> Result<(), RecordingControlRegistryError> {
        let mut slot = self
            .slot
            .lock()
            .map_err(|_| RecordingControlRegistryError::Poisoned)?;
        match &*slot {
            ControlSlot::Closing { label: current, .. } if current == label => {}
            _ => return Err(RecordingControlRegistryError::Superseded),
        }
        *slot = if succeeded {
            ControlSlot::Empty
        } else {
            ControlSlot::TerminalFailed
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(session_id: &str, generation: u64) -> RecordingToken {
        RecordingToken {
            session_id: session_id.to_string(),
            generation,
        }
    }

    #[test]
    fn caller_gets_only_the_exact_bound_generation() {
        let registry = RecordingControlRegistry::new();
        let label = registry.reserve("same-session").unwrap();
        assert_eq!(
            registry.token_for_caller(&label),
            Err(RecordingControlRegistryError::Busy)
        );
        let first = token("same-session", 7);
        assert_eq!(
            registry.bind(&first).unwrap(),
            RecordingControlBinding {
                label: label.clone(),
                reveal: false,
            }
        );
        assert_eq!(registry.token_for_caller(&label).unwrap(), first);
        assert_eq!(
            registry.token_for_caller("recording-control-forged"),
            Err(RecordingControlRegistryError::Superseded)
        );
        assert_eq!(
            registry.token_for_caller("capture-overlay-foreign"),
            Err(RecordingControlRegistryError::Missing)
        );
    }

    #[test]
    fn ready_then_bind_and_bind_then_ready_each_reveal_exactly_once() {
        let ready_first = RecordingControlRegistry::new();
        let ready_label = ready_first.reserve("ready-first").unwrap();
        assert!(!ready_first.mark_ready(&ready_label).unwrap());
        assert!(!ready_first.mark_ready(&ready_label).unwrap());
        assert_eq!(
            ready_first.bind(&token("ready-first", 1)).unwrap(),
            RecordingControlBinding {
                label: ready_label.clone(),
                reveal: true,
            }
        );
        assert!(!ready_first.mark_ready(&ready_label).unwrap());

        let bind_first = RecordingControlRegistry::new();
        let bind_label = bind_first.reserve("bind-first").unwrap();
        assert_eq!(
            bind_first.bind(&token("bind-first", 2)).unwrap(),
            RecordingControlBinding {
                label: bind_label.clone(),
                reveal: false,
            }
        );
        assert!(bind_first.mark_ready(&bind_label).unwrap());
        assert!(!bind_first.mark_ready(&bind_label).unwrap());
    }

    #[test]
    fn ready_rejects_forged_old_and_closing_windows() {
        let registry = RecordingControlRegistry::new();
        let label = registry.reserve("ready-guard").unwrap();
        assert_eq!(
            registry.mark_ready("capture-overlay-foreign"),
            Err(RecordingControlRegistryError::Missing)
        );
        assert_eq!(
            registry.mark_ready("recording-control-forged"),
            Err(RecordingControlRegistryError::Superseded)
        );
        registry.bind(&token("ready-guard", 1)).unwrap();
        registry.begin_close("ready-guard").unwrap();
        assert_eq!(
            registry.mark_ready(&label),
            Err(RecordingControlRegistryError::Busy)
        );
    }

    #[test]
    fn repeated_session_id_never_revives_old_window_label() {
        let registry = RecordingControlRegistry::new();
        let old_label = registry.reserve("repeated").unwrap();
        registry.bind(&token("repeated", 1)).unwrap();
        let close = registry.begin_close("repeated").unwrap();
        assert_eq!(close.label, old_label);
        registry.settle_close(&close.label, true).unwrap();

        let new_label = registry.reserve("repeated").unwrap();
        assert_ne!(new_label, old_label);
        let replacement = token("repeated", 2);
        registry.bind(&replacement).unwrap();
        assert_eq!(
            registry.token_for_caller(&old_label),
            Err(RecordingControlRegistryError::Superseded)
        );
        assert_eq!(registry.token_for_caller(&new_label).unwrap(), replacement);
    }

    #[test]
    fn late_bind_cannot_attach_to_replacement_reservation() {
        let registry = RecordingControlRegistry::new();
        let old_label = registry.reserve("old").unwrap();
        let close = registry.begin_close("old").unwrap();
        registry.settle_close(&close.label, true).unwrap();
        let new_label = registry.reserve("new").unwrap();

        assert_eq!(
            registry.bind(&token("old", 1)),
            Err(RecordingControlRegistryError::Superseded)
        );
        assert_eq!(
            registry.token_for_caller(&new_label),
            Err(RecordingControlRegistryError::Busy)
        );
        assert_ne!(old_label, new_label);
    }

    #[test]
    fn destroyed_bound_window_returns_token_once() {
        let registry = RecordingControlRegistry::new();
        let label = registry.reserve("destroyed").unwrap();
        let active = token("destroyed", 11);
        registry.bind(&active).unwrap();
        let close = registry.claim_destroyed(&label).unwrap();
        assert_eq!(close.token, Some(active));
        assert_eq!(
            registry.claim_destroyed(&label),
            Err(RecordingControlRegistryError::Busy)
        );
        registry.settle_close(&label, true).unwrap();
        assert_eq!(
            registry.token_for_caller(&label),
            Err(RecordingControlRegistryError::Missing)
        );
    }

    #[test]
    fn failed_window_close_blocks_replacement_until_process_recovery() {
        let registry = RecordingControlRegistry::new();
        let label = registry.reserve("failed-close").unwrap();
        registry.bind(&token("failed-close", 3)).unwrap();
        registry.begin_close("failed-close").unwrap();
        registry.settle_close(&label, false).unwrap();
        assert_eq!(
            registry.reserve("replacement"),
            Err(RecordingControlRegistryError::Busy)
        );
    }
}
