//! 跨领域错误聚合。
//!
//! 每个领域（storage / translation / paste / pin / capture）都有自己的错误类型和稳定
//! `code()`，但同一个 code 在不同领域里可能重名（例如 pin 与 capture 都有 `window`）。
//! 日志和排障需要的是"哪个领域的哪个错误"，`ClippyError` 只负责补上这个领域维度：
//! 通过 `From` 收集领域错误，暴露 `domain()` / `code()` / `identifier()`。
//!
//! IPC 边界仍然对前端返回 `String`，所以这里同样提供 `From<ClippyError> for String`。

use crate::capture::CaptureError;
use crate::paste::PasteError;
use crate::pin::PinError;
use crate::storage::StorageError;
use crate::translation::types::TranslationError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClippyError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Translation(#[from] TranslationError),
    #[error(transparent)]
    Paste(#[from] PasteError),
    #[error(transparent)]
    Pin(#[from] PinError),
    #[error(transparent)]
    Capture(#[from] CaptureError),
}

impl ClippyError {
    pub fn domain(&self) -> &'static str {
        match self {
            Self::Storage(_) => "storage",
            Self::Translation(_) => "translation",
            Self::Paste(_) => "paste",
            Self::Pin(_) => "pin",
            Self::Capture(_) => "capture",
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Storage(error) => error.code(),
            Self::Translation(error) => error.code(),
            Self::Paste(error) => error.code(),
            Self::Pin(error) => error.code(),
            Self::Capture(error) => error.code(),
        }
    }

    /// 日志与遥测里使用的稳定标识，形如 `paste.portal_start_rejected`。
    pub fn identifier(&self) -> String {
        format!("{}.{}", self.domain(), self.code())
    }
}

impl From<ClippyError> for String {
    fn from(error: ClippyError) -> Self {
        error.to_string()
    }
}

/// 记录一次真实故障，并返回可以交给前端的文案（不需要时可以忽略返回值）。
pub(crate) fn report(context: &str, error: impl Into<ClippyError>) -> String {
    log(log::Level::Warn, context, error.into())
}

/// 记录一次预期内的结果。Wayland 首次未授权、请求被新请求取代之类的路径不是故障，
/// 落到 warn 级只会淹没真正的告警。
pub(crate) fn note(context: &str, error: impl Into<ClippyError>) -> String {
    log(log::Level::Info, context, error.into())
}

fn log(level: log::Level, context: &str, error: ClippyError) -> String {
    log::log!(level, "{context}[{}]: {error}", error.identifier());
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_keeps_same_named_codes_apart_across_domains() {
        // pin 与 capture 都有 `window`，只有带上领域前缀才能在日志里区分。
        let pin: ClippyError = PinError::Window("boom".to_string()).into();
        let capture: ClippyError = CaptureError::Window("boom".to_string()).into();
        assert_eq!(pin.identifier(), "pin.window");
        assert_eq!(capture.identifier(), "capture.window");
        assert_ne!(pin.identifier(), capture.identifier());
    }

    #[test]
    fn display_stays_transparent_so_ipc_text_is_unchanged() {
        let error: ClippyError = CaptureError::SelectionTooSmall.into();
        assert_eq!(error.to_string(), "截图选择区域太小");
        assert_eq!(String::from(error), "截图选择区域太小");
    }

    #[test]
    fn every_domain_reports_its_own_name() {
        let errors: [ClippyError; 5] = [
            StorageError::Io(std::io::Error::other("boom")).into(),
            TranslationError::Internal.into(),
            PasteError::PortalSessionMissing.into(),
            PinError::EntryMissing.into(),
            CaptureError::SessionMissing.into(),
        ];
        let domains: Vec<&str> = errors.iter().map(ClippyError::domain).collect();
        assert_eq!(
            domains,
            ["storage", "translation", "paste", "pin", "capture"]
        );
    }
}
