//! DeepL 适配：官方 API + 未配置密钥时的非官方 web JSON-RPC 回退。
//!
//! 免费版与 Pro 版是不同主机（api-free / api），默认走免费版，Pro 用户改 endpoint。
//! web 路径没有可用性承诺，行为对齐参考项目，代价见 docs/reference-project-guidance.md。

use super::super::http::HttpRequest;
use super::super::service::ProviderClient;
use super::super::types::{
    ProviderCredentials, ProviderTranslation, TranslationError, TranslationRequest,
};
use super::routing::{route, Route};
use serde_json::{json, Value};

pub(crate) struct DeepLProvider;

impl ProviderClient for DeepLProvider {
    fn translate(
        &self,
        request: &TranslationRequest,
        credentials: &ProviderCredentials,
    ) -> Result<ProviderTranslation, TranslationError> {
        match route(request.provider, credentials)? {
            Route::Official { key, .. } => translate_official(request, key),
            Route::Web => translate_web(request),
        }
    }
}

fn translate_official(
    request: &TranslationRequest,
    api_key: &str,
) -> Result<ProviderTranslation, TranslationError> {
    let target = language_code(&request.target_language);
    let source = request.source().map(language_code);
    let mut fields: Vec<(&str, &str)> = vec![("text", &request.text), ("target_lang", &target)];
    if let Some(source) = source.as_deref() {
        fields.push(("source_lang", source));
    }

    let response = HttpRequest::post(format!("{}/v2/translate", request.endpoint()))
        .header("Authorization", &format!("DeepL-Auth-Key {api_key}"))
        .form(&fields)
        .send_json()?;
    parse_official(&response)
}

fn translate_web(request: &TranslationRequest) -> Result<ProviderTranslation, TranslationError> {
    let rpc_id = rpc_id();
    let payload = web_payload(request, rpc_id, timestamp(i_count(&request.text)));
    let response = HttpRequest::post(format!("{}/jsonrpc", request.web_endpoint()))
        .raw_json(rpc_body(&payload, rpc_id))
        .send_json()?;
    parse_web(&response)
}

/// DeepL 只接受主语言子标签的大写形式（`zh-Hans` → `ZH`）。
fn language_code(code: &str) -> String {
    let normalized = code.trim().replace('_', "-");
    normalized
        .split('-')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase()
}

fn parse_official(response: &Value) -> Result<ProviderTranslation, TranslationError> {
    let first = response
        .get("translations")
        .and_then(|value| value.as_array())
        .and_then(|items| items.first())
        .ok_or(TranslationError::InvalidResponse)?;
    let translated_text =
        non_empty_text(first.get("text")).ok_or(TranslationError::InvalidResponse)?;
    let detected_source_language = first
        .get("detected_source_language")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase);

    Ok(ProviderTranslation {
        translated_text,
        detected_source_language,
    })
}

/// web 端点的 JSON-RPC 请求体。`timestamp` 与文本里 `i` 的数量相关，
/// 服务端会校验这个关系，所以两者必须一起计算。
fn web_payload(request: &TranslationRequest, rpc_id: i64, timestamp: i64) -> Value {
    let source = request
        .source()
        .map(language_code)
        .unwrap_or_else(|| "auto".to_string());
    let mut params = json!({
        "texts": [{ "text": request.text, "requestAlternatives": 3 }],
        "splitting": "newlines",
        "lang": {
            "source_lang_user_selected": source,
            "target_lang": language_code(&request.target_language),
        },
        "timestamp": timestamp,
    });
    // 目标语言带地区（zh-Hans、pt-BR）时必须额外声明地区变体，否则会拿到错误变体。
    if request.target_language.contains('-') {
        params["commonJobParams"] = json!({
            "regionalVariant": request.target_language,
            "mode": "translate",
            "browserType": 1,
            "textType": "plaintext",
        });
    }

    json!({
        "jsonrpc": "2.0",
        "method": "LMT_handle_texts",
        "id": rpc_id,
        "params": params,
    })
}

