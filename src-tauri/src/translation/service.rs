use super::providers::{
    BingProvider, DeepLProvider, GoogleProvider, LibreTranslateProvider, OpenAiCompatibleProvider,
    YoudaoProvider,
};
use super::types::{
    ProviderCredentials, ProviderTranslation, TranslationError, TranslationProvider,
    TranslationRequest, TranslationResult,
};
use std::sync::atomic::{AtomicU64, Ordering};
use url::{Host, Url};

const MAX_INPUT_BYTES: usize = 1_048_576;

/// Provider HTTP 适配器。实现不得记录请求文本或凭据。
pub(crate) trait ProviderClient {
    fn translate(
        &self,
        request: &TranslationRequest,
        credentials: &ProviderCredentials,
    ) -> Result<ProviderTranslation, TranslationError>;
}

/// 翻译领域服务，负责 request-id 分配以及陈旧结果保护。
pub struct TranslationService {
    latest_request_id: AtomicU64,
}

impl Default for TranslationService {
    fn default() -> Self {
        Self::new()
    }
}

impl TranslationService {
    pub fn new() -> Self {
        Self {
            latest_request_id: AtomicU64::new(0),
        }
    }

    pub fn next_request_id(&self) -> u64 {
        self.latest_request_id.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// 将外部 request-id 提升为当前序列，供重试页面保持自己的 ID。
    pub fn register_request_id(&self, request_id: u64) -> u64 {
        if request_id == 0 {
            return self.next_request_id();
        }
        let mut current = self.latest_request_id.load(Ordering::Acquire);
        while current < request_id {
            match self.latest_request_id.compare_exchange(
                current,
                request_id,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(next) => current = next,
            }
        }
        request_id
    }

    pub fn is_latest(&self, request_id: u64) -> bool {
        self.latest_request_id.load(Ordering::Acquire) == request_id
    }

    pub fn translate(
        &self,
        mut request: TranslationRequest,
        credentials: ProviderCredentials,
    ) -> Result<TranslationResult, TranslationError> {
        validate_request(&request)?;
        request.request_id = self.register_request_id(request.request_id);
        let client = provider_client(request.provider);
        self.translate_with_client(request, &credentials, client)
    }

    fn translate_with_client(
        &self,
        request: TranslationRequest,
        credentials: &ProviderCredentials,
        client: &dyn ProviderClient,
    ) -> Result<TranslationResult, TranslationError> {
        self.ensure_latest(request.request_id)?;
        let result = client.translate(&request, credentials)?;
        self.ensure_latest(request.request_id)?;

        Ok(TranslationResult {
            request_id: request.request_id,
            provider: request.provider,
            translated_text: result.translated_text,
            detected_source_language: result.detected_source_language,
        })
    }

    fn ensure_latest(&self, request_id: u64) -> Result<(), TranslationError> {
        if !self.is_latest(request_id) {
            return Err(TranslationError::StaleRequest {
                request_id,
                latest_request_id: self.latest_request_id.load(Ordering::Acquire),
            });
        }
        Ok(())
    }
}

/// provider 到实现的唯一映射，新增服务只需在此登记一次。
fn provider_client(provider: TranslationProvider) -> &'static dyn ProviderClient {
    match provider {
        TranslationProvider::LibreTranslate => &LibreTranslateProvider,
        TranslationProvider::OpenAiCompatible => &OpenAiCompatibleProvider,
        TranslationProvider::DeepL => &DeepLProvider,
        TranslationProvider::Google => &GoogleProvider,
        TranslationProvider::Bing => &BingProvider,
        TranslationProvider::Youdao => &YoudaoProvider,
    }
}

fn validate_request(request: &TranslationRequest) -> Result<(), TranslationError> {
    if request.text.trim().is_empty() {
        return Err(TranslationError::EmptyInput);
    }
    if request.text.len() > MAX_INPUT_BYTES {
        return Err(TranslationError::InputTooLarge);
    }
    validate_endpoint(request.endpoint())?;
    // web 回退端点同样会收到用户文本，不能因为“只是回退路径”就跳过白名单。
    let web_endpoint = request.web_endpoint();
    if !web_endpoint.is_empty() {
        validate_endpoint(web_endpoint)?;
    }
    Ok(())
}

