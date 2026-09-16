#[cfg(target_os = "linux")]
use tauri::Manager;
#[cfg(target_os = "linux")]
use tauri_plugin_autostart::ManagerExt;

/// 仅清理明确指向当前开发二进制的自启动项；共享名称不能证明归属。
pub(crate) fn guard_dev_autostart(_app: &tauri::App) {
    if !crate::platform::is_dev_binary() {
        return;
    }
    #[cfg(target_os = "linux")]
    {
        let (Ok(home), Ok(executable)) = (_app.path().home_dir(), std::env::current_exe()) else {
            log::warn!("无法确定开发自启动项归属，保留现有启动项");
            return;
        };
        // 与 auto-launch 0.5 的实际路径一致：该依赖使用 home/.config，而非 XDG_CONFIG_HOME。
        let path = home
            .join(".config/autostart")
            .join(format!("{}.desktop", _app.package_info().name));
        match cleanup_dev_autostart(&path, &executable, || {
            _app.autolaunch()
                .disable()
                .map_err(|error| error.to_string())
        }) {
            Ok(true) => log::info!("已清理精确指向当前开发二进制的自启动项"),
            Ok(false) => {}
            Err(error) => log::warn!("开发自启动项清理失败，未确认注销: {error}"),
        }
    }
    #[cfg(not(target_os = "linux"))]
    log::debug!("当前平台无法精确确认自启动项归属，保留现有启动项");
}

#[cfg(any(test, target_os = "linux"))]
fn cleanup_dev_autostart(
    path: &std::path::Path,
    executable: &std::path::Path,
    disable: impl FnOnce() -> Result<(), String>,
) -> Result<bool, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    // 不确定的条目一律保留，包括符号链接、超大文件和无法解析的桌面项。
    if !metadata.file_type().is_file() || metadata.len() > 64 * 1024 {
        return Ok(false);
    }
    let content = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    if desktop_exec_path(&content).as_deref() != Some(executable) {
        return Ok(false);
    }
    disable()?;
    Ok(true)
}

#[cfg(any(test, target_os = "linux"))]
fn desktop_exec_path(content: &str) -> Option<std::path::PathBuf> {
    let mut in_entry = false;
    let mut found_entry = false;
    let mut application = None;
    let mut exec = None;
    for line in content.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            if in_entry {
                if found_entry {
                    return None;
                }
                found_entry = true;
            }
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some(value) = line.strip_prefix("Type=") {
            if application.is_some() {
                return None;
            }
            application = Some(value == "Application");
        }
        if let Some(value) = line.strip_prefix("Exec=") {
            if exec.is_some() {
                return None;
            }
            exec = Some(value);
        }
    }
    if application != Some(true) {
        return None;
    }
    let value = exec?.trim();
    let token = if let Some(rest) = value.strip_prefix('"') {
        let end = rest.find('"')?;
        let trailing = &rest[end + 1..];
        if !trailing.is_empty() && !trailing.starts_with(char::is_whitespace) {
            return None;
        }
        &rest[..end]
    } else {
        value.split_whitespace().next()?
    };
    // 保守解析：不能证明反斜杠/字段码/变量转义含义时不清理。
    if token.contains(['\\', '"', '\'', '$', '`', '%', ';', '|', '&', '<', '>']) {
        return None;
    }
    let path = std::path::PathBuf::from(token);
    path.is_absolute().then_some(path)
}

/// 仅在显式诊断开关启用时关闭硬件加速，避免全局策略导致 X11 黑屏。
pub(crate) fn configure_webkit_diagnostics(_app: &tauri::App) {
    #[cfg(target_os = "linux")]
    match webkit_diagnostic_policy(std::env::var("CLIPPY_DISABLE_GPU").ok().as_deref()) {
        WebkitDiagnosticPolicy::Default => {
            log::debug!("WebKit 使用默认硬件加速策略");
        }
        WebkitDiagnosticPolicy::DisableGpu => {
            log::warn!(
                "WebKit GPU 诊断已启用：关闭硬件加速、WebGL、媒体与页面缓存；删除 CLIPPY_DISABLE_GPU 即可回退"
            );
            let Some(main_window) = _app.get_webview_window("main") else {
                log::warn!("WebKit GPU 诊断未应用：找不到 main 窗口");
                return;
            };
            if let Err(error) = main_window.with_webview(|webview| {
                use webkit2gtk::{SettingsExt, WebViewExt};
                let webkit = webview.inner();
                if let Some(settings) = webkit.settings() {
                    settings.set_hardware_acceleration_policy(
                        webkit2gtk::HardwareAccelerationPolicy::Never,
                    );
                    settings.set_enable_webgl(false);
                    settings.set_enable_webaudio(false);
                    settings.set_enable_media_stream(false);
                    settings.set_enable_media(false);
                    settings.set_enable_page_cache(false);
                    settings.set_enable_smooth_scrolling(false);
                    log::info!("WebKit GPU 诊断策略已应用到 main 窗口");
                } else {
                    log::warn!("WebKit GPU 诊断未应用：WebView settings 不可用");
                }
            }) {
                log::warn!("WebKit GPU 诊断应用失败: {error}");
            }
        }
    }
}