/// 序列化后按 id 调整 `"method":` 后的空格。DeepL web 用这个排版特征识别官方前端，
/// 排版不对会被拒。因此这里必须手动改字节，不能交给 serde_json 重新序列化。
fn rpc_body(payload: &Value, rpc_id: i64) -> String {
    let body = payload.to_string();
    if (rpc_id + 5) % 29 == 0 || (rpc_id + 3) % 13 == 0 {
        body.replace("\"method\":\"", "\"method\" : \"")
    } else {
        body.replace("\"method\":\"", "\"method\": \"")
    }
}

fn i_count(text: &str) -> i64 {
    text.matches('i').count() as i64
}

fn timestamp(i_count: i64) -> i64 {
    let now = unix_millis();
    if i_count == 0 {
        return now;
    }
    let count = i_count + 1;
    now - (now % count) + count
}

fn unix_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn rpc_id() -> i64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as i64)
        .unwrap_or_default();
    100_000_000 + (nanos.abs() % 89_999_000)
}

fn parse_web(response: &Value) -> Result<ProviderTranslation, TranslationError> {
    let result = response
        .get("result")
        .ok_or(TranslationError::ProviderEndpointBroken)?;
    let first = result
        .get("texts")
        .and_then(Value::as_array)
        .and_then(|texts| texts.first())
        .ok_or(TranslationError::ProviderEndpointBroken)?;
    let translated_text =
        non_empty_text(first.get("text")).ok_or(TranslationError::ProviderEndpointBroken)?;
    let detected_source_language = result
        .get("lang")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase);

    Ok(ProviderTranslation {
        translated_text,
        detected_source_language,
    })
}

