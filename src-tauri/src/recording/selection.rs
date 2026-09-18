//! 冻结截图选区到录屏平台帧源的可信交接。
//!
//! 前端只提交普通 `CaptureSelection`；物理 crop 由仍然存活的截图会话计算。平台帧源建好之后才消费
//! 候选并把 Ordinary 所有权转换为 Recording，任一步失败都保留原截图会话供用户重试。

use crate::capture::{
    CaptureError, CaptureManager, CaptureRecordingCandidate, CaptureRecordingHandoff,
    CaptureSelection, RecordingCaptureSpec,
};

pub(super) struct PreparedRecordingSelection {
    candidate: CaptureRecordingCandidate,
}

impl PreparedRecordingSelection {
    pub(super) fn prepare(
        capture: &CaptureManager,
        caller: &str,
        selection: &CaptureSelection,
    ) -> Result<Self, CaptureError> {
        Ok(Self {
            candidate: capture.prepare_recording(caller, selection)?,
        })
    }

    pub(super) fn spec(&self) -> RecordingCaptureSpec {
        self.candidate.spec()
    }

    pub(super) fn commit(
        self,
        capture: &CaptureManager,
    ) -> Result<CaptureRecordingHandoff, CaptureError> {
        capture.commit_recording(self.candidate)
    }
}
