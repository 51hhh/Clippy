use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 翻译服务类型。字符串值是稳定的 IPC/config 合同，不能随意更改。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationProvider {
    #[serde(rename = "libretranslate")]
    LibreTranslate,
    #[serde(rename = "openai_compatible")]
    OpenAiCompatible,
}

impl TranslationProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LibreTranslate => "libretranslate",
            Self::OpenAiCompatible => "openai_compatible",
        }
    }
}

impl std::str::FromStr for TranslationProvider {
    type Err = TranslationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "libretranslate" | "libre_translate" => Ok(Self::LibreTranslate),
            "openai_compatible" | "openai-compatible" => Ok(Self::OpenAiCompatible),
            other => Err(TranslationError::UnsupportedProvider(other.to_string())),
        }
    }
}

/// 单次请求的服务参数。API key 刻意不放进此结构，避免被序列化或误记录。
#[derive(Clone, PartialEq, Eq)]
pub struct TranslationRequest {
    pub text: String,
    pub source_language: String,
    pub target_language: String,
    pub endpoint: String,
    pub provider: TranslationProvider,
    pub model: Option<String>,
    pub request_id: u64,
}

impl TranslationRequest {
    pub fn new(
        text: String,
        source_language: String,
        target_language: String,
        endpoint: String,
        provider: TranslationProvider,
        model: Option<String>,
        request_id: u64,
    ) -> Self {
        Self {
            text,
            source_language,
            target_language,
            endpoint,
            provider,
            model,
            request_id,
        }
    }
}

/// 返回给前端的翻译结果。request_id 用于丢弃并发请求的陈旧结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranslationResult {
    pub request_id: u64,
    pub provider: TranslationProvider,
    pub translated_text: String,
    pub detected_source_language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderTranslation {
    pub translated_text: String,
    pub detected_source_language: Option<String>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TranslationError {
    #[error("Translation input is empty")]
    EmptyInput,
    #[error("Translation input is too large")]
    InputTooLarge,
    #[error("Sensitive clipboard items cannot be sent to translation services")]
    SensitiveContent,
    #[error("A translation API key is required for this provider")]
    MissingApiKey,
    #[error("Secure credential storage is unavailable")]
    KeyringUnavailable,
    #[error("The clipboard item is unavailable")]
    ClipUnavailable,
    #[error("The clipboard image is unavailable")]
    ImageUnavailable,
    #[error("The capture selection is unavailable")]
    CaptureUnavailable,
    #[error("Local OCR could not extract text from the image")]
    OcrFailed,
    #[error("Invalid translation endpoint")]
    InvalidEndpoint,
    #[error("Unsupported translation provider: {0}")]
    UnsupportedProvider(String),
    #[error("Translation request timed out")]
    Timeout,
    #[error("Translation network request failed")]
    Network,
    #[error("Translation service returned HTTP {status}")]
    HttpStatus { status: u16 },
    #[error("Translation response exceeded the 1 MB limit")]
    ResponseTooLarge,
    #[error("Translation service returned an invalid response")]
    InvalidResponse,
    #[error("Translation request {request_id} was superseded by request {latest_request_id}")]
    StaleRequest {
        request_id: u64,
        latest_request_id: u64,
    },
    #[error("Translation is temporarily unavailable")]
    Internal,
}

impl TranslationError {
    pub(crate) fn retryable(&self) -> bool {
        matches!(self, Self::Network | Self::Timeout)
            || matches!(self, Self::HttpStatus { status } if *status >= 500)
    }

    /// IPC 错误码是前端可依赖的稳定合同；消息不得携带底层 I/O、DB 或密钥信息。
    pub fn ipc_message(&self) -> String {
        let message = match self {
            Self::EmptyInput => "Translation input is empty",
            Self::InputTooLarge => "Translation input is too large",
            Self::SensitiveContent => {
                "Sensitive clipboard items cannot be sent to translation services"
            }
            Self::MissingApiKey => "A translation API key is required for this provider",
            Self::KeyringUnavailable => "Secure credential storage is unavailable",
            Self::ClipUnavailable => "The clipboard item is unavailable",
            Self::ImageUnavailable => "The clipboard image is unavailable",
            Self::CaptureUnavailable => "The capture selection is unavailable",
            Self::OcrFailed => "Local OCR could not extract text from the image",
            Self::InvalidEndpoint => "Invalid translation endpoint",
            Self::UnsupportedProvider(_) => "Unsupported translation provider",
            Self::Timeout => "Translation request timed out",
            Self::Network => "Translation network request failed",
            Self::HttpStatus { .. } => "Translation service rejected the request",
            Self::ResponseTooLarge => "Translation response exceeded the 1 MB limit",
            Self::InvalidResponse => "Translation service returned an invalid response",
            Self::StaleRequest { .. } => "A newer translation request is active",
            Self::Internal => "Translation is temporarily unavailable",
        };
        format!("translation.{}: {message}", self.code())
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::EmptyInput => "empty_input",
            Self::InputTooLarge => "input_too_large",
            Self::SensitiveContent => "sensitive_content",
            Self::MissingApiKey => "missing_api_key",
            Self::KeyringUnavailable => "keyring_unavailable",
            Self::ClipUnavailable => "clip_unavailable",
            Self::ImageUnavailable => "image_unavailable",
            Self::CaptureUnavailable => "capture_unavailable",
            Self::OcrFailed => "ocr_failed",
            Self::InvalidEndpoint => "invalid_endpoint",
            Self::UnsupportedProvider(_) => "unsupported_provider",
            Self::Timeout => "timeout",
            Self::Network => "network",
            Self::HttpStatus { .. } => "http_status",
            Self::ResponseTooLarge => "response_too_large",
            Self::InvalidResponse => "invalid_response",
            Self::StaleRequest { .. } => "stale_request",
            Self::Internal => "internal",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn provider_values_are_stable() {
        assert_eq!(
            TranslationProvider::LibreTranslate.as_str(),
            "libretranslate"
        );
        assert_eq!(
            TranslationProvider::OpenAiCompatible.as_str(),
            "openai_compatible"
        );
        assert_eq!(
            TranslationProvider::from_str("openai_compatible").unwrap(),
            TranslationProvider::OpenAiCompatible
        );
    }

    #[test]
    fn only_network_and_server_errors_are_retryable() {
        assert!(TranslationError::Network.retryable());
        assert!(TranslationError::Timeout.retryable());
        assert!(TranslationError::HttpStatus { status: 503 }.retryable());
        assert!(!TranslationError::HttpStatus { status: 401 }.retryable());
        assert!(!TranslationError::InvalidResponse.retryable());
    }

    #[test]
    fn ipc_errors_have_stable_codes_and_hide_internal_context() {
        let unsupported = TranslationError::UnsupportedProvider("private-provider".to_string());
        assert_eq!(
            unsupported.ipc_message(),
            "translation.unsupported_provider: Unsupported translation provider"
        );
        assert!(!unsupported.ipc_message().contains("private-provider"));

        let stale = TranslationError::StaleRequest {
            request_id: 41,
            latest_request_id: 42,
        };
        assert_eq!(
            stale.ipc_message(),
            "translation.stale_request: A newer translation request is active"
        );
        assert!(!stale.ipc_message().contains("41"));

        assert_eq!(
            TranslationError::CaptureUnavailable.ipc_message(),
            "translation.capture_unavailable: The capture selection is unavailable"
        );
    }
}
