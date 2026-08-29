mod app;
mod capture;
mod clipboard_watcher;
mod commands;
mod config;
mod dialogs;
mod error;
mod gsettings_shortcuts;
mod image_io;
mod models;
mod ocr;
mod paste;
mod pin;
mod pin_window;
mod private_files;
mod screenshot;
mod storage;
mod translation;
mod tray_icon;
mod window_controller;

use commands::AppState;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};
use tauri::Manager;
use tauri_plugin_global_shortcut::ShortcutState;

pub(crate) use app::shortcuts::{register_x11_shortcuts, toggle_main_window};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            app::shortcuts::on_second_instance(app, args, cwd);
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state != ShortcutState::Pressed {
                        return;
                    }
                    app::shortcuts::handle_registered(app, _shortcut);
                })
                .build(),
        )
        .setup(|app| {
            app::startup::guard_dev_autostart(app);

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
                capture_generation: AtomicU64::new(0),
                capture_window_generation: AtomicU64::new(0),
                capture_editor_transition: Mutex::new(()),
                main_window_transition: Mutex::new(()),
                pin_transition: Mutex::new(()),
                main_window_position_generation: AtomicU64::new(0),
                capture_manager,
                pin_manager,
                paste_manager,
                translation,
                shortcuts_paused: AtomicBool::new(false),
                shortcut_transition: Mutex::new(()),
            });

            // ── 6. 构建托盘并监听主题变化 ────────────────────────────────
            app::tray::build(app, &app_config.theme).expect("无法构建系统托盘");
            app::tray::listen_for_theme_changes(app);

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

            // ── 8. 可回退的 WebKit 诊断开关 ────────────────────────────
            app::startup::configure_webkit_diagnostics(app);

            Ok(())
        })
        .on_window_event(app::window_events::handle)
        .invoke_handler(tauri::generate_handler![
            commands::get_clips,
            commands::delete_clip,
            commands::toggle_favorite,
            commands::clear_history,
            commands::select_clip,
            commands::copy_clip,
            commands::copy_text,
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
            capture::translate_capture_selection,
            commands::get_pending_capture,
            commands::clear_pending_capture,
            commands::copy_screenshot_image,
            commands::save_screenshot_image,
            commands::save_screenshot_image_as,
            commands::pick_screenshot_directory,
            pin::commands::pin_screenshot_image,
            pin::commands::pin_clip,
            pin::commands::get_pin_payload,
            pin::commands::pin_ready,
            pin::commands::update_pin,
            pin::commands::copy_pin,
            pin::commands::save_pin,
            pin::commands::edit_pin,
            pin::commands::close_pin,
            commands::ocr_available,
            commands::ocr_image,
            commands::ocr_install,
            commands::fetch_url_meta,
            commands::get_stats,
            commands::toggle_tmux_capture,
            commands::tmux_available,
            translation::commands::translate_text,
            translation::commands::translate_clip,
            translation::commands::translation_history,
            translation::commands::clear_translation_history,
            translation::commands::speak_text,
            translation::commands::speak_clip,
            translation::commands::set_translation_api_key,
            translation::commands::has_translation_api_key,
            translation::commands::delete_translation_api_key,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}
