//! 翻译服务共用的阻塞 HTTP 层。
//!
//! 所有 provider 都必须经过这里，好处是超时、重定向、响应上限和重试策略只有一处实现：
//! 15 秒全局超时、禁止重定向（避免密钥被跟到第三方主机）、1 MB 响应上限，
//! 网络错误与 5xx 只重试一次。实现不得记录请求文本或凭据。

use super::types::TranslationError;
use serde_json::Value;
use std::time::Duration;

pub(crate) const MAX_RESPONSE_BYTES: usize = 1_048_576;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// 请求体形态。web 回退路径大量使用表单编码，所以它和 JSON 一样是一等公民。
enum RequestBody {
    Empty,
    Json(String),
    Form(String),
}

/// 单次 HTTP 请求。构造器只负责拼装，真正发送在 `send_json`/`send_text`。
pub(super) struct HttpRequest {
    post: bool,
    url: String,
    headers: Vec<(String, String)>,
    body: RequestBody,
}

impl HttpRequest {
    pub(super) fn get(url: impl Into<String>) -> Self {
        Self::new(false, url)
    }

    pub(super) fn post(url: impl Into<String>) -> Self {
        Self::new(true, url)
    }

    fn new(post: bool, url: impl Into<String>) -> Self {
        Self {
            post,
            url: url.into(),
            headers: Vec::new(),
            body: RequestBody::Empty,
        }
    }

    pub(super) fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// 空 token 视为未配置，不发送 Authorization 头。
    pub(super) fn bearer(self, token: Option<&str>) -> Self {
        match token.filter(|token| !token.trim().is_empty()) {
            Some(token) => self.header("Authorization", &format!("Bearer {token}")),
            None => self,
        }
    }

    /// 追加查询参数。端点自身的合法性由 `validate_endpoint` 负责，这里只做拼接。
    pub(super) fn query(mut self, params: &[(&str, &str)]) -> Self {
        if params.is_empty() {
            return self;
        }
        let encoded = encode_form(params);
        let separator = if self.url.contains('?') { '&' } else { '?' };
        self.url.push(separator);
        self.url.push_str(&encoded);
        self
    }

    pub(super) fn json(mut self, body: &Value) -> Self {
        self.body = RequestBody::Json(body.to_string());
        self
    }

    /// 已经序列化好的 JSON。DeepL 的 web 端点会按字节检查 payload 排版，
    /// 交给 `serde_json` 重新序列化会破坏它期望的形状。
    pub(super) fn raw_json(mut self, body: String) -> Self {
        self.body = RequestBody::Json(body);
        self
    }

    pub(super) fn form(mut self, fields: &[(&str, &str)]) -> Self {
        self.body = RequestBody::Form(encode_form(fields));
        self
    }

    pub(super) fn send_json(self) -> Result<Value, TranslationError> {
        let body = self.send()?;
        decode_json_body(&body)
    }

    /// 用于响应不是 JSON 的路径（web 页面 HTML、有道 web 的密文）。
    pub(super) fn send_text(self) -> Result<String, TranslationError> {
        let body = self.send()?;
        decode_text_body(body)
    }

    /// 用于响应是二进制的路径（dictvoice 音频）。空响应按无效响应处理，
    /// 否则前端会拿到一段播不出声的数据却看不到原因。
    pub(super) fn send_bytes(self) -> Result<Vec<u8>, TranslationError> {
        let body = self.send()?;
        if body.len() > MAX_RESPONSE_BYTES {
            return Err(TranslationError::ResponseTooLarge);
        }
        if body.is_empty() {
            return Err(TranslationError::InvalidResponse);
        }
        Ok(body)
    }

    fn has_header(&self, name: &str) -> bool {
        self.headers
            .iter()
            .any(|(header, _)| header.eq_ignore_ascii_case(name))
    }

    fn send(self) -> Result<Vec<u8>, TranslationError> {
        let agent = build_agent();
        retry_once(|| self.send_once(&agent))
    }

    /// ureq 3 的 GET/POST builder 是两种类型，无法共用一个变量，
    /// 因此先把最终头部算好，再在各自分支里套用。
    fn effective_headers(&self) -> Vec<(&str, &str)> {
        let mut headers = Vec::with_capacity(self.headers.len() + 2);
        // 默认要 JSON，但抓 web 页面的路径需要能自己声明 Accept，不能被覆盖成两个头。
        if !self.has_header("accept") {
            headers.push(("Accept", "application/json"));
        }
        match &self.body {
            RequestBody::Empty => {}
            RequestBody::Json(_) => headers.push(("Content-Type", "application/json")),
            RequestBody::Form(_) => {
                headers.push(("Content-Type", "application/x-www-form-urlencoded"))
            }
        }
        headers.extend(
            self.headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        );
        headers
    }

    fn send_once(&self, agent: &ureq::Agent) -> Result<Vec<u8>, TranslationError> {
        let headers = self.effective_headers();
        let response = if self.post {
            let mut request = agent.post(&self.url);
            for (name, value) in &headers {
                request = request.header(*name, *value);
            }
            match &self.body {
                RequestBody::Empty => request.send_empty(),
                RequestBody::Json(payload) | RequestBody::Form(payload) => {
                    request.send(payload.as_str())
                }
            }
        } else {
            let mut request = agent.get(&self.url);
            for (name, value) in &headers {
                request = request.header(*name, *value);
            }
            request.call()
        }
        .map_err(map_ureq_error)?;

        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(classify_status(status));
        }

