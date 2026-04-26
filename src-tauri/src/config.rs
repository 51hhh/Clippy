use crate::models::AppConfig;
use std::fs;
use std::path::Path;

pub fn load_config(config_path: &Path) -> AppConfig {
    if config_path.exists() {
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
    if let Some(parent) = config_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(config) {
        Ok(json) => {
            if let Err(e) = fs::write(config_path, json) {
                log::error!("配置文件写入失败: {}", e);
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
        assert_eq!(config.theme, "light");
        assert_eq!(config.language, "auto");
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
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempdir().expect("创建临时目录失败");
        let config_path = dir.path().join("config.json");

        let mut config = AppConfig::default();
        config.max_history = 200;
        config.theme = "dark".to_string();

        save_config(&config_path, &config);
        assert!(config_path.exists(), "save_config 应写出文件");

        let loaded = load_config(&config_path);
        assert_eq!(loaded.max_history, 200);
        assert_eq!(loaded.theme, "dark");
        assert_eq!(loaded.storage_mode, config.storage_mode);
        assert_eq!(loaded.global_shortcut, config.global_shortcut);
    }
}
