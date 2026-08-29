//! Google 翻译适配：Cloud Translation v3 官方 API + 未配置凭据时的 gtx 回退。
//!
//! 参考项目还有一条单词查词的 webapp 路径，只用于补充音标和词性。Clippy 的翻译结果
//! 模型里没有词典字段，那条路径的产出无处存放，因此只实现 v3 与 gtx 两条。
//! gtx 没有可用性承诺，代价见 docs/reference-project-guidance.md。

use super::super::http::HttpRequest;
use super::super::service::ProviderClient;
use super::super::types::{
    ProviderCredentials, ProviderTranslation, TranslationError, TranslationRequest,
};
use super::routing::{route, Route};
use serde_json::{json, Value};

/// v3 的 location 段。区域化端点需要用户改 endpoint，这里固定 global。
const LOCATION: &str = "global";

pub(crate) struct GoogleProvider;

impl ProviderClient for GoogleProvider {
    fn translate(
        &self,
        request: &TranslationRequest,
        credentials: &ProviderCredentials,
    ) -> Result<ProviderTranslation, TranslationError> {
        match route(request.provider, credentials)? {
            Route::Official { key, .. } => translate_cloud(request, key),
            Route::Web => translate_gtx(request),
        }
    }
}

fn translate_cloud(
    request: &TranslationRequest,
    api_key: &str,
) -> Result<ProviderTranslation, TranslationError> {
    // v3 的 URL 里必须带项目 ID。有密钥却没填项目时报凭据不完整，
    // 而不是悄悄退回 gtx —— 否则用户永远发现不了自己的 Cloud 配置没生效。
    let project = request
        .project()
        .ok_or(TranslationError::IncompleteCredentials)?;
    let url = format!(
        "{}/v3/projects/{project}/locations/{LOCATION}:translateText",
        request.endpoint()
    );

    let response = HttpRequest::post(url)
        .query(&[("key", api_key)])
        .json(&cloud_body(request))
        .send_json()?;
    parse_cloud(&response)
}

fn cloud_body(request: &TranslationRequest) -> Value {
    let mut body = json!({
        "targetLanguageCode": request.target_language,
        "contents": [request.text],
        "mimeType": "text/plain",
    });
    if let Some(source) = request.source() {
        body["sourceLanguageCode"] = json!(source);
    }
    body
}

fn parse_cloud(response: &Value) -> Result<ProviderTranslation, TranslationError> {
    let first = response
        .get("translations")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .ok_or(TranslationError::InvalidResponse)?;
    let translated_text = first
        .get("translatedText")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or(TranslationError::InvalidResponse)?
        .to_string();
    let detected_source_language = first
        .get("detectedLanguageCode")
        .and_then(Value::as_str)
        .map(str::to_string);

    Ok(ProviderTranslation {
        translated_text,
        detected_source_language,
    })
}

fn translate_gtx(request: &TranslationRequest) -> Result<ProviderTranslation, TranslationError> {
    let source = request
        .source()
        .map(gtx_language)
        .unwrap_or_else(|| "auto".to_string());
    let target = gtx_language(&request.target_language);
    let response = HttpRequest::get(format!("{}/translate_a/single", request.web_endpoint()))
        .query(&[
            ("q", request.text.as_str()),
            ("sl", source.as_str()),
            ("tl", target.as_str()),
            // dt=t 只要译文，dj=1 让响应是对象而不是嵌套数组。
            ("dt", "t"),
            ("dj", "1"),
            ("ie", "UTF-8"),
            ("client", "gtx"),
        ])
        .send_json()?;
    parse_gtx(&response)
}

/// gtx 用的是旧式语言 ID（`zh-Hans` → `zh-CN`）。
fn gtx_language(code: &str) -> String {
    match code.trim().to_ascii_lowercase().as_str() {
        "zh-hans" | "zh-cn" => "zh-CN".to_string(),
        "zh-hant" | "zh-tw" | "zh-hk" => "zh-TW".to_string(),
        "" => "auto".to_string(),
        other => other.to_string(),
    }
}

/// gtx 会把长文本切成多个 sentence，必须按顺序拼接才是完整译文。
fn parse_gtx(response: &Value) -> Result<ProviderTranslation, TranslationError> {
    let sentences = response
        .get("sentences")
        .and_then(Value::as_array)
        .ok_or(TranslationError::ProviderEndpointBroken)?;
    let translated_text = sentences
        .iter()
        .filter_map(|sentence| sentence.get("trans").and_then(Value::as_str))
        .collect::<String>()
        .trim()
        .to_string();
    if translated_text.is_empty() {
        return Err(TranslationError::ProviderEndpointBroken);
    }
    let detected_source_language = response
        .get("src")
        .and_then(Value::as_str)
        .map(str::to_string);

    Ok(ProviderTranslation {
        translated_text,
        detected_source_language,
    })
}

