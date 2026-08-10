mod capture;
mod clipboard_watcher;
mod commands;
mod config;
mod gsettings_shortcuts;
mod image_io;
mod models;
mod ocr;
mod paste;
mod pin;
mod pin_window;
mod screenshot;
mod storage;
mod translation;
mod tray_icon;
mod window_controller;

use commands::AppState;
use models::AppConfig;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Listener, Manager};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

/// 当前可执行文件是否位于 cargo target 产物目录（开发期产物，不应被自启）
///
/// 防止开发者在 `cargo tauri dev` 状态下点了"开机自启"toggle，把 dev 路径
/// 写入 ~/.config/autostart/Clippy.desktop —— 那是 v0.1.6 幽灵进程问题的源头之一。
fn is_dev_binary() -> bool {
    match std::env::current_exe() {
        Ok(p) => {
            let s = p.to_string_lossy();
            s.contains("/target/debug/") || s.contains("/target/release/")
        }
        Err(_) => false,
    }
}

/// 已有实例运行时的回调：聚焦主窗口
fn on_second_instance(app: &tauri::AppHandle, _args: Vec<String>, _cwd: String) {
    if app.get_webview_window("main").is_some() {
        if let Some(state) = app.try_state::<AppState>() {
            state.paste_manager.capture_target();
        }
        let _ = window_controller::show_main_window(app);
    }
}

/// 构建系统托盘：左键点击弹出菜单，包含 Open Clipboard / Settings / Quit
fn build_tray(app: &tauri::App, theme: &str) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;

    let open_item = MenuItem::with_id(app, "open_clipboard", "Open Clipboard", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&open_item, &settings_item, &quit_item])?;

    // 优先使用按主题渲染的图标；失败回退到默认窗口图标
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
                // 打开或聚焦设置窗口（先销毁再重建，确保加载最新页面）
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
            "quit" => {
                app_handle.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}

/// 全局快捷键回调：切换主窗口可见性。被 plugin 全局 handler 调用。
fn toggle_main_window(handle: &tauri::AppHandle) {
    log::info!("全局快捷键触发 toggle_main_window");
    if let Some(window) = handle.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            if let Some(state) = handle.try_state::<AppState>() {
                state.paste_manager.capture_target();
            }
            if let Err(error) = window_controller::show_main_window(handle) {
                log::warn!("显示主窗口失败: {error}");
            }
        }
    } else {
        log::warn!("找不到 main 窗口");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShortcutAction {
    ToggleMain,
    PinCurrent,
    Capture,
}

fn shortcut_matches(pressed: &Shortcut, configured: &str) -> bool {
    let configured = configured.trim();
    if configured.is_empty() {
        return false;
    }
    Shortcut::from_str(configured)
        .map(|shortcut| shortcut.id() == pressed.id())
        .unwrap_or(false)
}

fn shortcut_action(config: &AppConfig, pressed: &Shortcut) -> Option<ShortcutAction> {
    if shortcut_matches(pressed, &config.global_shortcut) {
        Some(ShortcutAction::ToggleMain)
    } else if shortcut_matches(pressed, &config.pin_shortcut) {
        Some(ShortcutAction::PinCurrent)
    } else if shortcut_matches(pressed, &config.capture_shortcut) {
        Some(ShortcutAction::Capture)
    } else {
        None
    }
}

fn configured_shortcuts(config: &AppConfig) -> Result<Vec<Shortcut>, String> {
    let mut ids = HashSet::new();
    let mut shortcuts = Vec::new();
    for raw in [
        config.global_shortcut.as_str(),
        config.pin_shortcut.as_str(),
        config.capture_shortcut.as_str(),
    ] {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let shortcut =
            Shortcut::from_str(raw).map_err(|e| format!("快捷键 `{raw}` 解析失败: {e}"))?;
        if ids.insert(shortcut.id()) {
            shortcuts.push(shortcut);
        }
    }
    Ok(shortcuts)
}

pub(crate) fn register_x11_shortcuts(
    handle: &tauri::AppHandle,
    config: &AppConfig,
) -> Result<(), String> {
    let shortcuts = configured_shortcuts(config)?;
    let gs = handle.global_shortcut();
    gs.unregister_all().map_err(|e| e.to_string())?;
    if !shortcuts.is_empty() {
        gs.register_multiple(shortcuts).map_err(|e| e.to_string())?;
    }
    log::info!("X11 快捷键注册完成");
    Ok(())
}

fn trigger_pin_current(handle: &tauri::AppHandle) {
    let _ = handle.emit("pin-current", ());
}

fn trigger_capture(handle: &tauri::AppHandle) {
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = commands::show_capture_editor_for_app(handle).await {
            log::warn!("截图快捷键触发失败: {}", e);
        }
    });
}

