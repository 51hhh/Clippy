use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 翻译服务类型。字符串值是稳定的 IPC/config 合同，不能随意更改。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationProvider {
    #[serde(rename = "libretranslate")]
    LibreTranslate,
    #[serde(rename = "openai_compatible")]
    OpenAiCompatible,
    #[serde(rename = "deepl")]
    DeepL,
    #[serde(rename = "google")]
    Google,
    #[serde(rename = "bing")]
    Bing,
    #[serde(rename = "youdao")]
    Youdao,
}

impl TranslationProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LibreTranslate => "libretranslate",
            Self::OpenAiCompatible => "openai_compatible",
            Self::DeepL => "deepl",
            Self::Google => "google",
            Self::Bing => "bing",
            Self::Youdao => "youdao",
        }
    }

    /// 全部服务，顺序即设置页与结果卡的默认展示顺序。
    pub fn all() -> [Self; 6] {
        [
            Self::LibreTranslate,
            Self::OpenAiCompatible,
            Self::DeepL,
            Self::Google,
            Self::Bing,
            Self::Youdao,
        ]
    }

    /// 官方 API 基址。OpenAI-compatible 没有通用默认值，必须由用户填写。
    pub fn default_endpoint(self) -> &'static str {
        match self {
            Self::LibreTranslate => "https://libretranslate.com",
            Self::OpenAiCompatible => "",
            // DeepL 免费版与 Pro 版是不同主机，默认取免费版，Pro 用户改 endpoint。
            Self::DeepL => "https://api-free.deepl.com",
            Self::Google => "https://translation.googleapis.com",
            Self::Bing => "https://api.cognitive.microsofttranslator.com",
            Self::Youdao => "https://openapi.youdao.com",
        }
    }

    /// 未配置凭据时回退的非官方 web 端点，空串表示该服务没有回退路径。
    /// 走这条路径的代价见 docs/reference-project-guidance.md。
    pub fn default_web_endpoint(self) -> &'static str {
        match self {
            Self::LibreTranslate | Self::OpenAiCompatible => "",
            Self::DeepL => "https://www2.deepl.com",
            Self::Google => "https://translate.googleapis.com",
            Self::Bing => "https://cn.bing.com",
            Self::Youdao => "https://dict.youdao.com",
        }
    }

    /// 该服务是否还需要第二个凭据字段（有道官方 API 要 appKey + appSecret）。
    pub fn requires_api_secret(self) -> bool {
        matches!(self, Self::Youdao)
    }

    /// 没有凭据时是否完全不可用。LibreTranslate 公共实例允许匿名调用，
    /// DeepL/Google/Bing/有道 有 web 回退，只有 OpenAI-compatible 必须有密钥。
    pub fn requires_credentials(self) -> bool {
        matches!(self, Self::OpenAiCompatible)
    }
}

impl std::str::FromStr for TranslationProvider {
    type Err = TranslationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let normalized = value.trim().to_ascii_lowercase();
        // 规范名统一走 all()，新增服务不必在这里再登记一遍；下面只列历史别名。
        if let Some(provider) = Self::all()
            .into_iter()
            .find(|provider| provider.as_str() == normalized)
        {
            return Ok(provider);
        }
        match normalized.as_str() {
            "libre_translate" => Ok(Self::LibreTranslate),
            "openai-compatible" => Ok(Self::OpenAiCompatible),
            "microsoft" => Ok(Self::Bing),
            other => Err(TranslationError::UnsupportedProvider(other.to_string())),
        }
    }
}

/// 单个服务的可配置参数。空字符串/None 表示“用 provider 默认值”，
/// 这样新增服务不需要用户先去设置页填一遍端点。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderOptions {
    pub endpoint: String,
    pub web_endpoint: String,
    pub model: Option<String>,
    /// Azure 资源区域，仅 Bing 官方 API 使用。
    pub region: Option<String>,
    /// GCP 项目 ID，仅 Google Cloud v3 使用。
    pub project: Option<String>,
}

/// 单个服务的凭据。刻意不实现 Debug/Serialize，避免密钥进日志或跨 IPC。
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ProviderCredentials {
    api_key: Option<String>,
    api_secret: Option<String>,
}

