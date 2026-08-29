//! 微软翻译适配：Azure Translator v3 官方 API + 未配置密钥时的 web 回退。
//!
//! web 回退要先抓一次翻译页拿到 IG/IID 与防滥用 token，再带着它们调 ttranslatev3。
//! 参考项目额外实现了查词与 lookup 路径，Clippy 的结果模型没有词典字段，因此不实现。
//! web 路径没有可用性承诺，代价见 docs/reference-project-guidance.md。

use super::super::http::HttpRequest;
use super::super::service::ProviderClient;
use super::super::types::{
    ProviderCredentials, ProviderTranslation, TranslationError, TranslationRequest,
};
use super::routing::{route, Route};
use serde_json::{json, Value};

const API_VERSION: &str = "3.0";
/// web 端点会对陌生 UA 直接返回验证页，必须伪装成浏览器。
const WEB_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";

pub(crate) struct BingProvider;

impl ProviderClient for BingProvider {
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
    // Azure 的多区域资源必须带区域头，全局资源用 global。
    let region = request.region().unwrap_or("global");
    let mut query: Vec<(&str, &str)> = vec![
        ("api-version", API_VERSION),
        ("to", &request.target_language),
    ];
    if let Some(source) = request.source() {
        query.push(("from", source));
    }

    let response = HttpRequest::post(format!("{}/translate", request.endpoint()))
        .query(&query)
        .header("Ocp-Apim-Subscription-Key", api_key)
        .header("Ocp-Apim-Subscription-Region", region)
        .json(&json!([{ "Text": request.text }]))
        .send_json()?;
    parse_translations(&response, TranslationError::InvalidResponse)
}

fn translate_web(request: &TranslationRequest) -> Result<ProviderTranslation, TranslationError> {
    let base = request.web_endpoint().to_string();
    let config = fetch_web_config(&base)?;
    // web 端点用 auto-detect 而不是 auto 表示自动检测。
    let from = request.source().unwrap_or("auto-detect");

    let response = HttpRequest::post(format!("{base}/ttranslatev3"))
        .query(&[
            ("isVertical", "1"),
            ("IG", config.ig.as_str()),
            ("IID", config.iid.as_str()),
        ])
        .header("User-Agent", WEB_USER_AGENT)
        .form(&[
            ("text", request.text.as_str()),
            ("to", request.target_language.as_str()),
            ("token", config.token.as_str()),
            ("key", config.key.as_str()),
            ("tryFetchingGenderDebiasedTranslations", "true"),
            ("fromLang", from),
        ])
        .send_json()?;
    parse_translations(&response, TranslationError::ProviderEndpointBroken)
}

/// 官方与 web 路径的响应形状相同，只有失败时报的错误码不同：
/// 官方是响应异常，web 是接口失效。
fn parse_translations(
    response: &Value,
    broken: TranslationError,
) -> Result<ProviderTranslation, TranslationError> {
    let first = response
        .as_array()
        .and_then(|items| items.first())
        .ok_or_else(|| broken.clone())?;
    let translated_text = first
        .get("translations")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| broken.clone())?
        .to_string();
    let detected_source_language = first
        .get("detectedLanguage")
        .and_then(|detected| detected.get("language"))
        .and_then(Value::as_str)
        .map(str::to_string);

    Ok(ProviderTranslation {
        translated_text,
        detected_source_language,
    })
}

/// web 路径所需的一次性参数，全部从翻译页 HTML 里抓。
struct WebConfig {
    ig: String,
    iid: String,
    key: String,
    token: String,
}

fn fetch_web_config(base: &str) -> Result<WebConfig, TranslationError> {
    let html = HttpRequest::get(format!("{base}/translator"))
        .header("Accept", "text/html")
        .header("User-Agent", WEB_USER_AGENT)
        .send_text()?;
    parse_web_config(&html)
}

fn parse_web_config(html: &str) -> Result<WebConfig, TranslationError> {
    let ig =
        capture_between(html, "IG:\"", "\"").ok_or(TranslationError::ProviderEndpointBroken)?;
    let iid = capture_between(html, "data-iid=\"", "\"")
        .ok_or(TranslationError::ProviderEndpointBroken)?;
    let params = capture_between(html, "params_AbusePreventionHelper = [", "]")
        .or_else(|| capture_between(html, "params_AbusePreventionHelper=[", "]"))
        .ok_or(TranslationError::ProviderEndpointBroken)?;

    let mut parts = params
        .split(',')
        .map(|part| part.trim().trim_matches('"'))
        .filter(|part| !part.is_empty());
    let key = parts
        .next()
        .ok_or(TranslationError::ProviderEndpointBroken)?
        .to_string();
    let token = parts
        .next()
        .ok_or(TranslationError::ProviderEndpointBroken)?
        .to_string();

    Ok(WebConfig {
        ig,
        iid,
        key,
        token,
    })
}

