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

impl OverlaySpec {
    /// 光标是否落在这块覆盖层上。用来决定哪个覆盖层拿键盘焦点，
    /// 不能用窗口自身的 `outer_position()`：Wayland 下那是我们请求的位置，不是合成器的实际摆放。
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x as f64
            && x < self.x as f64 + self.width as f64
            && y >= self.y as f64
            && y < self.y as f64 + self.height as f64
    }
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

/// 覆盖层开局需要的一切**除了像素**。
///
/// 冻结帧本身由 `get_capture_frame` 单独交付原始 RGBA：以前这里带一个 `pngBase64`，
/// 于是 2560×1600 的帧要在 Rust 里编码一次 PNG（实测 215 ms）、base64 一次（3 MB 字符串）、
/// 再在 webview 里 atob + 解码一次。像素不走 JSON 之后这两头的开销一起没了，
/// 详见 docs/capture-linux.md §3。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureOverlayPayload {
    pub session_id: String,
    pub monitor_id: u32,
    /// 这块显示器在桌面逻辑坐标系里的左上角。覆盖层的选区坐标是相对自己的，
    /// 加上这个偏移才是"屏幕上的哪一块"——贴图靠它贴回原位（见 `pin::PinOrigin`）。
    pub logical_x: i32,
    pub logical_y: i32,
    pub logical_width: u32,
    pub logical_height: u32,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub windows: Vec<WindowCandidate>,
    /// 这次要不要在覆盖层里提示"窗口速选需要在设置页安装服务"。
    /// 由后端决定并且只置真一次，覆盖层照做即可，自己不判断桌面环境。
    pub probe_hint: bool,
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

/// 覆盖层里点勾/保存/贴图时要做的事。标注已经在覆盖层内完成，
/// 所以没有"转到编辑器"这一项了。
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureAction {
    Copy,
    Save,
    Pin,
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
