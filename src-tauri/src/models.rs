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
    pub created_at: i64,
    pub byte_size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub max_history: u32,
    pub storage_mode: String,
    pub global_shortcut: String,
    #[serde(default = "default_pin_shortcut")]
    pub pin_shortcut: String,
    pub theme: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_delete_confirm_ms")]
    pub delete_confirm_ms: u32,
    #[serde(default = "default_ocr_result_mode")]
    pub ocr_result_mode: String,
    #[serde(default = "default_ocr_enabled")]
    pub ocr_enabled: bool,
}

fn default_language() -> String {
    "auto".to_string()
}

fn default_pin_shortcut() -> String {
    "Ctrl+2".to_string()
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

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            max_history: 100,
            storage_mode: "persistent".to_string(),
            global_shortcut: "Alt+V".to_string(),
            pin_shortcut: "Ctrl+2".to_string(),
            theme: "light".to_string(),
            language: "auto".to_string(),
            delete_confirm_ms: 1200,
            ocr_result_mode: "preview".to_string(),
            ocr_enabled: true,
        }
    }
}