#[cfg(any(test, target_os = "linux"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebkitDiagnosticPolicy {
    Default,
    DisableGpu,
}

#[cfg(any(test, target_os = "linux"))]
fn webkit_diagnostic_policy(value: Option<&str>) -> WebkitDiagnosticPolicy {
    match value {
        Some("1") => WebkitDiagnosticPolicy::DisableGpu,
        Some(value) if !value.is_empty() && value != "0" => {
            log::warn!("忽略无效的 CLIPPY_DISABLE_GPU={value:?}，使用默认 WebKit 策略");
            WebkitDiagnosticPolicy::Default
        }
        _ => WebkitDiagnosticPolicy::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::{webkit_diagnostic_policy, WebkitDiagnosticPolicy};

    #[test]
    fn webkit_diagnostic_switch_is_explicit_and_reversible() {
        assert_eq!(
            webkit_diagnostic_policy(Some("1")),
            WebkitDiagnosticPolicy::DisableGpu
        );
        assert_eq!(
            webkit_diagnostic_policy(None),
            WebkitDiagnosticPolicy::Default
        );
        assert_eq!(
            webkit_diagnostic_policy(Some("0")),
            WebkitDiagnosticPolicy::Default
        );
        assert_eq!(
            webkit_diagnostic_policy(Some("yes")),
            WebkitDiagnosticPolicy::Default
        );
    }
}

#[cfg(test)]
mod autostart_ownership_tests {
    use super::*;
    use std::cell::Cell;
    use std::path::Path;

    #[test]
    fn keeps_production_other_dev_and_uncertain_entries() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("clippy-app.desktop");
        let current = Path::new("/workspace/Clippy/target/debug/clippy-app");
        for entry in [
            "Exec=/usr/bin/clippy-app",
            "Exec=/other/target/debug/clippy-app",
            "Exec=/workspace/Clippy/target/debug/clippy-app-other",
            "Exec=/usr/bin/env /workspace/Clippy/target/debug/clippy-app",
            "Exec=/workspace/Clippy/target/debug/clippy-app\nExec=/usr/bin/clippy-app",
            "Exec=\"/workspace/Clippy/target/debug/clippy-app\"invalid",
        ] {
            let content = format!("[Desktop Entry]\nType=Application\n{entry}\n");
            std::fs::write(&path, &content).unwrap();
            assert!(!cleanup_dev_autostart(&path, current, || panic!(
                "不得调用真实或模拟 disable"
            ))
            .unwrap());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
        }
    }

    #[test]
    fn only_exact_current_dev_exec_can_be_removed_and_errors_are_returned() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("clippy-app.desktop");
        let current = Path::new("/workspace/My Clippy/target/debug/clippy-app");
        std::fs::write(&path, "[Desktop Entry]\nType=Application\nExec=\"/workspace/My Clippy/target/debug/clippy-app\" --startup\n").unwrap();
        let calls = Cell::new(0);
        assert!(cleanup_dev_autostart(&path, current, || {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .unwrap());
        assert_eq!(calls.get(), 1);
        assert!(
            cleanup_dev_autostart(&path, current, || Err("permission denied".into()))
                .unwrap_err()
                .contains("permission denied")
        );
        assert!(path.exists());
    }

    #[test]
    fn desktop_actions_and_escaped_paths_do_not_prove_ownership() {
        for content in [
            "[Desktop Action Dev]\nType=Application\nExec=/dev/clippy",
            "[Desktop Entry]\nType=Application\nExec=\"/dev/a\\ b/clippy\"",
            "[Desktop Entry]\nType=Application\nExec=/dev/%f/clippy",
            "[Desktop Entry]\nType=Link\nType=Application\nExec=/dev/clippy",
        ] {
            assert!(desktop_exec_path(content).is_none());
        }
    }

    #[cfg(unix)]
    #[test]
    fn never_follows_a_symlink_to_an_autostart_entry() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.desktop");
        let path = directory.path().join("clippy-app.desktop");
        std::fs::write(
            &target,
            "[Desktop Entry]\nType=Application\nExec=/dev/clippy\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(
            !cleanup_dev_autostart(&path, Path::new("/dev/clippy"), || panic!("不能清理链接"))
                .unwrap()
        );
        assert!(target.exists());
        assert_eq!(
            desktop_exec_path("[Desktop Entry]\nType=Application\nExec=/dev/clippy --startup\n"),
            Some(Path::new("/dev/clippy").to_path_buf())
        );
    }
}
