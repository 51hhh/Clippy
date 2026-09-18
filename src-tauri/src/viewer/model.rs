use serde::{Deserialize, Serialize};

pub const MAX_PNG_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_WINDOWS: usize = 4;
pub const MAX_PIXELS: u64 = 32 * 1024 * 1024;
pub const MAX_EDGE: u32 = 16_384;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewerHandle {
    pub session_id: String,
    pub snapshot_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewerRequest {
    pub session_id: String,
    pub snapshot_id: String,
    pub request_id: u64,
}
impl ViewerRequest {
    pub fn handle(&self) -> ViewerHandle {
        ViewerHandle {
            session_id: self.session_id.clone(),
            snapshot_id: self.snapshot_id.clone(),
        }
    }
    pub fn reply<T>(self, value: T) -> ViewerReply<T> {
        ViewerReply {
            session_id: self.session_id,
            snapshot_id: self.snapshot_id,
            request_id: self.request_id,
            value,
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerReply<T> {
    pub session_id: String,
    pub snapshot_id: String,
    pub request_id: u64,
    pub value: T,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerSource {
    pub clip_id: Option<i64>,
    pub content_hash: String,
    pub width: u32,
    pub height: u32,
    pub byte_length: usize,
    pub media_type: &'static str,
    pub sensitive: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerLimits {
    pub can_edit: bool,
    pub can_scan: bool,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerPayload {
    pub handle: ViewerHandle,
    pub label: String,
    pub source: ViewerSource,
    // 命中本机内部修订时恢复根图上的累计操作层；普通历史图片保持 None。
    pub initial_project: Option<serde_json::Value>,
    pub limits: ViewerLimits,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerColor {
    pub x: u32,
    pub y: u32,
    pub rgba: [u8; 4],
    pub hex: String,
    pub rgb: String,
}
impl ViewerColor {
    pub fn new(x: u32, y: u32, rgba: [u8; 4]) -> Self {
        Self {
            x,
            y,
            rgba,
            hex: format!("#{:02X}{:02X}{:02X}", rgba[0], rgba[1], rgba[2]),
            rgb: format!("rgb({}, {}, {})", rgba[0], rgba[1], rgba[2]),
        }
    }
}
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewerTranslationOptions {
    pub source_language: Option<String>,
    pub target_language: Option<String>,
    pub providers: Option<Vec<String>>,
}
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextSource {
    Ocr,
    Code,
    Translation,
    Color,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{code}")]
pub struct ViewerError {
    pub code: String,
}
impl ViewerError {
    pub fn new(code: &str) -> Self {
        Self { code: code.into() }
    }
}
impl From<&str> for ViewerError {
    fn from(code: &str) -> Self {
        Self::new(code)
    }
}
pub(super) fn validate_dimensions(width: u32, height: u32) -> Result<(), ViewerError> {
    if width == 0
        || height == 0
        || width > MAX_EDGE
        || height > MAX_EDGE
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err("image_too_large".into());
    }
    Ok(())
}
pub(super) fn is_viewer_label(label: &str) -> bool {
    label.starts_with("image-viewer-")
        && label.len() <= 96
        && label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// 查看器只取得展示所需配置；不暴露存储路径、快捷键和完整服务凭据/项目配置。
#[derive(Debug, Serialize)]
pub struct ViewerSettings {
    pub theme: String,
    pub language: String,
    pub translation_source_language: String,
    pub translation_target_language: String,
    pub translation_services: Vec<ViewerService>,
}
#[derive(Debug, Serialize)]
pub struct ViewerService {
    pub provider: String,
    pub enabled: bool,
    pub endpoint: String,
}
impl From<&crate::models::AppConfig> for ViewerSettings {
    fn from(config: &crate::models::AppConfig) -> Self {
        Self {
            theme: config.theme.clone(),
            language: config.language.clone(),
            translation_source_language: config.translation_source_language.clone(),
            translation_target_language: config.translation_target_language.clone(),
            translation_services: config
                .translation_services
                .iter()
                .map(|service| ViewerService {
                    provider: service.provider.clone(),
                    enabled: service.enabled,
                    // 展示网络目的地即可，URL中的userinfo/path/query可能含私人token。
                    endpoint: url::Url::parse(&service.endpoint)
                        .ok()
                        .filter(|url| matches!(url.scheme(), "http" | "https") && url.has_host())
                        .map(|url| url.origin().ascii_serialization())
                        .unwrap_or_default(),
                })
                .collect(),
        }
    }
}