impl ProviderCredentials {
    pub fn new(api_key: Option<String>, api_secret: Option<String>) -> Self {
        Self {
            api_key: Self::clean(api_key),
            api_secret: Self::clean(api_secret),
        }
    }

    pub fn from_api_key(api_key: Option<String>) -> Self {
        Self::new(api_key, None)
    }

    fn clean(value: Option<String>) -> Option<String> {
        value.filter(|value| !value.trim().is_empty())
    }

    pub fn key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }

    pub fn secret(&self) -> Option<&str> {
        self.api_secret.as_deref()
    }

    pub fn is_empty(&self) -> bool {
        self.api_key.is_none() && self.api_secret.is_none()
    }

    /// 官方 API 所需的凭据是否齐全；缺一半时不能静默降级成 web 路径，
    /// 否则用户以为自己填了密钥、实际请求却发去了非官方端点。
    pub fn complete_for(&self, provider: TranslationProvider) -> bool {
        self.api_key.is_some() && (!provider.requires_api_secret() || self.api_secret.is_some())
    }
}

/// 单次请求的服务参数。凭据刻意不放进此结构，避免被序列化或误记录。
#[derive(Clone, PartialEq, Eq)]
pub struct TranslationRequest {
    pub text: String,
    pub source_language: String,
    pub target_language: String,
    pub provider: TranslationProvider,
    pub options: ProviderOptions,
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
        Self::with_options(
            text,
            source_language,
            target_language,
            provider,
            ProviderOptions {
                endpoint,
                model,
                ..ProviderOptions::default()
            },
            request_id,
        )
    }

    pub fn with_options(
        text: String,
        source_language: String,
        target_language: String,
        provider: TranslationProvider,
        options: ProviderOptions,
        request_id: u64,
    ) -> Self {
        Self {
            text,
            source_language,
            target_language,
            provider,
            options,
            request_id,
        }
    }

    /// 官方 API 基址，配置为空时回落到 provider 默认值。
    pub fn endpoint(&self) -> &str {
        let configured = self.options.endpoint.trim_end_matches('/');
        if configured.is_empty() {
            self.provider.default_endpoint()
        } else {
            configured
        }
    }

    /// 非官方 web 基址，配置为空时回落到 provider 默认值。
    pub fn web_endpoint(&self) -> &str {
        let configured = self.options.web_endpoint.trim_end_matches('/');
        if configured.is_empty() {
            self.provider.default_web_endpoint()
        } else {
            configured
        }
    }

    pub fn model(&self) -> Option<&str> {
        self.options
            .model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty())
    }

    pub fn region(&self) -> Option<&str> {
        Self::non_empty(self.options.region.as_deref())
    }

    pub fn project(&self) -> Option<&str> {
        Self::non_empty(self.options.project.as_deref())
    }

    /// 源语言为空或 auto 时返回 None，交给服务自行检测。
    pub fn source(&self) -> Option<&str> {
        Self::non_empty(Some(self.source_language.as_str()))
            .filter(|value| !value.eq_ignore_ascii_case("auto"))
    }

    fn non_empty(value: Option<&str>) -> Option<&str> {
        value.map(str::trim).filter(|value| !value.is_empty())
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
    #[error("The translation service credentials are incomplete")]
    IncompleteCredentials,
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
    #[error("The translation service rejected the credentials")]
    InvalidCredentials,
    #[error("The translation service is rate limiting requests")]
    RateLimited,
    #[error("The translation service quota is exhausted")]
    QuotaExceeded,
    #[error("Translation response exceeded the 1 MB limit")]
    ResponseTooLarge,
    #[error("Translation service returned an invalid response")]
    InvalidResponse,
    #[error("The translation service endpoint no longer works")]
    ProviderEndpointBroken,
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
            Self::IncompleteCredentials => "The translation service credentials are incomplete",
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
            Self::InvalidCredentials => "The translation service rejected the credentials",
            Self::RateLimited => "The translation service is rate limiting requests",
            Self::QuotaExceeded => "The translation service quota is exhausted",
            Self::ResponseTooLarge => "Translation response exceeded the 1 MB limit",
            Self::InvalidResponse => "Translation service returned an invalid response",
            Self::ProviderEndpointBroken => "The translation service endpoint no longer works",
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
            Self::IncompleteCredentials => "incomplete_credentials",
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
            Self::InvalidCredentials => "invalid_credentials",
            Self::RateLimited => "rate_limited",
            Self::QuotaExceeded => "quota_exceeded",
            Self::ResponseTooLarge => "response_too_large",
            Self::InvalidResponse => "invalid_response",
            Self::ProviderEndpointBroken => "provider_endpoint_broken",
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
        let expected = [
            "libretranslate",
            "openai_compatible",
            "deepl",
            "google",
            "bing",
            "youdao",
        ];
        let actual: Vec<&str> = TranslationProvider::all()
            .into_iter()
            .map(TranslationProvider::as_str)
            .collect();
        assert_eq!(actual, expected);

        for value in expected {
            let provider = TranslationProvider::from_str(value).unwrap();
            assert_eq!(provider.as_str(), value);
            assert_eq!(
                serde_json::to_value(provider).unwrap(),
                serde_json::json!(value)
            );
        }
    }

    #[test]
    fn provider_defaults_cover_official_and_web_paths() {
        for provider in TranslationProvider::all() {
            if provider != TranslationProvider::OpenAiCompatible {
                assert!(
                    provider.default_endpoint().starts_with("https://"),
                    "{} 缺少官方端点",
                    provider.as_str()
                );
            }
        }

        // 只有这四个服务有非官方 web 回退；Libre/OpenAI 没有，必须保持空串。
        for provider in [
            TranslationProvider::DeepL,
            TranslationProvider::Google,
            TranslationProvider::Bing,
            TranslationProvider::Youdao,
        ] {
            assert!(provider.default_web_endpoint().starts_with("https://"));
        }
        assert!(TranslationProvider::LibreTranslate
            .default_web_endpoint()
            .is_empty());
        assert!(TranslationProvider::OpenAiCompatible
            .default_web_endpoint()
            .is_empty());
        assert!(TranslationProvider::OpenAiCompatible.requires_credentials());
        assert!(!TranslationProvider::DeepL.requires_credentials());
    }

    #[test]
    fn request_falls_back_to_provider_defaults() {
        let request = TranslationRequest::with_options(
            "Hello".to_string(),
            "auto".to_string(),
            "zh-Hans".to_string(),
            TranslationProvider::DeepL,
            ProviderOptions::default(),
            1,
        );
        assert_eq!(request.endpoint(), "https://api-free.deepl.com");
        assert_eq!(request.web_endpoint(), "https://www2.deepl.com");
        assert_eq!(request.source(), None);
        assert_eq!(request.model(), None);

        let overridden = TranslationRequest::with_options(
            "Hello".to_string(),
            " en ".to_string(),
            "zh-Hans".to_string(),
            TranslationProvider::DeepL,
            ProviderOptions {
                endpoint: "https://api.deepl.com/".to_string(),
                model: Some("  ".to_string()),
                ..ProviderOptions::default()
            },
            1,
        );
        assert_eq!(overridden.endpoint(), "https://api.deepl.com");
        assert_eq!(overridden.source(), Some("en"));
        assert_eq!(overridden.model(), None);
    }

    #[test]
    fn credentials_require_every_field_the_provider_needs() {
        let key_only = ProviderCredentials::from_api_key(Some("app-key".to_string()));
        assert!(key_only.complete_for(TranslationProvider::DeepL));
        // 有道官方 API 只有 appKey 时不完整，不能悄悄降级成非官方端点。
        assert!(!key_only.complete_for(TranslationProvider::Youdao));

        let pair =
            ProviderCredentials::new(Some("app-key".to_string()), Some("app-secret".to_string()));
        assert!(pair.complete_for(TranslationProvider::Youdao));

        let blank = ProviderCredentials::new(Some("  ".to_string()), Some(String::new()));
        assert!(blank.is_empty());
        assert_eq!(blank.key(), None);
        assert_eq!(blank.secret(), None);
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

        // 前端要能把“接口失效”和“配置错误”分开提示，两者不能共用一个错误码。
        assert_eq!(
            TranslationError::ProviderEndpointBroken.ipc_message(),
            "translation.provider_endpoint_broken: The translation service endpoint no longer works"
        );
        assert_eq!(
            TranslationError::IncompleteCredentials.ipc_message(),
            "translation.incomplete_credentials: The translation service credentials are incomplete"
        );
    }
}