#[cfg(test)]
mod tests {
    use super::super::super::test_support::MockServer;
    use super::super::super::types::{ProviderOptions, TranslationProvider};
    use super::*;

    fn request(source: &str, options: ProviderOptions) -> TranslationRequest {
        TranslationRequest::with_options(
            "Hello world".to_string(),
            source.to_string(),
            "zh-Hans".to_string(),
            TranslationProvider::Google,
            options,
            1,
        )
    }

    #[test]
    fn cloud_body_omits_the_source_when_detecting() {
        let auto = cloud_body(&request("auto", ProviderOptions::default()));
        assert_eq!(auto["targetLanguageCode"], "zh-Hans");
        assert_eq!(auto["contents"][0], "Hello world");
        assert_eq!(auto["mimeType"], "text/plain");
        assert!(auto.get("sourceLanguageCode").is_none());

        let explicit = cloud_body(&request("en", ProviderOptions::default()));
        assert_eq!(explicit["sourceLanguageCode"], "en");
    }

    #[test]
    fn cloud_path_sends_the_key_as_a_query_parameter() {
        let server = MockServer::json_once(json!({
            "translations": [{ "translatedText": "你好世界", "detectedLanguageCode": "en" }]
        }));
        let request = request(
            "auto",
            ProviderOptions {
                endpoint: server.base_url.clone(),
                project: Some("my-gcp-project".to_string()),
                ..ProviderOptions::default()
            },
        );

        let result = GoogleProvider
            .translate(
                &request,
                &ProviderCredentials::new(Some("cloud-key".to_string()), None),
            )
            .unwrap();
        let captured = server.recv();
        server.finish();

        assert_eq!(captured.method(), "POST");
        assert!(captured
            .target()
            .starts_with("/v3/projects/my-gcp-project/locations/global:translateText?"));
        assert_eq!(
            captured.query().get("key").map(String::as_str),
            Some("cloud-key")
        );
        assert_eq!(captured.json()["contents"][0], "Hello world");
        assert_eq!(result.translated_text, "你好世界");
        assert_eq!(result.detected_source_language.as_deref(), Some("en"));
    }

    #[test]
    fn cloud_path_requires_the_project_id_instead_of_falling_back() {
        let request = request(
            "auto",
            ProviderOptions {
                endpoint: "https://translation.googleapis.test".to_string(),
                ..ProviderOptions::default()
            },
        );
        assert_eq!(
            GoogleProvider.translate(
                &request,
                &ProviderCredentials::new(Some("cloud-key".to_string()), None)
            ),
            Err(TranslationError::IncompleteCredentials)
        );
    }

    #[test]
    fn gtx_language_ids_use_the_legacy_chinese_codes() {
        assert_eq!(gtx_language("zh-Hans"), "zh-CN");
        assert_eq!(gtx_language("zh-TW"), "zh-TW");
        assert_eq!(gtx_language("ZH-HANT"), "zh-TW");
        assert_eq!(gtx_language(""), "auto");
        assert_eq!(gtx_language("de"), "de");
    }

    #[test]
    fn gtx_path_joins_every_sentence_in_order() {
        let server = MockServer::json_once(json!({
            "sentences": [{ "trans": "你好" }, { "trans": "世界" }, { "orig": "x" }],
            "src": "en"
        }));
        let request = request(
            "auto",
            ProviderOptions {
                web_endpoint: server.base_url.clone(),
                ..ProviderOptions::default()
            },
        );

        let result = GoogleProvider
            .translate(&request, &ProviderCredentials::default())
            .unwrap();
        let captured = server.recv();
        server.finish();

        assert_eq!(captured.method(), "GET");
        assert!(captured.target().starts_with("/translate_a/single?"));
        let query = captured.query();
        assert_eq!(query.get("q").map(String::as_str), Some("Hello world"));
        assert_eq!(query.get("sl").map(String::as_str), Some("auto"));
        assert_eq!(query.get("tl").map(String::as_str), Some("zh-CN"));
        assert_eq!(query.get("dj").map(String::as_str), Some("1"));
        assert_eq!(query.get("client").map(String::as_str), Some("gtx"));
        assert_eq!(result.translated_text, "你好世界");
        assert_eq!(result.detected_source_language.as_deref(), Some("en"));
    }

    #[test]
    fn gtx_response_without_sentences_reports_a_broken_endpoint() {
        assert_eq!(
            parse_gtx(&json!({ "src": "en" })),
            Err(TranslationError::ProviderEndpointBroken)
        );
        assert_eq!(
            parse_gtx(&json!({ "sentences": [{ "trans": "  " }] })),
            Err(TranslationError::ProviderEndpointBroken)
        );
    }
}