fn handle_registered_shortcut(app: &tauri::AppHandle, shortcut: &Shortcut) {
    let Some(state) = app.try_state::<AppState>() else {
        log::warn!("快捷键触发时 AppState 未就绪");
        return;
    };
    let action = {
        let Ok(config) = state.config.lock() else {
            log::warn!("快捷键触发时无法读取配置");
            return;
        };
        shortcut_action(&config, shortcut)
    };

    match action {
        Some(ShortcutAction::ToggleMain) => toggle_main_window(app),
        Some(ShortcutAction::PinCurrent) => trigger_pin_current(app),
        Some(ShortcutAction::Capture) => trigger_capture(app),
        None => log::warn!("收到未配置的快捷键事件: {}", shortcut),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            on_second_instance(app, args, cwd);
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state != ShortcutState::Pressed {
                        return;
                    }
                    handle_registered_shortcut(app, _shortcut);
                })
                .build(),
        )
        .setup(|app| {
            // ── 0. 自启路径合法性守护 ────────────────────────────────────────
            // 若开发期 dev 二进制被错误地写入 autostart（v0.1.6 幽灵进程根因之一），
            // 立即注销 autostart 并退出，避免抢占 D-Bus name 阻塞正常安装版启动。
            if is_dev_binary() {
                let autostart = app.autolaunch();
                if matches!(autostart.is_enabled(), Ok(true)) {
                    log::warn!("检测到 dev 二进制被加入开机自启，自动注销以避免幽灵进程");
                    let _ = autostart.disable();
                }
            }

            // ── 1. 确定数据目录 ──────────────────────────────────────────────
            let app_data_dir = app.path().app_data_dir().expect("无法获取应用数据目录");
            std::fs::create_dir_all(&app_data_dir).expect("无法创建应用数据目录");

            // ── 2. 加载配置 ──────────────────────────────────────────────────
            let config_path = app_data_dir.join("config.json");
            let app_config = config::load_config(&config_path);

            // ── 3. 初始化存储引擎 ────────────────────────────────────────────
            let storage = if app_config.storage_mode == "memory" {
                storage::StorageEngine::new_in_memory()
            } else {
                let db_path = app_data_dir.join("clips.db");
                storage::StorageEngine::new(&db_path)
            }
            .expect("无法初始化存储引擎");

            let storage = Arc::new(Mutex::new(storage));
            let config = Arc::new(Mutex::new(app_config.clone()));
            let paste_manager = Arc::new(paste::PasteManager::new(&app_data_dir));
            let pin_manager = Arc::new(pin::PinManager::new());
            let capture_manager = Arc::new(capture::CaptureManager::new());
            let translation = Arc::new(translation::TranslationService::new());

            // ── 4. 启动剪贴板监听器 ──────────────────────────────────────────
            let watcher = clipboard_watcher::ClipboardWatcher::new();
            watcher.start(
                app.handle().clone(),
                Arc::clone(&storage),
                Arc::clone(&config),
            );

            // ── 4b. 若 tmux 捕获已启用，配置 hook ────────────────────────────
            if app_config.tmux_capture {
                if let Err(e) = commands::setup_tmux_hook() {
                    log::warn!("tmux hook 配置失败（可能 tmux 未运行）: {}", e);
                }
            }

            // ── 5. 注册全局状态 ──────────────────────────────────────────────
            app.manage(AppState {
                storage,
                config,
                config_path,
                watcher,
                preview_visible: Arc::new(Mutex::new(false)),
                codec_visible: Arc::new(Mutex::new(false)),
                latest_capture: Arc::new(Mutex::new(None)),
                capture_manager,
                pin_manager,
                paste_manager,
                translation,
            });

            // ── 6. 构建系统托盘 ──────────────────────────────────────────────
            build_tray(app, &app_config.theme).expect("无法构建系统托盘");

            // ── 6b. 监听 config-changed，主题变更时刷新托盘图标 ──────────────
            let handle = app.handle().clone();
            app.listen("config-changed", move |event| {
                #[derive(serde::Deserialize)]
                struct Payload {
                    theme: String,
                }
                let theme = match serde_json::from_str::<Payload>(event.payload()) {
                    Ok(p) => p.theme,
                    Err(e) => {
                        log::warn!("config-changed payload 解析失败: {}", e);
                        return;
                    }
                };
                let Some(tray) = handle.tray_by_id("main") else {
                    log::warn!("找不到托盘 id=main，跳过主题刷新");
                    return;
                };
                match tray_icon::render_themed_tray_icon(&theme) {
                    Some(icon) => {
                        if let Err(e) = tray.set_icon(Some(icon)) {
                            log::warn!("托盘图标刷新失败: {}", e);
                        }
                    }
                    None => {
                        log::warn!("托盘图标渲染失败 (theme={}), 保持当前图标", theme);
                        // emit 事件通知前端（可选：前端显示 toast）
                        let _ = handle.emit("tray-icon-render-failed", theme);
                    }
                }
            });

            // ── 7. 注册全局快捷键（从配置读取）────────────────────────────────
            if gsettings_shortcuts::is_wayland() {
                log::info!("检测到 Wayland 会话，使用 gsettings 自定义快捷键 + D-Bus");
                // 注册 gsettings 自定义快捷键（Toggle）
                if let Err(e) = gsettings_shortcuts::register(&app_config.global_shortcut) {
                    log::warn!("gsettings 快捷键注册失败: {}", e);
                    use tauri::Emitter;
                    let _ = app.emit("shortcut-register-failed", &app_config.global_shortcut);
                }
                // 注册 Pin 快捷键
                if let Err(e) = gsettings_shortcuts::register_pin(&app_config.pin_shortcut) {
                    log::warn!("gsettings Pin 快捷键注册失败: {}", e);
                }
                // 注册 Capture 快捷键
                if let Err(e) = gsettings_shortcuts::register_capture(&app_config.capture_shortcut)
                {
                    log::warn!("gsettings Capture 快捷键注册失败: {}", e);
                }
                // 启动 D-Bus 服务接收 Toggle 调用 —— name 抢占必须成功，
                // 否则当前进程是"幽灵副本"，立即退出让 single-instance 自动清理。
                let handle = app.handle().clone();
                let (ready_tx, ready_rx) = std::sync::mpsc::channel();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = gsettings_shortcuts::start_dbus_service(handle, ready_tx).await
                    {
                        log::error!("D-Bus 服务运行期失败: {}", e);
                    }
                });
                // 等待最多 3 秒确认 name 抢占结果（一般毫秒级返回）
                match ready_rx.recv_timeout(std::time::Duration::from_secs(3)) {
                    Ok(Ok(())) => log::info!("D-Bus 服务 name 抢占成功"),
                    Ok(Err(e)) => {
                        log::error!(
                            "D-Bus name 抢占失败（已有 Clippy 实例驻留）: {} — 当前进程立即退出避免幽灵化",
                            e
                        );
                        app.handle().exit(1);
                        return Ok(());
                    }
                    Err(_) => {
                        log::error!("D-Bus 服务启动超时，当前进程立即退出避免幽灵化");
                        app.handle().exit(1);
                        return Ok(());
                    }
                }
            } else {
                log::info!("检测到 X11 会话，使用 tauri-plugin-global-shortcut");
                if let Err(e) = register_x11_shortcuts(app.handle(), &app_config) {
                    log::warn!("全局快捷键注册失败（可能已被占用）: {}", e);
                    // 通知前端快捷键注册失败
                    use tauri::Emitter;
                    let _ = app.emit("shortcut-register-failed", &app_config.global_shortcut);
                }
            }

            // ── 8. 可回退的 WebKit 诊断开关 ────────────────────────────────
            // 默认保留 WebKitGTK 平台策略；全局禁用 GPU 会在部分 X11 驱动上导致
            // 黑屏。仅在排障时通过 CLIPPY_DISABLE_GPU=1 显式启用软件渲染。
            #[cfg(target_os = "linux")]
            if std::env::var("CLIPPY_DISABLE_GPU").as_deref() == Ok("1") {
                if let Some(main_window) = app.get_webview_window("main") {
                    let _ = main_window.with_webview(|webview| {
                        use webkit2gtk::{SettingsExt, WebViewExt};
                        let wk = webview.inner();
                        if let Some(settings) = wk.settings() {
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

            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
                    // 主窗口：关闭时只隐藏，不退出（仅托盘 Quit 才真正退出）
                    api.prevent_close();
                    let _ = window.hide();
                }
                tauri::WindowEvent::Focused(false) if window.label() == "main" => {
                    // 仅主窗口：失焦后延迟隐藏（预览面板打开时跳过）
                    let window = window.clone();
                    let app_handle = window.app_handle().clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(200));
                        if !window.is_focused().unwrap_or(true) {
                            // 预览面板打开时不自动隐藏
                            if let Some(state) = app_handle.try_state::<commands::AppState>() {
                                if let Ok(pv) = state.preview_visible.lock() {
                                    if *pv {
                                        return;
                                    }
                                }
                            }
                            let _ = window.hide();
                        }
                    });
                }
                tauri::WindowEvent::Destroyed if window.label().starts_with("pin-") => {
                    if let Some(state) = window.app_handle().try_state::<commands::AppState>() {
                        state.pin_manager.remove_window(window.label());
                    }
                }
                tauri::WindowEvent::Destroyed
                    if window.label().starts_with("capture-overlay-") =>
                {
                    if let Some(state) = window.app_handle().try_state::<commands::AppState>() {
                        capture::handle_overlay_destroyed(
                            window.app_handle(),
                            &state,
                            window.label(),
                        );
                    }
                }
                tauri::WindowEvent::Destroyed if window.label() == "capture" => {
                    if let Some(state) = window.app_handle().try_state::<commands::AppState>() {
                        commands::clear_latest_capture(&state);
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_clips,
            commands::delete_clip,
            commands::toggle_favorite,
            commands::clear_history,
            commands::select_clip,
            commands::copy_clip,
            commands::get_paste_status,
            commands::request_paste_permission,
            commands::get_clip_image,
            commands::get_clip_detail,
            commands::set_preview_visible,
            commands::set_codec_visible,
            commands::get_config,
            commands::update_config,
            commands::update_shortcut,
            commands::check_shortcut_conflict,
            commands::show_settings,
            commands::pause_shortcuts,
            commands::resume_shortcuts,
            commands::get_install_type,
            commands::is_dev_binary,
            commands::show_capture_editor,
            capture::show_capture_overlay,
            capture::get_capture_overlay,
            capture::cancel_capture_overlay,
            capture::run_capture_action,
            commands::get_pending_capture,
            commands::clear_pending_capture,
            commands::copy_screenshot_image,
            commands::save_screenshot_image,
            pin::pin_screenshot_image,
            pin::pin_clip,
            pin::get_pin_payload,
            pin::pin_ready,
            pin::update_pin,
            pin::copy_pin,
            pin::save_pin,
            pin::edit_pin,
            pin::close_pin,
            commands::ocr_available,
            commands::ocr_image,
            commands::ocr_install,
            commands::fetch_url_meta,
            commands::get_stats,
            commands::toggle_tmux_capture,
            commands::tmux_available,
            translation::commands::translate_text,
            translation::commands::translate_clip,
            translation::commands::set_translation_api_key,
            translation::commands::has_translation_api_key,
            translation::commands::delete_translation_api_key,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}