/// 仅允许 HTTPS；HTTP 只为本地自托管服务保留，避免把密钥发送到明文远端。
pub(crate) fn validate_endpoint(endpoint: &str) -> Result<(), TranslationError> {
    if endpoint.is_empty() || endpoint != endpoint.trim() {
        return Err(TranslationError::InvalidEndpoint);
    }
    let parsed = Url::parse(endpoint).map_err(|_| TranslationError::InvalidEndpoint)?;
    if parsed.cannot_be_a_base()
        || parsed.host().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(TranslationError::InvalidEndpoint);
    }

    match (parsed.scheme(), parsed.host()) {
        ("https", Some(_)) => Ok(()),
        ("http", Some(Host::Domain(host))) if host.eq_ignore_ascii_case("localhost") => Ok(()),
        ("http", Some(Host::Ipv4(address))) if address.is_loopback() => Ok(()),
        ("http", Some(Host::Ipv6(address))) if address.is_loopback() => Ok(()),
        _ => Err(TranslationError::InvalidEndpoint),
    }
}

pub(super) fn append_endpoint_path(endpoint: &str, path: &str) -> String {
    let base = endpoint.trim_end_matches('/');
    if base.ends_with(path) {
        base.to_string()
    } else {
        format!("{base}{path}")
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::MockServer;
    use super::super::types::ProviderOptions;
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    fn request(request_id: u64) -> TranslationRequest {
        TranslationRequest::new(
            "Hello".to_string(),
            "auto".to_string(),
            "zh-CN".to_string(),
            "https://example.test".to_string(),
            TranslationProvider::LibreTranslate,
            None,
            request_id,
        )
    }

    struct CountingClient {
        calls: AtomicUsize,
    }

    impl ProviderClient for CountingClient {
        fn translate(
            &self,
            _request: &TranslationRequest,
            _credentials: &ProviderCredentials,
        ) -> Result<ProviderTranslation, TranslationError> {
            self.calls.fetch_add(1, AtomicOrdering::Relaxed);
            Ok(ProviderTranslation {
                translated_text: "你好".to_string(),
                detected_source_language: Some("en".to_string()),
            })
        }
    }

    #[test]
    fn endpoint_policy_accepts_https_and_local_http_only() {
        assert!(validate_endpoint("https://libretranslate.com").is_ok());
        assert!(validate_endpoint("http://localhost:5000").is_ok());
        assert!(validate_endpoint("http://127.0.0.1:5000/v1").is_ok());
        assert!(validate_endpoint("http://127.42.0.9:5000/v1").is_ok());
        assert!(validate_endpoint("http://[::1]:5000").is_ok());
        assert!(validate_endpoint("http://translate.example").is_err());
        assert!(validate_endpoint("http://localhost.example:5000").is_err());
        assert!(validate_endpoint("http://192.168.1.2:5000").is_err());
        assert!(validate_endpoint("https://user:pass@example").is_err());
        assert!(validate_endpoint("https://example.test?key=value").is_err());
        assert!(validate_endpoint("https://example.test/#fragment").is_err());
        assert!(validate_endpoint(" https://example.test").is_err());
    }

    #[test]
    fn every_provider_default_endpoint_passes_the_policy() {
        for provider in TranslationProvider::all() {
            let endpoint = provider.default_endpoint();
            if !endpoint.is_empty() {
                assert!(
                    validate_endpoint(endpoint).is_ok(),
                    "{} 的官方默认端点不符合端点策略",
                    provider.as_str()
                );
            }
            let web_endpoint = provider.default_web_endpoint();
            if !web_endpoint.is_empty() {
                assert!(
                    validate_endpoint(web_endpoint).is_ok(),
                    "{} 的 web 默认端点不符合端点策略",
                    provider.as_str()
                );
            }
        }
    }

    #[test]
    fn web_fallback_endpoint_is_validated_like_the_official_one() {
        let request = TranslationRequest::with_options(
            "Hello".to_string(),
            "auto".to_string(),
            "zh-Hans".to_string(),
            TranslationProvider::DeepL,
            ProviderOptions {
                web_endpoint: "http://deepl.example".to_string(),
                ..ProviderOptions::default()
            },
            1,
        );
        assert_eq!(
            validate_request(&request),
            Err(TranslationError::InvalidEndpoint)
        );
    }

    #[test]
    fn endpoint_path_is_appended_once() {
        assert_eq!(
            append_endpoint_path("https://example.test", "/translate"),
            "https://example.test/translate"
        );
        assert_eq!(
            append_endpoint_path("https://example.test/translate", "/translate"),
            "https://example.test/translate"
        );
    }

    #[test]
    fn request_ids_make_newer_request_current() {
        let service = TranslationService::new();
        let first = service.next_request_id();
        let second = service.next_request_id();
        assert!(second > first);
        assert!(!service.is_latest(first));
        assert!(service.is_latest(second));
    }

    #[test]
    fn stale_request_is_rejected_before_provider_call() {
        let service = TranslationService::new();
        let first = service.next_request_id();
        let second = service.next_request_id();
        let client = CountingClient {
            calls: AtomicUsize::new(0),
        };

        let error = service
            .translate_with_client(request(first), &ProviderCredentials::default(), &client)
            .unwrap_err();

        assert_eq!(
            error,
            TranslationError::StaleRequest {
                request_id: first,
                latest_request_id: second,
            }
        );
        assert_eq!(client.calls.load(AtomicOrdering::Relaxed), 0);
    }

    #[test]
    fn libre_provider_completes_a_loopback_http_request() {
        let server = MockServer::json_once(serde_json::json!({
            "translatedText": "你好",
            "detectedLanguage": { "language": "en" }
        }));
        let service = TranslationService::new();
        let request = TranslationRequest::new(
            "Hello".to_string(),
            "auto".to_string(),
            "zh-CN".to_string(),
            server.base_url.clone(),
            TranslationProvider::LibreTranslate,
            None,
            1,
        );

        let result = service
            .translate(
                request,
                ProviderCredentials::new(Some("local-test-key".to_string()), None),
            )
            .unwrap();
        let captured = server.recv();
        server.finish();

        assert_eq!(captured.method(), "POST");
        assert_eq!(captured.target(), "/translate");
        assert_eq!(captured.json()["q"], "Hello");
        assert_eq!(captured.json()["api_key"], "local-test-key");
        assert_eq!(result.translated_text, "你好");
        assert_eq!(result.detected_source_language.as_deref(), Some("en"));
    }

    #[test]
    fn openai_provider_sends_bearer_auth_over_loopback_http() {
        let server = MockServer::json_once(serde_json::json!({
            "choices": [{ "message": { "content": "Bonjour" } }]
        }));
        let service = TranslationService::new();
        let request = TranslationRequest::new(
            "Hello".to_string(),
            "auto".to_string(),
            "fr".to_string(),
            format!("{}/v1", server.base_url),
            TranslationProvider::OpenAiCompatible,
            Some("local-model".to_string()),
            2,
        );

        let result = service
            .translate(
                request,
                ProviderCredentials::new(Some("local-secret".to_string()), None),
            )
            .unwrap();
        let captured = server.recv();
        server.finish();

        assert_eq!(captured.target(), "/v1/chat/completions");
        assert_eq!(
            captured.header("authorization").as_deref(),
            Some("Bearer local-secret")
        );
        assert_eq!(captured.json()["model"], "local-model");
        assert_eq!(captured.json()["messages"][1]["content"], "Hello");
        assert_eq!(result.translated_text, "Bonjour");
    }
}
