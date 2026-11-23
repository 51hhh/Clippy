use crate::models::AppConfig;
use crate::private_files::{
    replace_private_file, restrict_directory, restrict_file, write_private,
};
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
            if let Err(error) = save_config(config_path, &config) {
                log::warn!("迁移配置保存失败: {error}");
            }
        }
        config
    } else {
        let config = AppConfig::default();
        if let Err(error) = save_config(config_path, &config) {
            log::warn!("默认配置保存失败: {error}");
        }
        config
    }
}

pub fn save_config(config_path: &Path, config: &AppConfig) -> std::io::Result<()> {
    if let Some(parent) = config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
        restrict_directory(parent)?;
    }
    let json = serde_json::to_string_pretty(config).map_err(std::io::Error::other)?;
    let temporary_path = config_path.with_extension("tmp");
    let result = write_private(&temporary_path, json.as_bytes())
        .and_then(|_| replace_private_file(&temporary_path, config_path));
    if result.is_err() {
        // 临时文件包含配置；失败后尽力清理，不掩盖原写入错误。
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

/// 持久化成功后才应用外部设置，任一同步副作用失败则回滚磁盘和外部状态。
/// 调用方串行提交配置；传入快照仅在两者均成功时更新。回滚失败也必须呈现给用户。
pub(crate) fn commit_config_change<T>(
    current: &mut AppConfig,
    next: AppConfig,
    mut persist: impl FnMut(&AppConfig) -> Result<(), String>,
    mut apply: impl FnMut(&AppConfig) -> Result<T, String>,
) -> Result<T, String> {
    persist(&next)?;
    match apply(&next) {
        Ok(status) => {
            *current = next;
            Ok(status)
        }
        Err(error) => {
            let disk = persist(current).err();
            let effects = apply(current).err();
            let mut message = error;
            if let Some(reason) = disk {
                message.push_str(&format!("; 配置回滚失败: {reason}"));
            }
            if let Some(reason) = effects {
                message.push_str(&format!("; 快捷键/外部设置恢复失败: {reason}"));
            }
            Err(message)
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
        #[cfg(target_os = "macos")]
        let expected_shortcuts = ("Command+Shift+V", "Command+2", "Command+Shift+S");
        #[cfg(not(target_os = "macos"))]
        let expected_shortcuts = ("Alt+V", "Ctrl+2", "Ctrl+Shift+S");
        assert_eq!(config.global_shortcut, expected_shortcuts.0);
        assert_eq!(config.pin_shortcut, expected_shortcuts.1);
        assert_eq!(config.capture_shortcut, expected_shortcuts.2);
        assert_eq!(config.theme, "light");
        assert_eq!(config.language, "auto");
        assert!(config.enhanced_ocr_manifest_path.is_empty());
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
            enhanced_ocr_manifest_path: "/opt/clippy-ocr/manifest.json".to_string(),
            ..AppConfig::default()
        };

        save_config(&config_path, &config).unwrap();
        assert!(config_path.exists(), "save_config 应写出文件");

        let updated = AppConfig {
            max_history: 321,
            theme: "rose".to_string(),
            ..config.clone()
        };
        save_config(&config_path, &updated).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&config_path, fs::Permissions::from_mode(0o664)).unwrap();
            save_config(&config_path, &updated).unwrap();
            assert_eq!(
                fs::metadata(&config_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        let loaded = load_config(&config_path);
        assert_eq!(loaded.max_history, 321);
        assert_eq!(loaded.theme, "rose");
        assert_eq!(loaded.storage_mode, config.storage_mode);
        assert_eq!(loaded.global_shortcut, config.global_shortcut);
        assert_eq!(loaded.pin_shortcut, config.pin_shortcut);
        assert_eq!(loaded.capture_shortcut, config.capture_shortcut);
        assert_eq!(
            loaded.enhanced_ocr_manifest_path,
            "/opt/clippy-ocr/manifest.json"
        );
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
    #[test]
    fn save_reports_directory_and_replacement_failures() {
        let dir = tempdir().unwrap();
        let parent_file = dir.path().join("not-a-directory");
        fs::write(&parent_file, "fixture").unwrap();
        assert!(save_config(&parent_file.join("config.json"), &AppConfig::default()).is_err());
        let target = dir.path().join("config.json");
        fs::create_dir(&target).unwrap();
        assert!(save_config(&target, &AppConfig::default()).is_err());
        assert!(target.is_dir());
        assert!(!target.with_extension("tmp").exists());
    }

    #[test]
    fn failed_save_does_not_apply_effects_or_change_memory() {
        let mut current = AppConfig::default();
        let mut next = current.clone();
        next.max_history = 333;
        let result = commit_config_change(
            &mut current,
            next,
            |_| Err("disk full".into()),
            |_| -> Result<(), String> { panic!("落盘失败后不应执行外部操作") },
        );
        assert!(result.is_err());
        assert_eq!(current.max_history, 100);
    }

    #[test]
    fn partial_effect_failure_rolls_back_disk_and_bindings() {
        let mut current = AppConfig::default();
        let mut next = current.clone();
        next.max_history = 333;
        let mut saved = Vec::new();
        let mut applied = Vec::new();
        let result = commit_config_change(
            &mut current,
            next,
            |value| {
                saved.push(value.max_history);
                Ok(())
            },
            |value| {
                applied.push(value.max_history);
                if value.max_history == 333 {
                    Err("capture shortcut conflict".into())
                } else {
                    Ok(())
                }
            },
        );
        assert!(result.unwrap_err().contains("capture shortcut conflict"));
        assert_eq!(saved, [333, 100]);
        assert_eq!(applied, [333, 100]);
        assert_eq!(current.max_history, 100);
    }

    #[test]
    fn shortcut_failure_restores_the_real_config_file() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("config.json");
        let mut current = AppConfig::default();
        save_config(&path, &current).unwrap();
        let previous_bytes = fs::read(&path).unwrap();
        let mut next = current.clone();
        next.theme = "dark".into();
        next.max_history = 333;
        let result = commit_config_change(
            &mut current,
            next,
            |value| save_config(&path, value).map_err(|error| error.to_string()),
            |value| {
                if value.max_history == 333 {
                    Err("shortcut conflict".to_string())
                } else {
                    Ok(())
                }
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), previous_bytes);
        assert_eq!(current.theme, "light");
        assert_eq!(load_config(&path).max_history, 100);
    }

    #[test]
    fn rollback_failure_is_returned_and_success_commits_memory() {
        let mut current = AppConfig::default();
        let mut next = current.clone();
        next.max_history = 333;
        let result = commit_config_change(
            &mut current,
            next.clone(),
            |value| {
                if value.max_history == 100 {
                    Err("restore disk".into())
                } else {
                    Ok(())
                }
            },
            |_| -> Result<(), String> { Err("register".into()) },
        );
        let error = result.unwrap_err();
        assert!(error.contains("配置回滚失败"));
        assert!(error.contains("外部设置恢复失败"));
        assert_eq!(current.max_history, 100);
        assert_eq!(
            commit_config_change(&mut current, next, |_| Ok(()), |_| Ok("applied")).unwrap(),
            "applied"
        );
        assert_eq!(current.max_history, 333);
    }
}
