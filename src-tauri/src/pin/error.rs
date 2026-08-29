//! 贴图领域错误。
//!
//! `Display` 文案与结构化前的字符串保持一致，command 层继续把错误转成 String 返回前端；
//! `code()` 提供稳定分类，便于日志聚合与区分"用户可重试"与"窗口已消失"。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PinError {
    #[error("贴图已经存在")]
    AlreadyExists,
    #[error("贴图不存在或已经关闭")]
    EntryMissing,
    #[error("贴图窗口不存在")]
    WindowMissing,
    #[error("贴图窗口状态不完整，请重试")]
    StateIncomplete,
    #[error("文本贴图不能保存或编辑为图片")]
    TextNotImage,
    /// PinManager 的 Mutex 被 poison，属于不可恢复状态。
    #[error("{0}")]
    StateLock(String),
    /// Tauri 窗口操作失败（创建、显示、缩放、定位、关闭）。
    #[error("{0}")]
    Window(String),
}

impl PinError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::AlreadyExists => "already_exists",
            Self::EntryMissing => "entry_missing",
            Self::WindowMissing => "window_missing",
            Self::StateIncomplete => "state_incomplete",
            Self::TextNotImage => "text_not_image",
            Self::StateLock(_) => "state_lock",
            Self::Window(_) => "window",
        }
    }

    /// 贴图或其窗口已经不在了。调用方据此静默收敛，而不是向用户报错。
    pub fn is_gone(&self) -> bool {
        matches!(self, Self::EntryMissing | Self::WindowMissing)
    }

    pub(super) fn state_lock(error: impl std::fmt::Display) -> Self {
        Self::StateLock(error.to_string())
    }

    pub(super) fn window(error: impl std::fmt::Display) -> Self {
        Self::Window(error.to_string())
    }
}

/// IPC 边界对前端返回 String。转换集中在此处，command 层用 `?` 即可，
/// 不需要每个调用点手写 `map_err`，也保证对外文案只有一个来源。
impl From<PinError> for String {
    fn from(error: PinError) -> Self {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_matches_pre_refactor_messages() {
        assert_eq!(PinError::AlreadyExists.to_string(), "贴图已经存在");
        assert_eq!(PinError::EntryMissing.to_string(), "贴图不存在或已经关闭");
        assert_eq!(PinError::WindowMissing.to_string(), "贴图窗口不存在");
        assert_eq!(
            PinError::StateIncomplete.to_string(),
            "贴图窗口状态不完整，请重试"
        );
        assert_eq!(
            PinError::TextNotImage.to_string(),
            "文本贴图不能保存或编辑为图片"
        );
        assert_eq!(PinError::window("boom").to_string(), "boom");
    }

    #[test]
    fn codes_are_stable_and_unique() {
        let errors = [
            PinError::AlreadyExists,
            PinError::EntryMissing,
            PinError::WindowMissing,
            PinError::StateIncomplete,
            PinError::TextNotImage,
            PinError::StateLock(String::new()),
            PinError::Window(String::new()),
        ];
        let mut codes: Vec<&str> = errors.iter().map(PinError::code).collect();
        let total = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), total, "错误码必须唯一");
    }

    #[test]
    fn only_missing_targets_count_as_gone() {
        assert!(PinError::EntryMissing.is_gone());
        assert!(PinError::WindowMissing.is_gone());
        assert!(!PinError::AlreadyExists.is_gone());
        assert!(!PinError::Window(String::new()).is_gone());
    }
}
