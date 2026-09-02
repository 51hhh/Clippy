mod app;
pub mod bench_support;
mod capture;
mod clipboard_watcher;
mod commands;
mod config;
mod dbus;
mod dialogs;
mod error;
mod gsettings_shortcuts;
mod i18n;
mod image_io;
mod models;
mod ocr;
mod paste;
mod pin;
mod pin_window;
mod platform;
mod private_files;
mod screenshot;
mod shortcut_conflict;
mod storage;
mod translation;
mod tray_icon;
mod webview_hardening;
mod window_controller;

use commands::AppState;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};
use tauri::Manager;
use tauri_plugin_global_shortcut::ShortcutState;

pub(crate) use app::shortcuts::{
    record_register_result, register_x11_shortcuts, toggle_main_window,
};

/// 命令行截图诊断：`clippy --capture-diagnose` / `--emit-test-case` /
/// `CLIPPY_CAPTURE_DIAGNOSE=1`。返回 `Some(退出码)` 表示这次启动只做诊断。
///
/// **必须在 [`run`] 之前调用**：诊断全程是阻塞 D-Bus，且不该拉起窗口，
/// 更不该撞上 single-instance 的 name 抢占把用户正在用的实例顶掉。
pub fn capture_diagnostics_cli() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    let env_flag = std::env::var(capture::diagnostics::DIAGNOSE_ENV).ok();
    let mode = capture::diagnostics::cli_mode(&args, env_flag.as_deref())?;
    let note = capture::diagnostics::cli_note(&args);
    // 采集过程里的 `log::warn!`（枚举失败、扩展不应答）本身就是诊断信息，得让它出现在
    // 终端里。这条路一定以 `exit` 结束，所以不会和 `run()` 里那次 init 撞上。
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("clippy_lib=info"))
        .init();
    Some(capture::diagnostics::run_cli(
        mode,
        env!("CARGO_PKG_VERSION"),
        note,
    ))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // env_logger 默认只放行 error，于是所有 `log::warn!`/`log::info!` 都是写给空气的——
    // "覆盖层超时未报告首帧""扩展未应答"这些排障线索一条都看不到。自己的 crate 默认放到
    // info，其余依赖留在 warn；`RUST_LOG` 仍然优先，需要更细的时候照常覆盖。
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("clippy_lib=info,warn"),
    )
    .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            app::shortcuts::on_second_instance(app, args, cwd);
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        // 关掉 WebKit 自带的右键菜单与开发者工具。注册成插件是为了覆盖**每一个** webview，
        // 包括按需创建的设置窗口与贴图窗口（见 `webview_hardening`）。
        .plugin(webview_hardening::plugin())
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
            let default_screenshot_dir = app
                .path()
                .picture_dir()
                .unwrap_or_else(|error| {
                    log::warn!("无法获取系统图片目录，退回兼容路径: {error}");
                    image_io::default_screenshot_dir()
                })
                .join("Clippy");

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
                default_screenshot_dir,
                watcher,
                preview_visible: Arc::new(Mutex::new(false)),
                codec_visible: Arc::new(Mutex::new(false)),
                main_window_transition: Mutex::new(()),
                pin_transition: Mutex::new(()),
                main_window_position_generation: AtomicU64::new(0),
                capture_manager,
                pin_manager,
                pin_origins: Arc::new(pin::PinOriginRegistry::default()),
                paste_manager,
                translation,
                shortcuts_paused: AtomicBool::new(false),
                shortcut_transition: Mutex::new(()),
                shortcut_failures: Mutex::new(Vec::new()),
            });

            // ── 6. 构建托盘并监听主题/语言变化 ────────────────────────────
            let tray_items = app::tray::build(app, &app_config).expect("无法构建系统托盘");
            app::tray::listen_for_config_changes(app, tray_items);

            // ── 7. 注册全局快捷键（从配置读取）────────────────────────────────
            if platform::uses_gnome_shortcuts() {
                log::info!("检测到 Wayland 会话，使用 gsettings 自定义快捷键 + D-Bus");
                // 三个动作的失败都要上报：非 GNOME 的 Wayland 桌面没有 media-keys schema，
                // 只写日志会让快捷键静默失效。
                record_register_result(
                    app.handle(),
                    &["global"],
                    &app_config.global_shortcut,
                    true,
                    gsettings_shortcuts::register(&app_config.global_shortcut),
                );
                record_register_result(
                    app.handle(),
                    &["pin"],
                    &app_config.pin_shortcut,
                    true,
                    gsettings_shortcuts::register_pin(&app_config.pin_shortcut),
                );
                record_register_result(
                    app.handle(),
                    &["capture"],
                    &app_config.capture_shortcut,
                    true,
                    gsettings_shortcuts::register_capture(&app_config.capture_shortcut),
                );
                // 用户装过窗口速选扩展的话，顺手做一次内容对齐与孤儿清理；
                // 没装过就什么都不做——绝不擅自往用户的 GNOME 里塞扩展。
                capture::reconcile_window_probe_extension();
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
                // 逐个动作注册并在内部按动作记账，这里只需记录"全都没注册上"的整体失败。
                if let Err(error) = register_x11_shortcuts(app.handle(), &app_config) {
                    log::warn!("X11 快捷键全部注册失败: {error}");
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
            commands::get_clip_thumbnail,
            commands::get_clip_detail,
            commands::set_preview_visible,
            commands::set_codec_visible,
            commands::get_config,
            commands::update_config,
            commands::check_shortcut_conflict,
            commands::get_shortcut_failures,
            commands::show_settings,
            commands::pause_shortcuts,
            commands::resume_shortcuts,
            commands::get_install_type,
            commands::get_platform_info,
            commands::is_dev_binary,
            capture::show_capture_overlay,
            capture::get_capture_overlay,
            capture::get_capture_frame,
            capture::mark_capture_overlay_ready,
            capture::cancel_capture_overlay,
            capture::commit_capture_action,
            capture::translate_capture_selection,
            capture::get_window_probe_status,
            capture::install_window_probe_extension,
            capture::uninstall_window_probe_extension,
            capture::diagnostics::run_capture_diagnostics,
            commands::pick_screenshot_directory,
            pin::commands::pin_clip,
            pin::commands::get_pin_payload,
            pin::commands::get_pin_toolbar_bounds,
            pin::commands::get_pin_source_image,
            pin::commands::pin_ready,
            pin::commands::update_pin,
            pin::commands::copy_pin,
            pin::commands::copy_pin_canvas,
            pin::commands::save_pin,
            pin::commands::save_pin_canvas,
            pin::commands::read_pin_project,
            pin::commands::open_pin_image_dialog,
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
