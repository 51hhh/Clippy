use super::super::http::post_json;
use super::super::service::{append_endpoint_path, ProviderClient};
use super::super::types::{
    ProviderCredentials, ProviderTranslation, TranslationError, TranslationRequest,
};
use serde_json::json;

pub(crate) struct LibreTranslateProvider;

impl ProviderClient for LibreTranslateProvider {
    fn translate(
        &self,
        request: &TranslationRequest,
        credentials: &ProviderCredentials,
    ) -> Result<ProviderTranslation, TranslationError> {
        let endpoint = append_endpoint_path(request.endpoint(), "/translate");
        let body = request_body(request, credentials.key());
        let response = post_json(&endpoint, &body, None)?;
        parse_response(&response)
    }
}

fn request_body(request: &TranslationRequest, api_key: Option<&str>) -> serde_json::Value {
    let mut body = json!({
        "q": request.text,
        "source": request.source_language,
        "target": request.target_language,
        "format": "text"
    });
    if let Some(key) = api_key.filter(|key| !key.is_empty()) {
        body["api_key"] = json!(key);
    }
    body
}

fn parse_response(response: &serde_json::Value) -> Result<ProviderTranslation, TranslationError> {
    let translated_text = response
        .get("translatedText")
        .or_else(|| response.get("translated_text"))
        .and_then(serde_json::Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or(TranslationError::InvalidResponse)?
        .to_string();
    let detected_source_language = response
        .get("detectedLanguage")
        .or_else(|| response.get("detected_source_language"))
        .and_then(|value| {
            value.as_str().map(str::to_string).or_else(|| {
                value
                    .get("language")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
        });

    Ok(ProviderTranslation {
        translated_text,
        detected_source_language,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translation::types::TranslationProvider;

    fn request() -> TranslationRequest {
        TranslationRequest::new(
            "Hello".to_string(),
            "auto".to_string(),
            "zh-CN".to_string(),
            "https://example.test".to_string(),
            TranslationProvider::LibreTranslate,
            None,
            7,
        )
    }

    #[test]
    fn payload_matches_libretranslate_contract() {
        let body = request_body(&request(), Some("test-key"));
        assert_eq!(body["q"], "Hello");
        assert_eq!(body["source"], "auto");
        assert_eq!(body["target"], "zh-CN");
        assert_eq!(body["format"], "text");
        assert_eq!(body["api_key"], "test-key");

        let body_without_key = request_body(&request(), None);
        assert!(body_without_key.get("api_key").is_none());
    }

    #[test]
    fn response_accepts_supported_detected_language_shapes() {
        let object = json!({
            "translatedText": "你好",
            "detectedLanguage": { "language": "en", "confidence": 99 }
        });
        assert_eq!(
            parse_response(&object).unwrap(),
            ProviderTranslation {
                translated_text: "你好".to_string(),
                detected_source_language: Some("en".to_string()),
            }
        );

        let snake_case = json!({
            "translated_text": "Bonjour",
            "detected_source_language": "en"
        });
        assert_eq!(
            parse_response(&snake_case)
                .unwrap()
                .detected_source_language
                .as_deref(),
            Some("en")
        );
        assert_eq!(
            parse_response(&json!({"translatedText": ""})),
            Err(TranslationError::InvalidResponse)
        );
    }
}
