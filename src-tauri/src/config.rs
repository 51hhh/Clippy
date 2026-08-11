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
        match fs::read_to_string(config_path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
                log::warn!("配置文件解析失败，使用默认配置: {}", e);
                AppConfig::default()
            }),
            Err(e) => {
                log::warn!("配置文件读取失败，使用默认配置: {}", e);
                AppConfig::default()
            }
        }
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
        assert_eq!(config.translation_provider, "libretranslate");
        assert_eq!(config.translation_endpoint, "https://libretranslate.com");
        assert!(config.translation_model.is_empty());
        assert_eq!(config.translation_source_language, "auto");
        assert_eq!(config.translation_target_language, "en");
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
