//! portal_shortcuts.rs — Wayland 下通过 XDG Desktop Portal 注册全局快捷键
//!
//! 在 Wayland 环境中，X11 的 XGrabKey 无法捕获全局按键。
//! 此模块使用 org.freedesktop.portal.GlobalShortcuts D-Bus 接口
//! 注册快捷键并监听激活信号，作为 tauri-plugin-global-shortcut 的替代。

use tauri::AppHandle;

/// 检测当前是否运行在 Wayland 会话中
pub fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|v| v == "wayland")
        .unwrap_or(false)
}

/// 在后台任务中启动门户快捷键监听。
/// 调用方应使用 tauri::async_runtime::spawn 运行此函数。
pub async fn setup_portal_shortcuts(handle: AppHandle, preferred_trigger: String) {
    if let Err(e) = run_portal_loop(handle, preferred_trigger).await {
        log::error!("门户快捷键初始化失败: {}", e);
    }
}

async fn run_portal_loop(
    handle: AppHandle,
    preferred_trigger: String,
) -> Result<(), Box<dyn std::error::Error>> {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_lite::StreamExt;

    log::info!("初始化 XDG GlobalShortcuts 门户 (Wayland 模式)");

    // 帮助门户识别应用身份
    std::env::set_var("GIO_LAUNCHED_DESKTOP_FILE", "com.clippy.app.desktop");
    std::env::set_var("GIO_LAUNCHED_DESKTOP_FILE_PID", std::process::id().to_string());

    let shortcuts = GlobalShortcuts::new().await?;
    log::info!("门户版本: {}", shortcuts.version());

    let session = shortcuts
        .create_session(Default::default())
        .await?;

    // 将配置中的快捷键字符串作为偏好触发器
    let new_shortcut =
        NewShortcut::new("toggle-clipboard", "Toggle Clipboard Panel")
            .preferred_trigger(Some(preferred_trigger.as_str()));

    let request = shortcuts
        .bind_shortcuts(&session, &[new_shortcut], None::<&ashpd::WindowIdentifier>, Default::default())
        .await?;
    let response = request.response()?;
    log::info!("门户快捷键绑定成功: {:?}", response.shortcuts());

    // 通知前端快捷键已通过门户注册
    use tauri::Emitter;
    let _ = handle.emit("shortcut-portal-ready", ());

    // 持续监听激活信号
    let mut activated = shortcuts.receive_activated().await?;
    while let Some(event) = activated.next().await {
        let id = event.shortcut_id();
        log::info!("门户快捷键激活: {}", id);
        if id == "toggle-clipboard" {
            super::toggle_main_window(&handle);
        }
    }

    log::warn!("门户快捷键监听流已结束");
    Ok(())
}
