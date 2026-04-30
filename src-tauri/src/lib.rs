mod clipboard_watcher;
mod commands;
mod config;
mod gsettings_shortcuts;
mod models;
mod ocr;
mod storage;
mod tray_icon;

use commands::AppState;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Listener, Manager};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::GlobalShortcutExt;

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
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
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
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
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
            let _ = window.show();
            let _ = window.set_focus();
        }
    } else {
        log::warn!("找不到 main 窗口");
    }
}

/// 注册全局快捷键：仅 register accelerator，回调由全局 handler 统一处理
fn register_shortcut(app: &tauri::App, shortcut: &str) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("注册全局快捷键: {}", shortcut);
    app.global_shortcut().register(shortcut)?;
    log::info!("全局快捷键注册成功: {}", shortcut);
    Ok(())
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
                    use tauri_plugin_global_shortcut::ShortcutState;
                    if event.state != ShortcutState::Pressed {
                        return;
                    }
                    toggle_main_window(app);
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

            // ── 4. 启动剪贴板监听器 ──────────────────────────────────────────
            let watcher = clipboard_watcher::ClipboardWatcher::new();
            watcher.start(
                app.handle().clone(),
                Arc::clone(&storage),
                Arc::clone(&config),
            );

            // ── 5. 注册全局状态 ──────────────────────────────────────────────
            app.manage(AppState {
                storage,
                config,
                config_path,
                watcher,
                preview_visible: Arc::new(Mutex::new(false)),
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
                if let Err(e) = register_shortcut(app, &app_config.global_shortcut) {
                    log::warn!("全局快捷键注册失败（可能已被占用）: {}", e);
                    // 通知前端快捷键注册失败
                    use tauri::Emitter;
                    let _ = app.emit("shortcut-register-failed", &app_config.global_shortcut);
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
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_clips,
            commands::delete_clip,
            commands::toggle_favorite,
            commands::clear_history,
            commands::select_clip,
            commands::get_clip_image,
            commands::get_clip_detail,
            commands::set_preview_visible,
            commands::get_config,
            commands::update_config,
            commands::update_shortcut,
            commands::check_shortcut_conflict,
            commands::show_settings,
            commands::pause_shortcuts,
            commands::resume_shortcuts,
            commands::get_install_type,
            commands::is_dev_binary,
            commands::pin_clip,
            commands::close_pin,
            commands::ocr_image,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}
