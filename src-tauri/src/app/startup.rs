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
    if std::env::var("CLIPPY_DISABLE_GPU").as_deref() == Ok("1") {
        if let Some(main_window) = app.get_webview_window("main") {
            let _ = main_window.with_webview(|webview| {
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
                }
            });
        }
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
