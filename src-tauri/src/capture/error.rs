//! 截图领域错误。
//!
//! `Display` 文案与结构化前的字符串保持一致，command 层继续把错误转成 String 返回前端；
//! `code()` 提供稳定分类，便于区分"会话已失效"（静默收敛）与真实故障（需要提示）。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("截图没有可用显示器帧")]
    NoMonitorFrames,
    #[error("已有截图会话正在进行")]
    SessionBusy,
    #[error("截图会话不存在")]
    SessionMissing,
    #[error("截图会话已经更新，请重新选择")]
    SessionSupersededRetry,
    #[error("截图会话已经更新")]
    SessionSuperseded,
    #[error("覆盖层不属于当前截图会话")]
    OverlayNotInSession,
    #[error("覆盖层帧不存在")]
    OverlayFrameMissing,
    #[error("无效的截图覆盖层标签")]
    OverlayLabelInvalid,
    #[error("选择区域不属于当前显示器")]
    SelectionMonitorMismatch,
    #[error("截图选择区域包含无效数值")]
    SelectionNotFinite,
    #[error("截图选择区域太小")]
    SelectionTooSmall,
    #[error("截图选择区域为空")]
    SelectionEmpty,
    #[error("截图帧裁剪越界")]
    CropOutOfBounds,
    #[error("提交的截图数据无效")]
    CommitPayloadInvalid,
    #[error("提交的截图数据过大")]
    CommitPayloadTooLarge,
    #[error("创建截图覆盖层失败: {0}")]
    OverlayCreate(String),
    #[error("截图失败: {0}")]
    Screenshot(String),
    #[error("截图线程异常: {0}")]
    ThreadPanic(String),
    /// CaptureManager 的 Mutex 被 poison，属于不可恢复状态。
    #[error("{0}")]
    StateLock(String),
    /// PNG 编解码失败。
    #[error("{0}")]
    Codec(String),
    /// Tauri 覆盖层窗口操作失败（定位、缩放、显示）。
    #[error("{0}")]
    Window(String),
}

impl CaptureError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoMonitorFrames => "no_monitor_frames",
            Self::SessionBusy => "session_busy",
            Self::SessionMissing => "session_missing",
            Self::SessionSupersededRetry => "session_superseded_retry",
            Self::SessionSuperseded => "session_superseded",
            Self::OverlayNotInSession => "overlay_not_in_session",
            Self::OverlayFrameMissing => "overlay_frame_missing",
            Self::OverlayLabelInvalid => "overlay_label_invalid",
            Self::SelectionMonitorMismatch => "selection_monitor_mismatch",
            Self::SelectionNotFinite => "selection_not_finite",
            Self::SelectionTooSmall => "selection_too_small",
            Self::SelectionEmpty => "selection_empty",
            Self::CropOutOfBounds => "crop_out_of_bounds",
            Self::CommitPayloadInvalid => "commit_payload_invalid",
            Self::CommitPayloadTooLarge => "commit_payload_too_large",
            Self::OverlayCreate(_) => "overlay_create",
            Self::Screenshot(_) => "screenshot",
            Self::ThreadPanic(_) => "thread_panic",
            Self::StateLock(_) => "state_lock",
            Self::Codec(_) => "codec",
            Self::Window(_) => "window",
        }
    }

    pub(super) fn state_lock(error: impl std::fmt::Display) -> Self {
        Self::StateLock(error.to_string())
    }

    pub(super) fn codec(error: impl std::fmt::Display) -> Self {
        Self::Codec(error.to_string())
    }

    pub(super) fn window(error: impl std::fmt::Display) -> Self {
        Self::Window(error.to_string())
    }
}

/// IPC 边界对前端返回 String。转换集中在此处，command 层用 `?` 即可。
impl From<CaptureError> for String {
    fn from(error: CaptureError) -> Self {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_matches_pre_refactor_messages() {
        assert_eq!(
            CaptureError::NoMonitorFrames.to_string(),
            "截图没有可用显示器帧"
        );
        assert_eq!(
            CaptureError::SessionBusy.to_string(),
            "已有截图会话正在进行"
        );
        assert_eq!(CaptureError::SessionMissing.to_string(), "截图会话不存在");
        assert_eq!(
            CaptureError::SessionSupersededRetry.to_string(),
            "截图会话已经更新，请重新选择"
        );
        assert_eq!(
            CaptureError::SessionSuperseded.to_string(),
            "截图会话已经更新"
        );
        assert_eq!(
            CaptureError::OverlayLabelInvalid.to_string(),
            "无效的截图覆盖层标签"
        );
        assert_eq!(
            CaptureError::OverlayCreate("boom".to_string()).to_string(),
            "创建截图覆盖层失败: boom"
        );
        assert_eq!(
            CaptureError::Screenshot("boom".to_string()).to_string(),
            "截图失败: boom"
        );
        assert_eq!(
            CaptureError::ThreadPanic("boom".to_string()).to_string(),
            "截图线程异常: boom"
        );
    }

    #[test]
    fn codes_are_stable_and_unique() {
        let errors = [
            CaptureError::NoMonitorFrames,
            CaptureError::SessionBusy,
            CaptureError::SessionMissing,
            CaptureError::SessionSupersededRetry,
            CaptureError::SessionSuperseded,
            CaptureError::OverlayNotInSession,
            CaptureError::OverlayFrameMissing,
            CaptureError::OverlayLabelInvalid,
            CaptureError::SelectionMonitorMismatch,
            CaptureError::SelectionNotFinite,
            CaptureError::SelectionTooSmall,
            CaptureError::SelectionEmpty,
            CaptureError::CropOutOfBounds,
            CaptureError::CommitPayloadInvalid,
            CaptureError::CommitPayloadTooLarge,
            CaptureError::OverlayCreate(String::new()),
            CaptureError::Screenshot(String::new()),
            CaptureError::ThreadPanic(String::new()),
            CaptureError::StateLock(String::new()),
            CaptureError::Codec(String::new()),
            CaptureError::Window(String::new()),
        ];
        let mut codes: Vec<&str> = errors.iter().map(CaptureError::code).collect();
        let total = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), total, "错误码必须唯一");
    }
}
