use crate::models::AppConfig;
use crate::private_files::{restrict_directory, restrict_file, write_private};
use std::fs;
use std::path::Path;

pub fn load_config(config_path: &Path) -> AppConfig {
    if let Some(parent) = config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        if let Err(error) = fs::create_dir_all(parent).and_then(|_| restrict_directory(parent)) {
            log::warn!("配置目录创建或权限设置失败: {}", error);
        }
    }
    if config_path.exists() {
        if let Err(error) = restrict_file(config_path) {
            log::warn!("配置文件权限设置失败: {}", error);
        }
        let mut config = match fs::read_to_string(config_path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
                log::warn!("配置文件解析失败，使用默认配置: {}", e);
                AppConfig::default()
            }),
            Err(e) => {
                log::warn!("配置文件读取失败，使用默认配置: {}", e);
                AppConfig::default()
            }
        };
        // 迁移后立刻回写，否则每次启动都要重算一遍，旧字段也会一直留在文件里。
        if config.migrate() {
            save_config(config_path, &config);
        }
        config
    } else {
        let config = AppConfig::default();
        save_config(config_path, &config);
        config
    }
}

pub fn save_config(config_path: &Path, config: &AppConfig) {
    if let Some(parent) = config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        if let Err(error) = fs::create_dir_all(parent).and_then(|_| restrict_directory(parent)) {
            log::error!("配置目录创建或权限设置失败: {}", error);
            return;
        }
    }
    match serde_json::to_string_pretty(config) {
        Ok(json) => {
            let temporary_path = config_path.with_extension("tmp");
            if let Err(error) = write_private(&temporary_path, json.as_bytes())
                .and_then(|_| fs::rename(&temporary_path, config_path))
                .and_then(|_| crate::private_files::restrict_file(config_path))
            {
                log::error!("配置文件写入失败: {}", error);
            }
        }
        Err(e) => {
            log::error!("配置序列化失败: {}", e);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 单元测试
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.max_history, 100);
        assert_eq!(config.storage_mode, "persistent");
        assert_eq!(config.global_shortcut, "Alt+V");
        assert_eq!(config.pin_shortcut, "Ctrl+2");
        assert_eq!(config.capture_shortcut, "Ctrl+Shift+S");
        assert_eq!(config.theme, "light");
        assert_eq!(config.language, "auto");
        assert_eq!(config.translation_source_language, "auto");
        assert_eq!(config.translation_target_language, "en");
        // 默认只启用 LibreTranslate，其余服务预置但未启用。
        let enabled: Vec<&str> = config
            .enabled_translation_services()
            .iter()
            .map(|service| service.provider.as_str())
            .collect();
        assert_eq!(enabled, ["libretranslate"]);
        assert_eq!(config.translation_services.len(), 6);
        assert!(config
            .translation_services
            .iter()
            .all(|service| service.endpoint.is_empty()));
    }

    #[test]
    fn v1_single_service_config_migrates_into_the_service_list() {
        let dir = tempdir().expect("创建临时目录失败");
        let config_path = dir.path().join("config.json");
        let v1 = serde_json::json!({
            "version": 1,
            "max_history": 100,
            "storage_mode": "persistent",
            "global_shortcut": "Alt+V",
            "theme": "light",
            "translation_provider": "openai_compatible",
            "translation_endpoint": "https://api.openai.com/v1",
            "translation_model": "gpt-4o-mini",
            "translation_target_language": "zh",
        });
        fs::write(&config_path, v1.to_string()).unwrap();

        let loaded = load_config(&config_path);
        assert_eq!(loaded.version, 2);
        let enabled = loaded.enabled_translation_services();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].provider, "openai_compatible");
        assert_eq!(enabled[0].model, "gpt-4o-mini");
        // 用户没改过端点，迁移后留空以便将来跟随内置默认值。
        assert!(enabled[0].endpoint.is_empty());
        assert_eq!(loaded.translation_target_language, "zh");

        // 迁移结果已回写，v1 的单服务字段不再留在文件里。
        let written = fs::read_to_string(&config_path).unwrap();
        assert!(!written.contains("\"translation_provider\""));
        assert!(written.contains("\"translation_services\""));
    }

    #[test]
    fn v1_custom_endpoint_survives_the_migration() {
        let dir = tempdir().expect("创建临时目录失败");
        let config_path = dir.path().join("config.json");
        let v1 = serde_json::json!({
            "version": 1,
            "max_history": 100,
            "storage_mode": "persistent",
            "global_shortcut": "Alt+V",
            "theme": "light",
            "translation_provider": "libretranslate",
            "translation_endpoint": "https://libretranslate.example.com",
        });
        fs::write(&config_path, v1.to_string()).unwrap();

        let enabled_endpoint = load_config(&config_path)
            .enabled_translation_services()
            .first()
            .map(|service| service.endpoint.clone());
        assert_eq!(
            enabled_endpoint.as_deref(),
            Some("https://libretranslate.example.com")
        );
    }

    #[test]
    fn capture_commit_action_defaults_to_the_editor_for_old_configs() {
        let dir = tempdir().expect("创建临时目录失败");
        let config_path = dir.path().join("config.json");
        // 老配置里没有这个字段
        let older = serde_json::json!({
            "version": 2,
            "max_history": 100,
            "storage_mode": "persistent",
            "global_shortcut": "Alt+V",
            "theme": "light",
        });
        fs::write(&config_path, older.to_string()).unwrap();
        assert_eq!(load_config(&config_path).capture_commit_action(), "editor");

        let mut config = AppConfig::default();
        assert_eq!(config.capture_commit_action(), "editor");
        config.capture_commit_action = "toolbar".to_string();
        assert_eq!(config.capture_commit_action(), "toolbar");
        // 认不出的值不能让截图流程停在未知状态
        config.capture_commit_action = "whatever".to_string();
        assert_eq!(config.capture_commit_action(), "editor");
    }

    #[test]
    fn migration_is_idempotent_once_the_version_matches() {
        let mut config = AppConfig::default();
        assert!(!config.migrate(), "当前版本配置不该再被判定为需要迁移");
    }

    #[test]
    fn test_load_missing_creates_default() {
        let dir = tempdir().expect("创建临时目录失败");
        let config_path = dir.path().join("config.json");

        // 文件不存在，load_config 应创建并返回默认值
        assert!(!config_path.exists());
        let config = load_config(&config_path);

        // 返回值是默认配置
        assert_eq!(config.max_history, AppConfig::default().max_history);
        assert_eq!(config.theme, AppConfig::default().theme);

        // 文件已被创建
        assert!(
            config_path.exists(),
            "load_config 应在文件不存在时写出默认配置"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&config_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempdir().expect("创建临时目录失败");
        let config_path = dir.path().join("config.json");

        let config = AppConfig {
            max_history: 200,
            theme: "dark".to_string(),
            ..AppConfig::default()
        };

        save_config(&config_path, &config);
        assert!(config_path.exists(), "save_config 应写出文件");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&config_path, fs::Permissions::from_mode(0o664)).unwrap();
            save_config(&config_path, &config);
            assert_eq!(
                fs::metadata(&config_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        let loaded = load_config(&config_path);
        assert_eq!(loaded.max_history, 200);
        assert_eq!(loaded.theme, "dark");
        assert_eq!(loaded.storage_mode, config.storage_mode);
        assert_eq!(loaded.global_shortcut, config.global_shortcut);
        assert_eq!(loaded.pin_shortcut, config.pin_shortcut);
        assert_eq!(loaded.capture_shortcut, config.capture_shortcut);
    }

    #[cfg(unix)]
    #[test]
    fn test_load_repairs_existing_config_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().expect("创建临时目录失败");
        let config_path = dir.path().join("config.json");
        let json = serde_json::to_string(&AppConfig::default()).unwrap();
        fs::write(&config_path, json).unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&config_path, fs::Permissions::from_mode(0o664)).unwrap();

        let loaded = load_config(&config_path);
        assert_eq!(loaded.max_history, AppConfig::default().max_history);
        assert_eq!(
            fs::metadata(config_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}
