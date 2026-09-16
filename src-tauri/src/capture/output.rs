//! 普通截图输出的认领身份与可恢复产物。
use super::{CaptureAction, CommitImage};
use crate::pin::PinOrigin;
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OutputPhase {
    Rendering,
    Executing,
    Failed,
}

#[derive(Debug)]
pub(super) struct OutputAttempt {
    pub token: Arc<()>,
    pub caller: String,
    pub phase: OutputPhase,
    pub artifact: Option<Arc<CommitImage>>,
    pub origin: Option<PinOrigin>,
    pub copy_only: bool,
    pub abandoned: bool,
}

#[derive(Debug, Clone)]
pub(super) struct OutputClaim {
    pub session_id: String,
    pub token: Arc<()>,
    pub artifact: Option<Arc<CommitImage>>,
    pub origin: Option<PinOrigin>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureOutputError {
    pub message: String,
    pub output_pending: bool,
    pub retry_actions: Vec<CaptureAction>,
}
impl CaptureOutputError {
    pub(super) fn editing(message: impl ToString) -> Self {
        Self {
            message: message.to_string(),
            output_pending: false,
            retry_actions: Vec::new(),
        }
    }
    pub(super) fn pending(message: impl ToString, copy_only: bool) -> Self {
        Self {
            message: message.to_string(),
            output_pending: true,
            retry_actions: if copy_only {
                vec![CaptureAction::Copy]
            } else {
                vec![CaptureAction::Copy, CaptureAction::Save, CaptureAction::Pin]
            },
        }
    }
}
impl From<super::CaptureError> for CaptureOutputError {
    fn from(value: super::CaptureError) -> Self {
        Self::editing(value)
    }
}

pub(super) struct OutputFailure {
    pub message: String,
    pub uncertain: bool,
}
impl From<String> for OutputFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            uncertain: false,
        }
    }
}
