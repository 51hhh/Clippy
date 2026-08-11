use crate::commands::AppState;
use crate::{tray_icon, window_controller};
use tauri::{Emitter, Listener, Manager};

/// 构建系统托盘及 Open/Settings/Quit 菜单。
pub(crate) fn build(app: &tauri::App, theme: &str) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;

    let open_item = MenuItem::with_id(app, "open_clipboard", "Open Clipboard", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_item, &settings_item, &quit_item])?;

    let icon = tray_icon::render_themed_tray_icon(theme)
        .unwrap_or_else(|| app.default_window_icon().expect("缺少默认窗口图标").clone());

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip("Clippy")
        .on_menu_event(|app_handle, event| match event.id.as_ref() {
            "open_clipboard" => {
                if let Some(state) = app_handle.try_state::<AppState>() {
                    state.paste_manager.capture_target();
                }
                let _ = window_controller::show_main_window(app_handle);
            }
            "settings" => {
                if let Some(window) = app_handle.get_webview_window("settings") {
                    let _ = window.close();
                }
                let _ = tauri::WebviewWindowBuilder::new(
                    app_handle,
                    "settings",
                    tauri::WebviewUrl::App("settings.html".into()),
                )
                .title("Clippy Settings")
                .inner_size(720.0, 560.0)
                .min_inner_size(480.0, 400.0)
                .center()
                .resizable(true)
                .build();
            }
            "quit" => app_handle.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

pub(crate) fn listen_for_theme_changes(app: &tauri::App) {
    let handle = app.handle().clone();
    app.listen("config-changed", move |event| {
        #[derive(serde::Deserialize)]
        struct Payload {
            theme: String,
        }

        let theme = match serde_json::from_str::<Payload>(event.payload()) {
            Ok(payload) => payload.theme,
            Err(error) => {
                log::warn!("config-changed payload 解析失败: {}", error);
                return;
            }
        };
        let Some(tray) = handle.tray_by_id("main") else {
            log::warn!("找不到托盘 id=main，跳过主题刷新");
            return;
        };
        match tray_icon::render_themed_tray_icon(&theme) {
            Some(icon) => {
                if let Err(error) = tray.set_icon(Some(icon)) {
                    log::warn!("托盘图标刷新失败: {}", error);
                }
            }
            None => {
                log::warn!("托盘图标渲染失败 (theme={}), 保持当前图标", theme);
                let _ = handle.emit("tray-icon-render-failed", theme);
            }
        }
    });
}
