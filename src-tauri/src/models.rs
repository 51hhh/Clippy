use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ContentType {
    Text,
    Html,
    Image,
}

impl ContentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContentType::Text => "text",
            ContentType::Html => "html",
            ContentType::Image => "image",
        }
    }
}

impl std::fmt::Display for ContentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ContentType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "text" => Ok(ContentType::Text),
            "html" => Ok(ContentType::Html),
            "image" => Ok(ContentType::Image),
            other => Err(format!("未知内容类型: {}", other)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipItem {
    pub id: i64,
    pub content_type: ContentType,
    pub text_content: Option<String>,
    pub html_content: Option<String>,
    pub image_data: Option<Vec<u8>>,
    pub content_hash: String,
    pub is_favorite: bool,
    pub is_sensitive: bool,
    pub created_at: i64,
    pub byte_size: i64,
}

const CURRENT_CONFIG_VERSION: u32 = 1;

fn current_config_version() -> u32 {
    CURRENT_CONFIG_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "current_config_version")]
    pub version: u32,
    pub max_history: u32,
    pub storage_mode: String,
    pub global_shortcut: String,
    #[serde(default = "default_pin_shortcut")]
    pub pin_shortcut: String,
    #[serde(default = "default_capture_shortcut")]
    pub capture_shortcut: String,
    pub theme: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_delete_confirm_ms")]
    pub delete_confirm_ms: u32,
    #[serde(default = "default_ocr_result_mode")]
    pub ocr_result_mode: String,
    #[serde(default = "default_ocr_enabled")]
    pub ocr_enabled: bool,
    #[serde(default)]
    pub tmux_capture: bool,
    #[serde(default = "default_auto_paste")]
    pub auto_paste: bool,
    #[serde(default = "default_translation_provider")]
    pub translation_provider: String,
    #[serde(default = "default_translation_endpoint")]
    pub translation_endpoint: String,
    #[serde(default = "default_translation_model")]
    pub translation_model: String,
    #[serde(default = "default_translation_source_language")]
    pub translation_source_language: String,
    #[serde(default = "default_translation_target_language")]
    pub translation_target_language: String,
}

fn default_language() -> String {
    "auto".to_string()
}

fn default_pin_shortcut() -> String {
    "Ctrl+2".to_string()
}

fn default_capture_shortcut() -> String {
    "Ctrl+Shift+S".to_string()
}

fn default_delete_confirm_ms() -> u32 {
    1200
}

fn default_ocr_result_mode() -> String {
    "preview".to_string()
}

fn default_ocr_enabled() -> bool {
    true
}

fn default_auto_paste() -> bool {
    true
}

fn default_translation_provider() -> String {
    "libretranslate".to_string()
}

fn default_translation_endpoint() -> String {
    "https://libretranslate.com".to_string()
}

fn default_translation_model() -> String {
    String::new()
}

fn default_translation_source_language() -> String {
    "auto".to_string()
}

fn default_translation_target_language() -> String {
    "en".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CURRENT_CONFIG_VERSION,
            max_history: 100,
            storage_mode: "persistent".to_string(),
            global_shortcut: "Alt+V".to_string(),
            pin_shortcut: "Ctrl+2".to_string(),
            capture_shortcut: "Ctrl+Shift+S".to_string(),
            theme: "light".to_string(),
            language: "auto".to_string(),
            delete_confirm_ms: 1200,
            ocr_result_mode: "preview".to_string(),
            ocr_enabled: true,
            tmux_capture: false,
            auto_paste: true,
            translation_provider: default_translation_provider(),
            translation_endpoint: default_translation_endpoint(),
            translation_model: default_translation_model(),
            translation_source_language: default_translation_source_language(),
            translation_target_language: default_translation_target_language(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrlMeta {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub favicon: Option<String>,
    pub site_name: Option<String>,
}