        response
            .into_body()
            .with_config()
            .limit((MAX_RESPONSE_BYTES + 1) as u64)
            .read_to_vec()
            .map_err(map_body_error)
    }
}

/// 把状态码归一化成用户可以行动的错误：改密钥、等一会、还是充值。
/// 只保留跨服务含义一致的映射，服务专属语义留给 provider 自己解释响应体。
/// 456 是 DeepL 的配额耗尽码，402 是 Azure 的付费要求码，两者对用户是同一件事。
fn classify_status(status: u16) -> TranslationError {
    match status {
        401 | 403 => TranslationError::InvalidCredentials,
        402 | 456 => TranslationError::QuotaExceeded,
        429 => TranslationError::RateLimited,
        status => TranslationError::HttpStatus { status },
    }
}

fn build_agent() -> ureq::Agent {
    let config = ureq::config::Config::builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .max_redirects(0)
        .http_status_as_error(false)
        .build();
    ureq::Agent::new_with_config(config)
}

/// 统一的 JSON POST，保留给只需要 JSON + Bearer 的 provider。
pub(super) fn post_json(
    endpoint: &str,
    body: &Value,
    bearer_token: Option<&str>,
) -> Result<Value, TranslationError> {
    HttpRequest::post(endpoint)
        .bearer(bearer_token)
        .json(body)
        .send_json()
}

fn encode_form(fields: &[(&str, &str)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in fields {
        serializer.append_pair(name, value);
    }
    serializer.finish()
}

fn decode_json_body(body: &[u8]) -> Result<Value, TranslationError> {
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(TranslationError::ResponseTooLarge);
    }
    serde_json::from_slice(body).map_err(|_| TranslationError::InvalidResponse)
}

fn decode_text_body(body: Vec<u8>) -> Result<String, TranslationError> {
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(TranslationError::ResponseTooLarge);
    }
    String::from_utf8(body).map_err(|_| TranslationError::InvalidResponse)
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
        ureq::Error::StatusCode(status) => classify_status(status),
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
    use super::super::test_support::{MockResponse, MockServer};
    use super::*;
    use std::cell::Cell;

    #[test]
    fn form_encoding_escapes_reserved_characters() {
        let encoded = encode_form(&[("q", "a b&c=d"), ("to", "zh-Hans")]);
        assert_eq!(encoded, "q=a+b%26c%3Dd&to=zh-Hans");
    }

    #[test]
    fn query_params_respect_an_existing_query_string() {
        let first = HttpRequest::get("https://example.test/path").query(&[("key", "a b")]);
        assert_eq!(first.url, "https://example.test/path?key=a+b");

        let second = HttpRequest::get("https://example.test/path?api-version=3.0")
            .query(&[("to", "zh-Hans")]);
        assert_eq!(
            second.url,
            "https://example.test/path?api-version=3.0&to=zh-Hans"
        );

        let untouched = HttpRequest::get("https://example.test/path").query(&[]);
        assert_eq!(untouched.url, "https://example.test/path");
    }

    #[test]
    fn empty_bearer_token_does_not_add_a_header() {
        assert!(HttpRequest::get("https://example.test")
            .bearer(Some("  "))
            .headers
            .is_empty());
        assert!(HttpRequest::get("https://example.test")
            .bearer(None)
            .headers
            .is_empty());
        assert_eq!(
            HttpRequest::get("https://example.test")
                .bearer(Some("token"))
                .headers,
            vec![("Authorization".to_string(), "Bearer token".to_string())]
        );
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
            TranslationError::ProviderEndpointBroken,
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
        assert_eq!(decode_json_body(&exact).unwrap()["ok"], true);

        let oversized = vec![b' '; MAX_RESPONSE_BYTES + 1];
        assert_eq!(
            decode_json_body(&oversized),
            Err(TranslationError::ResponseTooLarge)
        );
        assert_eq!(
            decode_text_body(oversized),
            Err(TranslationError::ResponseTooLarge)
        );
    }

    #[test]
    fn status_codes_map_to_actionable_errors() {
        assert_eq!(classify_status(401), TranslationError::InvalidCredentials);
        assert_eq!(classify_status(403), TranslationError::InvalidCredentials);
        assert_eq!(classify_status(429), TranslationError::RateLimited);
        assert_eq!(classify_status(402), TranslationError::QuotaExceeded);
        assert_eq!(classify_status(456), TranslationError::QuotaExceeded);
        assert_eq!(
            classify_status(500),
            TranslationError::HttpStatus { status: 500 }
        );
        // 5xx 仍然可重试，凭据和配额错误重试没有意义。
        assert!(classify_status(503).retryable());
        assert!(!classify_status(401).retryable());
        assert!(!classify_status(429).retryable());
    }

    #[test]
    fn non_success_status_short_circuits_before_parsing_the_body() {
        // 429 不可重试，所以单个响应就够；正文是 HTML 也不该变成 InvalidResponse。
        let server = MockServer::new(vec![MockResponse::status(429, "<html>slow down</html>")]);
        let result = HttpRequest::get(format!("{}/translate", server.base_url)).send_json();
        server.recv();
        server.finish();

        assert_eq!(result, Err(TranslationError::RateLimited));
    }

    #[test]
    fn text_bodies_reject_invalid_utf8() {
        assert_eq!(
            decode_text_body(vec![0xff, 0xfe]),
            Err(TranslationError::InvalidResponse)
        );
        assert_eq!(decode_text_body(b"<html>".to_vec()).unwrap(), "<html>");
    }
}
