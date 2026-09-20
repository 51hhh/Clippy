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
    #[error("截图会话用途与请求不匹配")]
    CaptureIntentMismatch,
    #[error("长截图帧为空")]
    LongshotFrameEmpty,
    #[error("长截图尚未追加任何帧")]
    LongshotEmpty,
    #[error("长截图首帧不能包含重叠行")]
    LongshotFirstFrameOverlap,
    #[error("长截图重叠行无效")]
    LongshotOverlapInvalid,
    #[error("长截图帧宽度与首帧不一致")]
    LongshotWidthMismatch,
    #[error("长截图帧数超过上限")]
    LongshotFrameLimit,
    #[error("长截图尺寸或缓冲超过资源上限")]
    LongshotResourceLimit,
    #[error("长截图内存分配失败")]
    LongshotAllocationFailed,
    #[error("长截图帧元数据或像素缓冲无效")]
    LongshotFrameInvalid,
    #[error("长截图连续帧几何发生变化")]
    LongshotFrameGeometryChanged,
    #[error("长截图重捕获缺少目标显示器帧")]
    LongshotRecaptureMonitorMissing,
    #[error("长截图相邻帧尺寸不一致")]
    LongshotEstimateSizeMismatch,
    #[error("长截图相邻帧尺寸过小")]
    LongshotEstimateTooSmall,
    #[error("长截图相邻帧纹理不足")]
    LongshotEstimateLowTexture,
    #[error("长截图相邻帧相似度不足")]
    LongshotEstimateLowSimilarity,
    #[error("长截图相邻帧匹配存在歧义")]
    LongshotEstimateAmbiguous,
    #[error("长截图相邻帧位移超过上限")]
    LongshotEstimateDisplacementTooLarge,
    #[error("长截图相邻帧没有新增内容")]
    LongshotEstimateNoExtension,
    #[error("已有长截图会话正在进行")]
    LongshotSessionBusy,
    #[error("长截图会话不存在")]
    LongshotSessionMissing,
    #[error("长截图会话已经更新")]
    LongshotSessionSuperseded,
    #[error("长截图会话代次已耗尽")]
    LongshotGenerationExhausted,
    #[error("当前平台不支持自动滚动长截图")]
    LongshotAutoUnsupported,
    #[error("自动滚动没有命中原目标窗口")]
    LongshotAutoTargetLost,
    #[error("检测到用户移动鼠标，已暂停自动滚动")]
    LongshotAutoUserInterrupted,
    #[error("自动滚动输入失败: {0}")]
    LongshotAutoInput(String),
    #[error("已有截图模式正在进行")]
    CaptureModeBusy,
    #[error("截图模式所有权已经更新")]
    CaptureModeSuperseded,
    #[error("截图模式代次已耗尽")]
    CaptureModeGenerationExhausted,
    #[error("创建截图覆盖层失败: {0}")]
    OverlayCreate(String),
    #[error("截图失败: {0}")]
    Screenshot(String),
    #[error("截图线程异常: {0}")]
    ThreadPanic(String),
    /// 截图领域状态 Mutex 被 poison，属于不可恢复状态。
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
            Self::CaptureIntentMismatch => "capture_intent_mismatch",
            Self::LongshotFrameEmpty => "longshot_frame_empty",
            Self::LongshotEmpty => "longshot_empty",
            Self::LongshotFirstFrameOverlap => "longshot_first_frame_overlap",
            Self::LongshotOverlapInvalid => "longshot_overlap_invalid",
            Self::LongshotWidthMismatch => "longshot_width_mismatch",
            Self::LongshotFrameLimit => "longshot_frame_limit",
            Self::LongshotResourceLimit => "longshot_resource_limit",
            Self::LongshotAllocationFailed => "longshot_allocation_failed",
            Self::LongshotFrameInvalid => "longshot_frame_invalid",
            Self::LongshotFrameGeometryChanged => "longshot_frame_geometry_changed",
            Self::LongshotRecaptureMonitorMissing => "longshot_recapture_monitor_missing",
            Self::LongshotEstimateSizeMismatch => "longshot_estimate_size_mismatch",
            Self::LongshotEstimateTooSmall => "longshot_estimate_too_small",
            Self::LongshotEstimateLowTexture => "longshot_estimate_low_texture",
            Self::LongshotEstimateLowSimilarity => "longshot_estimate_low_similarity",
            Self::LongshotEstimateAmbiguous => "longshot_estimate_ambiguous",
            Self::LongshotEstimateDisplacementTooLarge => {
                "longshot_estimate_displacement_too_large"
            }
            Self::LongshotEstimateNoExtension => "longshot_estimate_no_extension",
            Self::LongshotSessionBusy => "longshot_session_busy",
            Self::LongshotSessionMissing => "longshot_session_missing",
            Self::LongshotSessionSuperseded => "longshot_session_superseded",
            Self::LongshotGenerationExhausted => "longshot_generation_exhausted",
            Self::LongshotAutoUnsupported => "longshot_auto_unsupported",
            Self::LongshotAutoTargetLost => "longshot_auto_target_lost",
            Self::LongshotAutoUserInterrupted => "longshot_auto_user_interrupted",
            Self::LongshotAutoInput(_) => "longshot_auto_input",
            Self::CaptureModeBusy => "capture_mode_busy",
            Self::CaptureModeSuperseded => "capture_mode_superseded",
            Self::CaptureModeGenerationExhausted => "capture_mode_generation_exhausted",
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
        assert_eq!(
            CaptureError::CaptureModeBusy.to_string(),
            "已有截图模式正在进行"
        );
        assert_eq!(
            CaptureError::CaptureModeSuperseded.to_string(),
            "截图模式所有权已经更新"
        );
        assert_eq!(
            CaptureError::CaptureModeGenerationExhausted.to_string(),
            "截图模式代次已耗尽"
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
            CaptureError::CaptureIntentMismatch,
            CaptureError::LongshotFrameEmpty,
            CaptureError::LongshotEmpty,
            CaptureError::LongshotFirstFrameOverlap,
            CaptureError::LongshotOverlapInvalid,
            CaptureError::LongshotWidthMismatch,
            CaptureError::LongshotFrameLimit,
            CaptureError::LongshotResourceLimit,
            CaptureError::LongshotAllocationFailed,
            CaptureError::LongshotFrameInvalid,
            CaptureError::LongshotFrameGeometryChanged,
            CaptureError::LongshotRecaptureMonitorMissing,
            CaptureError::LongshotEstimateSizeMismatch,
            CaptureError::LongshotEstimateTooSmall,
            CaptureError::LongshotEstimateLowTexture,
            CaptureError::LongshotEstimateLowSimilarity,
            CaptureError::LongshotEstimateAmbiguous,
            CaptureError::LongshotEstimateDisplacementTooLarge,
            CaptureError::LongshotEstimateNoExtension,
            CaptureError::LongshotSessionBusy,
            CaptureError::LongshotSessionMissing,
            CaptureError::LongshotSessionSuperseded,
            CaptureError::LongshotGenerationExhausted,
            CaptureError::CaptureModeBusy,
            CaptureError::CaptureModeSuperseded,
            CaptureError::CaptureModeGenerationExhausted,
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