fn non_empty_text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::super::super::test_support::MockServer;
    use super::super::super::types::{ProviderOptions, TranslationProvider};
    use super::*;

    fn request(source: &str, options: ProviderOptions) -> TranslationRequest {
        TranslationRequest::with_options(
            "Hi it is I".to_string(),
            source.to_string(),
            "zh-Hans".to_string(),
            TranslationProvider::DeepL,
            options,
            1,
        )
    }

    fn official_options(endpoint: String) -> ProviderOptions {
        ProviderOptions {
            endpoint,
            ..ProviderOptions::default()
        }
    }

    fn web_options(web_endpoint: String) -> ProviderOptions {
        ProviderOptions {
            web_endpoint,
            ..ProviderOptions::default()
        }
    }

    #[test]
    fn language_codes_use_the_primary_subtag_in_upper_case() {
        assert_eq!(language_code("zh-Hans"), "ZH");
        assert_eq!(language_code("pt_BR"), "PT");
        assert_eq!(language_code(" en "), "EN");
    }

    #[test]
    fn official_response_keeps_the_detected_language_lower_case() {
        let response = json!({
            "translations": [{ "detected_source_language": "EN", "text": "你好" }]
        });
        assert_eq!(
            parse_official(&response).unwrap(),
            ProviderTranslation {
                translated_text: "你好".to_string(),
                detected_source_language: Some("en".to_string()),
            }
        );
        assert_eq!(
            parse_official(&json!({ "translations": [] })),
            Err(TranslationError::InvalidResponse)
        );
        assert_eq!(
            parse_official(&json!({ "translations": [{ "text": "  " }] })),
            Err(TranslationError::InvalidResponse)
        );
    }

    #[test]
    fn official_path_sends_the_form_body_and_auth_header() {
        let server = MockServer::json_once(json!({
            "translations": [{ "detected_source_language": "EN", "text": "你好" }]
        }));
        let request = request("auto", official_options(server.base_url.clone()));

        let result = DeepLProvider
            .translate(
                &request,
                &ProviderCredentials::new(Some("free-key".to_string()), None),
            )
            .unwrap();
        let captured = server.recv();
        server.finish();

        assert_eq!(captured.method(), "POST");
        assert_eq!(captured.target(), "/v2/translate");
        assert_eq!(
            captured.header("authorization").as_deref(),
            Some("DeepL-Auth-Key free-key")
        );
        assert_eq!(
            captured.header("content-type").as_deref(),
            Some("application/x-www-form-urlencoded")
        );
        let form = captured.form();
        assert_eq!(form.get("text").map(String::as_str), Some("Hi it is I"));
        assert_eq!(form.get("target_lang").map(String::as_str), Some("ZH"));
        // 源语言为 auto 时不发送 source_lang，交给 DeepL 自行检测。
        assert!(!form.contains_key("source_lang"));
        assert_eq!(result.translated_text, "你好");
    }

    #[test]
    fn official_path_forwards_an_explicit_source_language() {
        let server = MockServer::json_once(json!({ "translations": [{ "text": "你好" }] }));
        let request = request("en-GB", official_options(server.base_url.clone()));

        DeepLProvider
            .translate(
                &request,
                &ProviderCredentials::new(Some("free-key".to_string()), None),
            )
            .unwrap();
        let captured = server.recv();
        server.finish();

        assert_eq!(
            captured.form().get("source_lang").map(String::as_str),
            Some("EN")
        );
    }

    #[test]
    fn web_payload_ties_the_timestamp_to_the_i_count() {
        // 只数小写 i，大写 I 不计入（DeepL 前端就是这么算的）。
        assert_eq!(i_count("Hi it is I"), 3);
        assert_eq!(i_count("HI THERE"), 0);
        let stamped = timestamp(3);
        assert_eq!(stamped % 4, 0, "timestamp 必须能被 i 数量+1 整除");
        // 没有 i 时不做对齐，直接用当前毫秒。
        assert!(timestamp(0) > 0);
    }

    #[test]
    fn web_payload_declares_the_regional_variant_only_for_regional_targets() {
        let regional = web_payload(&request("auto", ProviderOptions::default()), 7, 1_000);
        assert_eq!(
            regional["params"]["commonJobParams"]["regionalVariant"],
            "zh-Hans"
        );
        assert_eq!(
            regional["params"]["lang"]["source_lang_user_selected"],
            "auto"
        );
        assert_eq!(regional["params"]["lang"]["target_lang"], "ZH");
        assert_eq!(regional["params"]["timestamp"], 1_000);
        assert_eq!(regional["method"], "LMT_handle_texts");

        let plain = TranslationRequest::with_options(
            "Hello".to_string(),
            "en".to_string(),
            "de".to_string(),
            TranslationProvider::DeepL,
            ProviderOptions::default(),
            1,
        );
        let payload = web_payload(&plain, 7, 1_000);
        assert!(payload["params"].get("commonJobParams").is_none());
        assert_eq!(payload["params"]["lang"]["source_lang_user_selected"], "EN");
    }

    #[test]
    fn rpc_body_spacing_depends_on_the_request_id() {
        let payload = json!({ "method": "LMT_handle_texts", "id": 24 });
        // (24 + 5) % 29 == 0 —— 命中宽空格排版。
        assert!(rpc_body(&payload, 24).contains("\"method\" : \""));
        assert!(rpc_body(&payload, 25).contains("\"method\": \""));
        assert!(!rpc_body(&payload, 25).contains("\"method\":\""));
    }

    #[test]
    fn web_path_posts_json_rpc_and_parses_the_result() {
        let server = MockServer::json_once(json!({
            "jsonrpc": "2.0",
            "result": { "lang": "EN", "texts": [{ "text": "你好", "alternatives": [] }] }
        }));
        let request = request("auto", web_options(server.base_url.clone()));

        let result = DeepLProvider
            .translate(&request, &ProviderCredentials::default())
            .unwrap();
        let captured = server.recv();
        server.finish();

        assert_eq!(captured.method(), "POST");
        assert_eq!(captured.target(), "/jsonrpc");
        assert_eq!(
            captured.header("content-type").as_deref(),
            Some("application/json")
        );
        let body = captured.json();
        assert_eq!(body["method"], "LMT_handle_texts");
        assert_eq!(body["params"]["texts"][0]["text"], "Hi it is I");
        assert_eq!(result.translated_text, "你好");
        assert_eq!(result.detected_source_language.as_deref(), Some("en"));
    }

    #[test]
    fn web_response_without_a_result_reports_a_broken_endpoint() {
        // 非官方端点改了响应形状时，用户需要看到“接口失效”而不是“配置错误”。
        assert_eq!(
            parse_web(&json!({ "jsonrpc": "2.0", "error": { "code": -32600 } })),
            Err(TranslationError::ProviderEndpointBroken)
        );
        assert_eq!(
            parse_web(&json!({ "result": { "texts": [] } })),
            Err(TranslationError::ProviderEndpointBroken)
        );
    }
}
