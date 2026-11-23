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
            Self::Text => "text",
            Self::Html => "html",
            Self::Image => "image",
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

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "text" => Ok(Self::Text),
            "html" => Ok(Self::Html),
            "image" => Ok(Self::Image),
            other => Err(format!("未知内容类型: {other}")),
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

impl ClipItem {
    pub const LIST_TEXT_MAX_CHARS: usize = 2_048;

    /// 列表与实时事件只携带有界摘要；完整正文由详情命令按当前焦点读取。
    pub fn for_list(mut self) -> Self {
        self.image_data = None;
        self.html_content = None;
        if let Some(text) = self.text_content.as_mut() {
            if let Some((byte_index, _)) = text.char_indices().nth(Self::LIST_TEXT_MAX_CHARS) {
                text.truncate(byte_index);
            }
        }
        self
    }
}

/// 一条翻译记录。`clip_id` 为 0 表示不来自剪贴板条目（选区翻译或临时文本）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranslationHistoryEntry {
    pub id: i64,
    pub clip_id: i64,
    pub provider: String,
    pub source_language: String,
    pub target_language: String,
    pub source_text: String,
    pub translated_text: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrlMeta {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub favicon: Option<String>,
    pub site_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_type_wire_values_are_stable() {
        for (value, content_type) in [
            ("text", ContentType::Text),
            ("html", ContentType::Html),
            ("image", ContentType::Image),
        ] {
            assert_eq!(content_type.as_str(), value);
            assert_eq!(value.parse::<ContentType>().unwrap(), content_type);
        }
        assert!("video".parse::<ContentType>().is_err());
    }

    #[test]
    fn list_payload_drops_large_fields_and_truncates_on_a_unicode_boundary() {
        let text = format!("{}尾", "界".repeat(ClipItem::LIST_TEXT_MAX_CHARS));
        let clip = ClipItem {
            id: 9,
            content_type: ContentType::Html,
            text_content: Some(text),
            html_content: Some("<p>full</p>".to_string()),
            image_data: Some(vec![1, 2, 3]),
            content_hash: "hash".to_string(),
            is_favorite: false,
            is_sensitive: true,
            created_at: 2,
            byte_size: 99,
        };

        let sent = clip.for_list();
        assert_eq!(
            sent.text_content.unwrap().chars().count(),
            ClipItem::LIST_TEXT_MAX_CHARS
        );
        assert!(sent.html_content.is_none());
        assert!(sent.image_data.is_none());
        assert!(sent.is_sensitive);
    }
}
