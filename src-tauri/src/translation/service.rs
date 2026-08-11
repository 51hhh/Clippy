use super::providers::{LibreTranslateProvider, OpenAiCompatibleProvider};
use super::types::{ProviderTranslation, TranslationError, TranslationRequest, TranslationResult};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use url::{Host, Url};

pub(crate) const MAX_RESPONSE_BYTES: usize = 1_048_576;
const MAX_INPUT_BYTES: usize = 1_048_576;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Provider HTTP 适配器。实现不得记录请求文本或凭据。
pub(crate) trait ProviderClient {
    fn translate(
        &self,
        request: &TranslationRequest,
        api_key: Option<&str>,
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
        api_key: Option<String>,
    ) -> Result<TranslationResult, TranslationError> {
        validate_request(&request)?;
        request.request_id = self.register_request_id(request.request_id);

        match request.provider {
            super::types::TranslationProvider::LibreTranslate => {
                self.translate_with_client(request, api_key.as_deref(), &LibreTranslateProvider)
            }
            super::types::TranslationProvider::OpenAiCompatible => {
                self.translate_with_client(request, api_key.as_deref(), &OpenAiCompatibleProvider)
            }
        }
    }

    fn translate_with_client(
        &self,
        request: TranslationRequest,
        api_key: Option<&str>,
        client: &dyn ProviderClient,
    ) -> Result<TranslationResult, TranslationError> {
        self.ensure_latest(request.request_id)?;
        let result = client.translate(&request, api_key)?;
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

fn validate_request(request: &TranslationRequest) -> Result<(), TranslationError> {
    if request.text.trim().is_empty() {
        return Err(TranslationError::EmptyInput);
    }
    if request.text.len() > MAX_INPUT_BYTES {
        return Err(TranslationError::InputTooLarge);
    }
    validate_endpoint(&request.endpoint)
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

/// 统一的 JSON POST：15 秒全局超时、1 MB 响应上限、网络/5xx 只重试一次。
pub(super) fn post_json(
    endpoint: &str,
    body: &Value,
    bearer_token: Option<&str>,
) -> Result<Value, TranslationError> {
    let config = ureq::config::Config::builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .max_redirects(0)
        .http_status_as_error(false)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let payload = body.to_string();

    retry_once(|| post_json_once(&agent, endpoint, payload.as_str(), bearer_token))
}

fn post_json_once(
    agent: &ureq::Agent,
    endpoint: &str,
    payload: &str,
    bearer_token: Option<&str>,
) -> Result<Value, TranslationError> {
    let mut request = agent
        .post(endpoint)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json");
    if let Some(token) = bearer_token.filter(|token| !token.is_empty()) {
        request = request.header("Authorization", &format!("Bearer {token}"));
    }

    let response = request.send(payload).map_err(map_ureq_error)?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(TranslationError::HttpStatus { status });
    }

    let body = response
        .into_body()
        .with_config()
        .limit((MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_vec()
        .map_err(map_body_error)?;
    decode_response_body(&body)
}

fn decode_response_body(body: &[u8]) -> Result<Value, TranslationError> {
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(TranslationError::ResponseTooLarge);
    }
    serde_json::from_slice(body).map_err(|_| TranslationError::InvalidResponse)
}

fn retry_once<T>(
    mut operation: impl FnMut() -> Result<T, TranslationError>,
) -> Result<T, TranslationError> {
    let first = operation();
    match first {
        Err(error) if error.retryable() => operation(),
        result => result,
    }
}

fn map_ureq_error(error: ureq::Error) -> TranslationError {
    match error {
        ureq::Error::Timeout(_) => TranslationError::Timeout,
        ureq::Error::StatusCode(status) => TranslationError::HttpStatus { status },
        _ => TranslationError::Network,
    }
}

fn map_body_error(error: ureq::Error) -> TranslationError {
    match error {
        ureq::Error::BodyExceedsLimit(_) => TranslationError::ResponseTooLarge,
        ureq::Error::Timeout(_) => TranslationError::Timeout,
        ureq::Error::Io(_) | ureq::Error::BodyStalled => TranslationError::Network,
        _ => TranslationError::InvalidResponse,
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::TranslationProvider;
    use super::*;
    use std::cell::Cell;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::mpsc::{self, Receiver};
    use std::thread::JoinHandle;
    use std::time::Duration;

    struct CapturedRequest {
        head: String,
        body: Value,
    }

    fn serve_json_once(response: Value) -> (String, Receiver<CapturedRequest>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let (header_end, content_length) = loop {
                let mut chunk = [0_u8; 4096];
                let count = stream.read(&mut chunk).unwrap();
                assert!(count > 0, "HTTP 请求在 header 完成前关闭");
                request.extend_from_slice(&chunk[..count]);
                if let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request[..header_end]);
                    let length = head
                        .lines()
                        .filter_map(|line| line.split_once(':'))
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if request.len() >= header_end + 4 + length {
                        break (header_end, length);
                    }
                }
            };

            let head = String::from_utf8(request[..header_end].to_vec()).unwrap();
            let body_start = header_end + 4;
            let body =
                serde_json::from_slice(&request[body_start..body_start + content_length]).unwrap();
            sender.send(CapturedRequest { head, body }).unwrap();

            let response = response.to_string();
            let wire = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response.len(),
                response
            );
            stream.write_all(wire.as_bytes()).unwrap();
        });
        (format!("http://{address}"), receiver, handle)
    }

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
            _api_key: Option<&str>,
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
            .translate_with_client(request(first), None, &client)
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
    fn retry_policy_retries_transient_failures_once() {
        let attempts = Cell::new(0);
        let result = retry_once(|| {
            attempts.set(attempts.get() + 1);
            if attempts.get() == 1 {
                Err(TranslationError::HttpStatus { status: 503 })
            } else {
                Ok("ok")
            }
        });

        assert_eq!(result, Ok("ok"));
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn retry_policy_does_not_retry_client_or_parse_errors() {
        for error in [
            TranslationError::HttpStatus { status: 401 },
            TranslationError::InvalidResponse,
            TranslationError::ResponseTooLarge,
        ] {
            let attempts = Cell::new(0);
            let result: Result<(), _> = retry_once(|| {
                attempts.set(attempts.get() + 1);
                Err(error.clone())
            });
            assert_eq!(result, Err(error));
            assert_eq!(attempts.get(), 1);
        }
    }

    #[test]
    fn response_body_limit_is_exactly_one_megabyte() {
        let mut exact = br#"{"ok":true}"#.to_vec();
        exact.resize(MAX_RESPONSE_BYTES, b' ');
        assert_eq!(decode_response_body(&exact).unwrap()["ok"], true);

        let oversized = vec![b' '; MAX_RESPONSE_BYTES + 1];
        assert_eq!(
            decode_response_body(&oversized),
            Err(TranslationError::ResponseTooLarge)
        );
    }

    #[test]
    fn libre_provider_completes_a_loopback_http_request() {
        let (endpoint, received, server) = serve_json_once(serde_json::json!({
            "translatedText": "你好",
            "detectedLanguage": { "language": "en" }
        }));
        let service = TranslationService::new();
        let request = TranslationRequest::new(
            "Hello".to_string(),
            "auto".to_string(),
            "zh-CN".to_string(),
            endpoint,
            TranslationProvider::LibreTranslate,
            None,
            1,
        );

        let result = service
            .translate(request, Some("local-test-key".to_string()))
            .unwrap();
        let captured = received.recv_timeout(Duration::from_secs(2)).unwrap();
        server.join().unwrap();

        assert!(captured.head.starts_with("POST /translate HTTP/1.1"));
        assert_eq!(captured.body["q"], "Hello");
        assert_eq!(captured.body["api_key"], "local-test-key");
        assert_eq!(result.translated_text, "你好");
        assert_eq!(result.detected_source_language.as_deref(), Some("en"));
    }

    #[test]
    fn openai_provider_sends_bearer_auth_over_loopback_http() {
        let (endpoint, received, server) = serve_json_once(serde_json::json!({
            "choices": [{ "message": { "content": "Bonjour" } }]
        }));
        let service = TranslationService::new();
        let request = TranslationRequest::new(
            "Hello".to_string(),
            "auto".to_string(),
            "fr".to_string(),
            format!("{endpoint}/v1"),
            TranslationProvider::OpenAiCompatible,
            Some("local-model".to_string()),
            2,
        );

        let result = service
            .translate(request, Some("local-secret".to_string()))
            .unwrap();
        let captured = received.recv_timeout(Duration::from_secs(2)).unwrap();
        server.join().unwrap();

        assert!(captured
            .head
            .starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(captured
            .head
            .lines()
            .any(|line| line.eq_ignore_ascii_case("authorization: Bearer local-secret")));
        assert_eq!(captured.body["model"], "local-model");
        assert_eq!(captured.body["messages"][1]["content"], "Hello");
        assert_eq!(result.translated_text, "Bonjour");
    }
}
