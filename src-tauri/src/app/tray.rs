use crate::commands::AppState;
use crate::i18n::{self, NativeText};
use crate::models::AppConfig;
use crate::{tray_icon, window_controller};
use tauri::menu::MenuItem;
use tauri::{Emitter, Listener, Manager};

/// 托盘菜单项 id。文案随语言变化，id 保持不变，事件分支才能稳定匹配。
const OPEN_ID: &str = "open_clipboard";
const SETTINGS_ID: &str = "settings";
const QUIT_ID: &str = "quit";

/// 托盘菜单项句柄。语言切换时直接改文案，不重建菜单，
/// 避免刷新过程中托盘短暂没有菜单。`MenuItem` 的写操作自己会切回主线程。
pub(crate) struct TrayMenuItems {
    open_clipboard: MenuItem<tauri::Wry>,
    settings: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
}

impl TrayMenuItems {
    fn apply(&self, text: NativeText) {
        for (item, label) in [
            (&self.open_clipboard, text.open_clipboard),
            (&self.settings, text.settings_menu),
            (&self.quit, text.quit_menu),
        ] {
            if let Err(error) = item.set_text(label) {
                log::warn!("托盘菜单文案刷新失败 ({label}): {error}");
            }
        }
    }
}

/// 构建系统托盘及 Open/Settings/Quit 菜单，文案按配置语言。
pub(crate) fn build(
    app: &tauri::App,
    config: &AppConfig,
) -> Result<TrayMenuItems, Box<dyn std::error::Error>> {
    use tauri::menu::Menu;
    use tauri::tray::TrayIconBuilder;

    let text = i18n::text_for_language(&config.language);
    let items = TrayMenuItems {
        open_clipboard: MenuItem::with_id(app, OPEN_ID, text.open_clipboard, true, None::<&str>)?,
        settings: MenuItem::with_id(app, SETTINGS_ID, text.settings_menu, true, None::<&str>)?,
        quit: MenuItem::with_id(app, QUIT_ID, text.quit_menu, true, None::<&str>)?,
    };
    let menu = Menu::with_items(app, &[&items.open_clipboard, &items.settings, &items.quit])?;

    let icon = tray_icon::render_themed_tray_icon(&config.theme)
        .unwrap_or_else(|| app.default_window_icon().expect("缺少默认窗口图标").clone());

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip("Clippy")
        .on_menu_event(|app_handle, event| match event.id.as_ref() {
            OPEN_ID => {
                if let Some(state) = app_handle.try_state::<AppState>() {
                    state.paste_manager.capture_target();
                }
                let _ = window_controller::show_main_window(app_handle);
            }
            SETTINGS_ID => {
                if let Err(error) = window_controller::open_settings_window(app_handle) {
                    log::warn!("打开设置窗口失败: {error}");
                }
            }
            QUIT_ID => app_handle.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(items)
}

/// 托盘随配置变化刷新：菜单文案跟语言，图标跟主题。
pub(crate) fn listen_for_config_changes(app: &tauri::App, items: TrayMenuItems) {
    let handle = app.handle().clone();
    app.listen("config-changed", move |event| {
        #[derive(serde::Deserialize)]
        struct Payload {
            theme: String,
            /// 裁剪过的 payload 可能没有这个字段，缺失时按 auto 处理。
            #[serde(default)]
            language: String,
        }

        let (theme, language) = match serde_json::from_str::<Payload>(event.payload()) {
            Ok(payload) => (payload.theme, payload.language),
            Err(error) => {
                log::warn!("config-changed payload 解析失败: {}", error);
                return;
            }
        };
        // 文案刷新不依赖托盘句柄，放在图标之前，托盘丢失时语言仍然生效。
        items.apply(i18n::text_for_language(&language));
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
