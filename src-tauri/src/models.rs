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

/// v2 起翻译配置从单服务字段改为 `translation_services` 列表。
const CURRENT_CONFIG_VERSION: u32 = 2;

fn current_config_version() -> u32 {
    CURRENT_CONFIG_VERSION
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct MainWindowPosition {
    pub x: i32,
    pub y: i32,
}

/// 单个翻译服务的用户配置。空字符串一律表示「用该服务的内置默认值」，
/// 这样新增服务不需要用户先去设置页把端点填一遍。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranslationServiceConfig {
    pub provider: String,
    /// 未启用的服务仍然保留各自的端点/模型，便于用户来回切换而不丢配置。
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub model: String,
    /// Azure 资源区域，仅 Bing 官方 API 使用。
    #[serde(default)]
    pub region: String,
    /// GCP 项目 ID，仅 Google Cloud v3 使用。
    #[serde(default)]
    pub project: String,
}

impl TranslationServiceConfig {
    pub fn new(provider: &str, enabled: bool) -> Self {
        Self {
            provider: provider.to_string(),
            enabled,
            endpoint: String::new(),
            model: String::new(),
            region: String::new(),
            project: String::new(),
        }
    }
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
    #[serde(default = "default_translation_services")]
    pub translation_services: Vec<TranslationServiceConfig>,
    /// v1 的单服务字段，只用于迁移成 `translation_services`，迁移后不再写回配置文件。
    #[serde(default, rename = "translation_provider", skip_serializing)]
    pub legacy_translation_provider: String,
    #[serde(default, rename = "translation_endpoint", skip_serializing)]
    pub legacy_translation_endpoint: String,
    #[serde(default, rename = "translation_model", skip_serializing)]
    pub legacy_translation_model: String,
    #[serde(default = "default_translation_source_language")]
    pub translation_source_language: String,
    #[serde(default = "default_translation_target_language")]
    pub translation_target_language: String,
    /// 备选目标语言，按优先级排列。文本本来就是目标语言时换向到这里的第一个其他语言。
    /// 留空表示用配置里的目标/源语言，因此新增此字段不需要迁移或提升配置版本。
    #[serde(default)]
    pub preferred_languages: Vec<String>,
    #[serde(default)]
    pub main_window_position: Option<MainWindowPosition>,
    /// 截图/Pin 的保存目录，支持 `~` 开头。空表示内置默认（`$HOME/Pictures/Clippy`），
    /// 这样以后改默认值对老用户同样生效，因此新增此字段不需要迁移或提升配置版本。
    #[serde(default)]
    pub screenshot_save_dir: String,
    /// 保存文件名模板，支持 `{prefix}` `{date}` `{time}` `{unix}` `{seq}`。
    /// 扩展名固定为 `.png`，模板只描述主干；空表示内置默认。
    #[serde(default)]
    pub screenshot_filename_template: String,
    /// 框选完成后的默认动作：`"editor"` 直接开编辑器（参考项目的手感），
    /// `"toolbar"` 停在覆盖层上等用户点工具条。认不出的值按 `"editor"` 处理，
    /// 因此新增此字段不需要迁移或提升配置版本。
    #[serde(default = "default_capture_commit_action")]
    pub capture_commit_action: String,
}

/// 截图选区提交后的默认动作。
pub const CAPTURE_COMMIT_ACTION_EDITOR: &str = "editor";
pub const CAPTURE_COMMIT_ACTION_TOOLBAR: &str = "toolbar";

fn default_capture_commit_action() -> String {
    CAPTURE_COMMIT_ACTION_EDITOR.to_string()
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

/// 默认只启用 LibreTranslate：它的公共实例允许匿名调用，新用户不配置也能用。
/// 其余服务预置为未启用，用户在设置页勾选即可，不必手填端点。
fn default_translation_services() -> Vec<TranslationServiceConfig> {
    vec![
        TranslationServiceConfig::new("libretranslate", true),
        TranslationServiceConfig::new("openai_compatible", false),
        TranslationServiceConfig::new("deepl", false),
        TranslationServiceConfig::new("google", false),
        TranslationServiceConfig::new("bing", false),
        TranslationServiceConfig::new("youdao", false),
    ]
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
            translation_services: default_translation_services(),
            legacy_translation_provider: String::new(),
            legacy_translation_endpoint: String::new(),
            legacy_translation_model: String::new(),
            translation_source_language: default_translation_source_language(),
            translation_target_language: default_translation_target_language(),
            preferred_languages: Vec::new(),
            main_window_position: None,
            screenshot_save_dir: String::new(),
            screenshot_filename_template: String::new(),
            capture_commit_action: default_capture_commit_action(),
        }
    }
}

/// v1 每个服务的内置默认端点。迁移时与用户值相同就清空，
/// 否则老配置会把当年的默认地址永久钉住，以后改默认值对老用户无效。
const V1_DEFAULT_ENDPOINTS: [(&str, &str); 2] = [
    ("libretranslate", "https://libretranslate.com"),
    ("openai_compatible", "https://api.openai.com/v1"),
];

impl AppConfig {
    /// 归一化后的选区提交动作。老配置没有这个字段、或者写了认不出的值，
    /// 都按默认的「直接开编辑器」处理，绝不让截图流程卡在一个未知状态。
    pub fn capture_commit_action(&self) -> &'static str {
        if self.capture_commit_action.trim() == CAPTURE_COMMIT_ACTION_TOOLBAR {
            CAPTURE_COMMIT_ACTION_TOOLBAR
        } else {
            CAPTURE_COMMIT_ACTION_EDITOR
        }
    }

    /// 把旧版本配置迁移到当前版本，返回是否发生了改动（调用方据此决定是否回写）。
    pub fn migrate(&mut self) -> bool {
        let mut changed = false;

        if self.translation_services.is_empty() {
            self.translation_services = default_translation_services();
            changed = true;
        }
        if self.version < 2 {
            changed |= self.migrate_translation_services_from_v1();
        }
        if self.version != CURRENT_CONFIG_VERSION {
            self.version = CURRENT_CONFIG_VERSION;
            changed = true;
        }

        changed
    }

    /// v1 只能启用一个服务，迁移后保持同一个服务启用，其余保持未启用。
    fn migrate_translation_services_from_v1(&mut self) -> bool {
        let legacy_provider = self.legacy_translation_provider.trim().to_string();
        if legacy_provider.is_empty() {
            return false;
        }
        let Some(service) = self
            .translation_services
            .iter_mut()
            .find(|service| service.provider == legacy_provider)
        else {
            // 认不出的 provider 名保留默认启用项，不至于让用户完全没有可用服务。
            log::warn!("配置迁移遇到未知翻译服务，保留默认启用项: {legacy_provider}");
            return false;
        };

        service.enabled = true;
        let endpoint = self.legacy_translation_endpoint.trim();
        let is_v1_default = V1_DEFAULT_ENDPOINTS
            .iter()
            .any(|(provider, default)| *provider == legacy_provider && *default == endpoint);
        if !is_v1_default {
            service.endpoint = endpoint.to_string();
        }
        service.model = self.legacy_translation_model.trim().to_string();

        for service in &mut self.translation_services {
            if service.provider != legacy_provider {
                service.enabled = false;
            }
        }
        true
    }

    /// 启用的服务，顺序即设置页与结果卡的展示顺序。
    pub fn enabled_translation_services(&self) -> Vec<&TranslationServiceConfig> {
        self.translation_services
            .iter()
            .filter(|service| service.enabled)
            .collect()
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