fn capture_between(text: &str, prefix: &str, suffix: &str) -> Option<String> {
    let start = text.find(prefix)? + prefix.len();
    let tail = &text[start..];
    let end = tail.find(suffix)?;
    Some(tail[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::super::super::test_support::{MockResponse, MockServer};
    use super::super::super::types::{ProviderOptions, TranslationProvider};
    use super::*;

    const WEB_PAGE: &str = r#"<html><script>var _G = {IG:"ABC123DEF"};</script>
        <div id="tta_input" data-iid="translator.5028"></div>
        <script>params_AbusePreventionHelper = ["1717171717","token-value",3600];</script></html>"#;

    fn request(source: &str, options: ProviderOptions) -> TranslationRequest {
        TranslationRequest::with_options(
            "Hello".to_string(),
            source.to_string(),
            "zh-Hans".to_string(),
            TranslationProvider::Bing,
            options,
            1,
        )
    }

    fn translation_payload() -> Value {
        json!([{
            "detectedLanguage": { "language": "en", "score": 1.0 },
            "translations": [{ "text": "你好", "to": "zh-Hans" }]
        }])
    }

    #[test]
    fn official_path_sends_the_subscription_headers_and_region() {
        let server = MockServer::json_once(translation_payload());
        let request = request(
            "en",
            ProviderOptions {
                endpoint: server.base_url.clone(),
                region: Some("eastasia".to_string()),
                ..ProviderOptions::default()
            },
        );

        let result = BingProvider
            .translate(
                &request,
                &ProviderCredentials::from_api_key(Some("azure-key".to_string())),
            )
            .unwrap();
        let captured = server.recv();
        server.finish();

        assert_eq!(captured.method(), "POST");
        assert!(captured.target().starts_with("/translate?"));
        let query = captured.query();
        assert_eq!(query.get("api-version").map(String::as_str), Some("3.0"));
        assert_eq!(query.get("to").map(String::as_str), Some("zh-Hans"));
        assert_eq!(query.get("from").map(String::as_str), Some("en"));
        assert_eq!(
            captured.header("ocp-apim-subscription-key").as_deref(),
            Some("azure-key")
        );
        assert_eq!(
            captured.header("ocp-apim-subscription-region").as_deref(),
            Some("eastasia")
        );
        assert_eq!(captured.json()[0]["Text"], "Hello");
        assert_eq!(result.translated_text, "你好");
        assert_eq!(result.detected_source_language.as_deref(), Some("en"));
    }

    #[test]
    fn official_path_defaults_to_the_global_region_and_omits_auto_source() {
        let server = MockServer::json_once(translation_payload());
        let request = request(
            "auto",
            ProviderOptions {
                endpoint: server.base_url.clone(),
                ..ProviderOptions::default()
            },
        );

        BingProvider
            .translate(
                &request,
                &ProviderCredentials::from_api_key(Some("azure-key".to_string())),
            )
            .unwrap();
        let captured = server.recv();
        server.finish();

        assert_eq!(
            captured.header("ocp-apim-subscription-region").as_deref(),
            Some("global")
        );
        assert!(!captured.query().contains_key("from"));
    }

    #[test]
    fn web_config_is_scraped_from_the_translator_page() {
        let config = parse_web_config(WEB_PAGE).unwrap();
        assert_eq!(config.ig, "ABC123DEF");
        assert_eq!(config.iid, "translator.5028");
        assert_eq!(config.key, "1717171717");
        assert_eq!(config.token, "token-value");

        // 页面结构变了就是接口失效，而不是用户配置错误。
        assert_eq!(
            parse_web_config("<html>no tokens here</html>")
                .err()
                .map(|error| error.code()),
            Some("provider_endpoint_broken")
        );
        assert_eq!(
            parse_web_config(r#"<html>IG:"A" data-iid="b"</html>"#)
                .err()
                .map(|error| error.code()),
            Some("provider_endpoint_broken")
        );
    }

    #[test]
    fn web_config_accepts_the_unspaced_assignment() {
        let html = WEB_PAGE.replace(
            "params_AbusePreventionHelper = [",
            "params_AbusePreventionHelper=[",
        );
        assert_eq!(parse_web_config(&html).unwrap().token, "token-value");
    }

    #[test]
    fn web_path_scrapes_the_page_then_posts_the_form() {
        let server = MockServer::new(vec![
            MockResponse::html(WEB_PAGE),
            MockResponse::json(translation_payload()),
        ]);
        let request = request(
            "auto",
            ProviderOptions {
                web_endpoint: server.base_url.clone(),
                ..ProviderOptions::default()
            },
        );

        let result = BingProvider
            .translate(&request, &ProviderCredentials::default())
            .unwrap();
        let page_request = server.recv();
        let translate_request = server.recv();
        server.finish();

        assert_eq!(page_request.method(), "GET");
        assert_eq!(page_request.target(), "/translator");
        assert_eq!(
            page_request.header("user-agent").as_deref(),
            Some(WEB_USER_AGENT)
        );

        assert_eq!(translate_request.method(), "POST");
        let query = translate_request.query();
        assert_eq!(query.get("IG").map(String::as_str), Some("ABC123DEF"));
        assert_eq!(
            query.get("IID").map(String::as_str),
            Some("translator.5028")
        );
        let form = translate_request.form();
        assert_eq!(form.get("text").map(String::as_str), Some("Hello"));
        assert_eq!(form.get("token").map(String::as_str), Some("token-value"));
        assert_eq!(form.get("key").map(String::as_str), Some("1717171717"));
        // 自动检测在 web 端点上叫 auto-detect。
        assert_eq!(
            form.get("fromLang").map(String::as_str),
            Some("auto-detect")
        );
        assert_eq!(result.translated_text, "你好");
    }

    #[test]
    fn broken_shapes_are_reported_per_path() {
        let empty = json!([]);
        assert_eq!(
            parse_translations(&empty, TranslationError::InvalidResponse),
            Err(TranslationError::InvalidResponse)
        );
        assert_eq!(
            parse_translations(&empty, TranslationError::ProviderEndpointBroken),
            Err(TranslationError::ProviderEndpointBroken)
        );
        assert_eq!(
            parse_translations(
                &json!([{ "translations": [{ "text": " " }] }]),
                TranslationError::ProviderEndpointBroken
            ),
            Err(TranslationError::ProviderEndpointBroken)
        );
    }
}
