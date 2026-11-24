use serde::{Deserialize, Serialize};
use std::fmt;

/// v2 起翻译配置从单服务字段改为 `translation_services` 列表。
pub const CURRENT_CONFIG_VERSION: u32 = 2;

fn current_config_version() -> u32 {
    CURRENT_CONFIG_VERSION
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct MainWindowPosition {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranslationServiceConfig {
    pub provider: String,
    /// 未启用的服务仍保留端点与模型，切换服务不会丢失用户配置。
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub region: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppConfig {
    #[serde(default = "current_config_version")]
    pub version: u32,
    pub max_history: u32,
    pub storage_mode: String,
    #[serde(default = "default_global_shortcut")]
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
    /// 空值保留开发环境变量兜底，应用不会自动下载 OCR 模型。
    #[serde(default)]
    pub enhanced_ocr_manifest_path: String,
    #[serde(default)]
    pub tmux_capture: bool,
    #[serde(default = "default_auto_paste")]
    pub auto_paste: bool,
    #[serde(default = "default_translation_services")]
    pub translation_services: Vec<TranslationServiceConfig>,
    /// v1 字段只参与迁移，当前格式不再写回。
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
    /// 文本已经是目标语言时，优先换向到这里的第一个其他语言。
    #[serde(default)]
    pub preferred_languages: Vec<String>,
    #[serde(default)]
    pub main_window_position: Option<MainWindowPosition>,
    #[serde(default)]
    pub screenshot_save_dir: String,
    /// 支持 `{prefix}`、`{date}`、`{time}`、`{unix}`、`{seq}`，扩展名固定为 `.png`。
    #[serde(default)]
    pub screenshot_filename_template: String,
    /// 窗口速选扩展提示只出现一次，避免用户选择不安装后被反复打扰。
    #[serde(default)]
    pub capture_probe_hint_shown: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShortcutStyle {
    Standard,
    Macos,
}

fn current_shortcut_style() -> ShortcutStyle {
    if cfg!(target_os = "macos") {
        ShortcutStyle::Macos
    } else {
        ShortcutStyle::Standard
    }
}

fn default_shortcuts_for(style: ShortcutStyle) -> (&'static str, &'static str, &'static str) {
    match style {
        ShortcutStyle::Macos => ("Command+Shift+V", "Command+2", "Command+Shift+S"),
        ShortcutStyle::Standard => ("Alt+V", "Ctrl+2", "Ctrl+Shift+S"),
    }
}

fn default_global_shortcut() -> String {
    default_shortcuts_for(current_shortcut_style())
        .0
        .to_string()
}

fn default_pin_shortcut() -> String {
    default_shortcuts_for(current_shortcut_style())
        .1
        .to_string()
}

fn default_capture_shortcut() -> String {
    default_shortcuts_for(current_shortcut_style())
        .2
        .to_string()
}

fn default_language() -> String {
    "auto".to_string()
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
        let (global_shortcut, pin_shortcut, capture_shortcut) =
            default_shortcuts_for(current_shortcut_style());
        Self {
            version: CURRENT_CONFIG_VERSION,
            max_history: 100,
            storage_mode: "persistent".to_string(),
            global_shortcut: global_shortcut.to_string(),
            pin_shortcut: pin_shortcut.to_string(),
            capture_shortcut: capture_shortcut.to_string(),
            theme: "light".to_string(),
            language: "auto".to_string(),
            delete_confirm_ms: 1200,
            ocr_result_mode: "preview".to_string(),
            ocr_enabled: true,
            enhanced_ocr_manifest_path: String::new(),
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
            capture_probe_hint_shown: false,
        }
    }
}

/// 宿主根据该结果决定是否记录告警、是否回写配置文件。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigMigration {
    pub changed: bool,
    pub unknown_legacy_translation_provider: Option<String>,
}

const V1_DEFAULT_ENDPOINTS: [(&str, &str); 2] = [
    ("libretranslate", "https://libretranslate.com"),
    ("openai_compatible", "https://api.openai.com/v1"),
];

impl AppConfig {
    pub fn migrate(&mut self) -> ConfigMigration {
        let mut outcome = ConfigMigration::default();

        if self.translation_services.is_empty() {
            self.translation_services = default_translation_services();
            outcome.changed = true;
        }
        if self.version < 2 {
            match self.migrate_translation_services_from_v1() {
                Ok(changed) => outcome.changed |= changed,
                Err(provider) => outcome.unknown_legacy_translation_provider = Some(provider),
            }
            self.legacy_translation_provider.clear();
            self.legacy_translation_endpoint.clear();
            self.legacy_translation_model.clear();
        }
        if self.version != CURRENT_CONFIG_VERSION {
            self.version = CURRENT_CONFIG_VERSION;
            outcome.changed = true;
        }

        outcome
    }

    fn migrate_translation_services_from_v1(&mut self) -> Result<bool, String> {
        let legacy_provider = self.legacy_translation_provider.trim().to_string();
        if legacy_provider.is_empty() {
            return Ok(false);
        }
        let Some(service) = self
            .translation_services
            .iter_mut()
            .find(|service| service.provider == legacy_provider)
        else {
            return Err(legacy_provider);
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
        Ok(true)
    }

    pub fn enabled_translation_services(&self) -> Vec<&TranslationServiceConfig> {
        self.translation_services
            .iter()
            .filter(|service| service.enabled)
            .collect()
    }
}

pub fn parse_config_json(json: &str) -> serde_json::Result<AppConfig> {
    serde_json::from_str(json)
}

pub fn serialize_config_json(config: &AppConfig) -> serde_json::Result<String> {
    serde_json::to_string_pretty(config)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigCommitError {
    Persist(String),
    Apply {
        error: String,
        persistence_rollback: Option<String>,
        effects_rollback: Option<String>,
    },
}

impl fmt::Display for ConfigCommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Persist(error) => formatter.write_str(error),
            Self::Apply {
                error,
                persistence_rollback,
                effects_rollback,
            } => {
                formatter.write_str(error)?;
                if let Some(reason) = persistence_rollback {
                    write!(formatter, "; 配置回滚失败: {reason}")?;
                }
                if let Some(reason) = effects_rollback {
                    write!(formatter, "; 快捷键/外部设置恢复失败: {reason}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ConfigCommitError {}

/// 先持久化，再应用宿主副作用；副作用失败时补偿磁盘与外部状态。
pub fn commit_config_change<T>(
    current: &mut AppConfig,
    next: AppConfig,
    mut persist: impl FnMut(&AppConfig) -> Result<(), String>,
    mut apply: impl FnMut(&AppConfig) -> Result<T, String>,
) -> Result<T, ConfigCommitError> {
    persist(&next).map_err(ConfigCommitError::Persist)?;
    match apply(&next) {
        Ok(status) => {
            *current = next;
            Ok(status)
        }
        Err(error) => Err(ConfigCommitError::Apply {
            persistence_rollback: persist(current).err(),
            effects_rollback: apply(current).err(),
            error,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_shortcut_styles_are_stable() {
        assert_eq!(
            default_shortcuts_for(ShortcutStyle::Macos),
            ("Command+Shift+V", "Command+2", "Command+Shift+S")
        );
        assert_eq!(
            default_shortcuts_for(ShortcutStyle::Standard),
            ("Alt+V", "Ctrl+2", "Ctrl+Shift+S")
        );
        let config = AppConfig::default();
        assert_eq!(config.max_history, 100);
        assert_eq!(config.storage_mode, "persistent");
        assert_eq!(config.theme, "light");
        assert_eq!(config.language, "auto");
        assert_eq!(config.translation_services.len(), 6);
        assert_eq!(
            config
                .enabled_translation_services()
                .iter()
                .map(|service| service.provider.as_str())
                .collect::<Vec<_>>(),
            ["libretranslate"]
        );
    }

    #[test]
    fn v1_service_migrates_and_round_trips_without_legacy_fields() {
        let mut config = parse_config_json(
            r#"{
                "version": 1,
                "max_history": 100,
                "storage_mode": "persistent",
                "theme": "light",
                "translation_provider": "openai_compatible",
                "translation_endpoint": "https://api.openai.com/v1",
                "translation_model": "gpt-4o-mini",
                "translation_target_language": "zh"
            }"#,
        )
        .unwrap();

        let outcome = config.migrate();
        assert!(outcome.changed);
        assert_eq!(outcome.unknown_legacy_translation_provider, None);
        let enabled = config.enabled_translation_services();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].provider, "openai_compatible");
        assert_eq!(enabled[0].model, "gpt-4o-mini");
        assert!(enabled[0].endpoint.is_empty());

        let encoded = serialize_config_json(&config).unwrap();
        assert!(!encoded.contains("translation_provider"));
        let mut round_trip = parse_config_json(&encoded).unwrap();
        assert_eq!(round_trip.migrate(), ConfigMigration::default());
        assert_eq!(round_trip, config);
    }

    #[test]
    fn migration_preserves_custom_endpoint_and_reports_unknown_provider() {
        let mut custom = AppConfig {
            version: 1,
            legacy_translation_provider: "libretranslate".into(),
            legacy_translation_endpoint: "https://translate.example.com".into(),
            ..AppConfig::default()
        };
        assert!(custom.migrate().changed);
        assert_eq!(
            custom.enabled_translation_services()[0].endpoint,
            "https://translate.example.com"
        );

        let mut unknown = AppConfig {
            version: 1,
            legacy_translation_provider: "retired-provider".into(),
            ..AppConfig::default()
        };
        let outcome = unknown.migrate();
        assert!(outcome.changed);
        assert_eq!(
            outcome.unknown_legacy_translation_provider.as_deref(),
            Some("retired-provider")
        );
        assert_eq!(
            unknown.enabled_translation_services()[0].provider,
            "libretranslate"
        );
    }

    #[test]
    fn failed_persist_does_not_apply_or_replace_memory() {
        let mut current = AppConfig::default();
        let mut next = current.clone();
        next.max_history = 333;
        let error = commit_config_change(
            &mut current,
            next,
            |_| Err("disk full".into()),
            |_| -> Result<(), String> { panic!("落盘失败后不应执行外部操作") },
        )
        .unwrap_err();
        assert_eq!(error, ConfigCommitError::Persist("disk full".into()));
        assert_eq!(current.max_history, 100);
    }

    #[test]
    fn effect_failure_rolls_back_and_success_commits_memory() {
        let mut current = AppConfig::default();
        let mut next = current.clone();
        next.max_history = 333;
        let mut saved = Vec::new();
        let mut applied = Vec::new();
        let error = commit_config_change(
            &mut current,
            next.clone(),
            |value| {
                saved.push(value.max_history);
                Ok(())
            },
            |value| {
                applied.push(value.max_history);
                if value.max_history == 333 {
                    Err("shortcut conflict".into())
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        assert_eq!(saved, [333, 100]);
        assert_eq!(applied, [333, 100]);
        assert_eq!(error.to_string(), "shortcut conflict");
        assert_eq!(current.max_history, 100);

        assert_eq!(
            commit_config_change(&mut current, next, |_| Ok(()), |_| Ok("applied")).unwrap(),
            "applied"
        );
        assert_eq!(current.max_history, 333);
    }

    #[test]
    fn rollback_failures_remain_structured_and_visible() {
        let mut current = AppConfig::default();
        let mut next = current.clone();
        next.max_history = 333;
        let error = commit_config_change(
            &mut current,
            next,
            |value| {
                if value.max_history == 100 {
                    Err("restore disk".into())
                } else {
                    Ok(())
                }
            },
            |value| -> Result<(), String> {
                if value.max_history == 100 {
                    Err("restore effects".into())
                } else {
                    Err("register".into())
                }
            },
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ConfigCommitError::Apply {
                persistence_rollback: Some(_),
                effects_rollback: Some(_),
                ..
            }
        ));
        assert!(error.to_string().contains("配置回滚失败: restore disk"));
        assert!(error
            .to_string()
            .contains("快捷键/外部设置恢复失败: restore effects"));
        assert_eq!(current.max_history, 100);
    }
}
