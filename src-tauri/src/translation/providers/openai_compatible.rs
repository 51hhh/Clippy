use super::super::http::post_json;
use super::super::service::{append_endpoint_path, ProviderClient};
use super::super::types::{
    ProviderCredentials, ProviderTranslation, TranslationError, TranslationRequest,
};
use serde_json::json;

pub(crate) struct OpenAiCompatibleProvider;

impl ProviderClient for OpenAiCompatibleProvider {
    fn translate(
        &self,
        request: &TranslationRequest,
        credentials: &ProviderCredentials,
    ) -> Result<ProviderTranslation, TranslationError> {
        let api_key = credentials.key().ok_or(TranslationError::MissingApiKey)?;
        let endpoint = append_endpoint_path(request.endpoint(), "/chat/completions");
        let body = request_body(request);

        let response = post_json(&endpoint, &body, Some(api_key))?;
        parse_response(&response)
    }
}

fn request_body(request: &TranslationRequest) -> serde_json::Value {
    let model = request.model().unwrap_or("gpt-4o-mini");
    let source = if request.source_language.eq_ignore_ascii_case("auto") {
        "the source language detected automatically".to_string()
    } else {
        format!("the {} language", request.source_language)
    };
    let system = format!(
        "Translate the user's text from {source} to {}. Return only the translation, with no commentary.",
        request.target_language
    );
    json!({
        "model": model,
        "temperature": 0,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": request.text}
        ]
    })
}

fn parse_response(response: &serde_json::Value) -> Result<ProviderTranslation, TranslationError> {
    let translated_text = response
        .get("choices")
        .and_then(|choices| choices.as_array())
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(serde_json::Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or(TranslationError::InvalidResponse)?
        .to_string();

    Ok(ProviderTranslation {
        translated_text,
        detected_source_language: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translation::types::{ProviderOptions, TranslationProvider};

    fn request() -> TranslationRequest {
        TranslationRequest::with_options(
            "Hello".to_string(),
            "auto".to_string(),
            "zh-CN".to_string(),
            TranslationProvider::OpenAiCompatible,
            ProviderOptions {
                endpoint: "https://example.test/v1".to_string(),
                model: Some("custom-model".to_string()),
                ..ProviderOptions::default()
            },
            8,
        )
    }

    #[test]
    fn payload_keeps_user_text_separate_from_translation_instruction() {
        let body = request_body(&request());
        assert_eq!(body["model"], "custom-model");
        assert_eq!(body["temperature"], 0);
        assert_eq!(body["messages"][0]["role"], "system");
        assert!(body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("detected automatically"));
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "Hello");
    }

    #[test]
    fn response_requires_non_empty_first_message_content() {
        let response = json!({
            "choices": [{ "message": { "content": "你好" } }]
        });
        assert_eq!(
            parse_response(&response).unwrap(),
            ProviderTranslation {
                translated_text: "你好".to_string(),
                detected_source_language: None,
            }
        );
        assert_eq!(
            parse_response(&json!({"choices": []})),
            Err(TranslationError::InvalidResponse)
        );
    }

    #[test]
    fn provider_rejects_missing_key_before_network_access() {
        assert_eq!(
            OpenAiCompatibleProvider.translate(&request(), &ProviderCredentials::default()),
            Err(TranslationError::MissingApiKey)
        );
    }
}
