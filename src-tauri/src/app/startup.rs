use tauri::Manager;
use tauri_plugin_autostart::ManagerExt;

/// 防止开发期二进制被写入自启后抢占正式安装版实例。
pub(crate) fn guard_dev_autostart(app: &tauri::App) {
    if !is_dev_binary() {
        return;
    }
    let autostart = app.autolaunch();
    if matches!(autostart.is_enabled(), Ok(true)) {
        log::warn!("检测到 dev 二进制被加入开机自启，自动注销以避免幽灵进程");
        let _ = autostart.disable();
    }
}

/// 仅在显式诊断开关启用时关闭硬件加速，避免全局策略导致 X11 黑屏。
pub(crate) fn configure_webkit_diagnostics(app: &tauri::App) {
    #[cfg(target_os = "linux")]
    match webkit_diagnostic_policy(std::env::var("CLIPPY_DISABLE_GPU").ok().as_deref()) {
        WebkitDiagnosticPolicy::Default => {
            log::debug!("WebKit 使用默认硬件加速策略");
        }
        WebkitDiagnosticPolicy::DisableGpu => {
            log::warn!(
                "WebKit GPU 诊断已启用：关闭硬件加速、WebGL、媒体与页面缓存；删除 CLIPPY_DISABLE_GPU 即可回退"
            );
            let Some(main_window) = app.get_webview_window("main") else {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebkitDiagnosticPolicy {
    Default,
    DisableGpu,
}

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

fn is_dev_binary() -> bool {
    match std::env::current_exe() {
        Ok(path) => {
            let path = path.to_string_lossy();
            path.contains("/target/debug/") || path.contains("/target/release/")
        }
        Err(_) => false,
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
