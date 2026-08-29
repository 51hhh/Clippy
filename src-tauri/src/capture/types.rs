use serde::{Deserialize, Serialize};

use crate::translation::types::{TranslationProvider, TranslationResult};

#[derive(Debug, Clone)]
pub(super) struct OverlaySpec {
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowCandidate {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureOverlayPayload {
    pub session_id: String,
    pub monitor_id: u32,
    pub png_base64: String,
    pub logical_width: u32,
    pub logical_height: u32,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub windows: Vec<WindowCandidate>,
    /// 框选完成后覆盖层该做什么：`"editor"` 直接开编辑器，`"toolbar"` 停下等用户点工具条。
    /// 由后端归一化后下发，前端不必再认识配置里可能出现的怪值。
    pub commit_action: &'static str,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSelection {
    pub session_id: String,
    pub monitor_id: u32,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureAction {
    Copy,
    Save,
    Pin,
    Edit,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureActionResult {
    pub action: &'static str,
    pub path: Option<String>,
    pub pin_label: Option<String>,
}

/// 截图选区的本地 OCR 与翻译结果。原图不会跨越此 IPC 边界进入翻译服务。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureTranslationResult {
    pub request_id: u64,
    pub provider: TranslationProvider,
    pub source_text: String,
    pub translated_text: String,
    pub detected_source_language: Option<String>,
    /// 实际使用的目标语言，可能因自动换向与设置里的目标语言不同。
    pub target_language: String,
}

impl CaptureTranslationResult {
    pub fn from_translation(source_text: String, result: TranslationResult) -> Self {
        Self {
            request_id: result.request_id,
            provider: result.provider,
            source_text,
            translated_text: result.translated_text,
            detected_source_language: result.detected_source_language,
            target_language: result.target_language,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_translation_contract_contains_text_but_no_image_payload() {
        let result = CaptureTranslationResult::from_translation(
            "recognized locally".to_string(),
            TranslationResult {
                request_id: 8,
                provider: TranslationProvider::LibreTranslate,
                translated_text: "translated remotely".to_string(),
                detected_source_language: Some("en".to_string()),
                target_language: "zh".to_string(),
            },
        );
        let json = serde_json::to_value(result).unwrap();

        assert_eq!(json["requestId"], 8);
        assert_eq!(json["sourceText"], "recognized locally");
        assert_eq!(json["translatedText"], "translated remotely");
        assert_eq!(json["provider"], "libretranslate");
        assert!(json.get("pngBase64").is_none());
        assert!(json.get("image").is_none());
    }
}
