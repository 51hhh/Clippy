use crate::clipboard_watcher::ClipboardWatcher;
use crate::config::save_config;
use crate::models::{AppConfig, ClipItem, ContentType};
use crate::storage::StorageEngine;
use arboard::Clipboard;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{Manager, State};

const WINDOW_WIDTH_DEFAULT: f64 = 380.0;
const WINDOW_WIDTH_PREVIEW: f64 = 780.0;
const WINDOW_HEIGHT: f64 = 500.0;

/// 全局应用状态，通过 Tauri 的 manage() 注入并在各命令中共享
pub struct AppState {
    pub storage: Arc<Mutex<StorageEngine>>,
    pub config: Arc<Mutex<AppConfig>>,
    pub config_path: PathBuf,
    pub watcher: ClipboardWatcher,
    pub preview_visible: Arc<Mutex<bool>>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tauri IPC 命令
// ─────────────────────────────────────────────────────────────────────────────

/// 查询剪贴板历史列表，支持全文搜索和收藏过滤
#[tauri::command]
pub fn get_clips(
    query: Option<String>,
    favorites_only: bool,
    offset: i64,
    limit: i64,
    state: State<AppState>,
) -> Result<Vec<ClipItem>, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    storage
        .get_clips(query.as_deref(), favorites_only, offset, limit)
        .map_err(|e| e.to_string())
}

/// 删除指定 id 的剪贴板条目
#[tauri::command]
pub fn delete_clip(id: i64, state: State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    storage.delete_clip(id).map_err(|e| e.to_string())
}

/// 切换指定条目的收藏状态，返回新的收藏状态
#[tauri::command]
pub fn toggle_favorite(id: i64, state: State<AppState>) -> Result<bool, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    storage.toggle_favorite(id).map_err(|e| e.to_string())
}

/// 清空所有历史（保留收藏条目）
#[tauri::command]
pub fn clear_history(state: State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    storage.clear_history().map_err(|e| e.to_string())
}

/// 将指定条目的内容写入系统剪贴板，隐藏面板，并模拟粘贴
#[tauri::command]
pub fn select_clip(
    id: i64,
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    // 从存储中读取条目
    let clip = {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        storage.get_clip_by_id(id).map_err(|e| e.to_string())?
    };

    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;

    match clip.content_type {
        ContentType::Text => {
            if let Some(content) = clip.text_content {
                use sha2::{Digest, Sha256};
                let hash = format!(
                    "{:x}",
                    Sha256::new_with_prefix(content.as_bytes()).finalize()
                );
                state.watcher.set_skip_hash(hash);
                clipboard.set_text(content).map_err(|e| e.to_string())?;
            }
        }
        ContentType::Image => {
            // 需要从数据库按 id 加载完整图片数据
            let image_bytes = {
                let storage = state.storage.lock().map_err(|e| e.to_string())?;
                storage
                    .get_clip_image(id)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "图片数据为空".to_string())?
            };

            // 解码 PNG 为 RGBA 用于 arboard
            let img = image::load_from_memory_with_format(&image_bytes, image::ImageFormat::Png)
                .map_err(|e| format!("PNG 解码失败: {}", e))?;
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();

            use sha2::{Digest, Sha256};
            let hash = format!("{:x}", Sha256::new_with_prefix(&image_bytes).finalize());
            state.watcher.set_skip_hash(hash);

            let img_data = arboard::ImageData {
                width: w as usize,
                height: h as usize,
                bytes: std::borrow::Cow::Owned(rgba.into_raw()),
            };
            clipboard.set_image(img_data).map_err(|e| e.to_string())?;
        }
        ContentType::Html => {
            if let Some(html) = &clip.html_content {
                use sha2::{Digest, Sha256};
                let hash = format!("{:x}", Sha256::new_with_prefix(html.as_bytes()).finalize());
                state.watcher.set_skip_hash(hash);
                let alt_text = clip.text_content.as_deref().or(Some(""));
                clipboard
                    .set()
                    .html(html.as_str(), alt_text)
                    .map_err(|e| e.to_string())?;
            } else if let Some(content) = clip.text_content {
                use sha2::{Digest, Sha256};
                let hash = format!(
                    "{:x}",
                    Sha256::new_with_prefix(content.as_bytes()).finalize()
                );
                state.watcher.set_skip_hash(hash);
                clipboard.set_text(content).map_err(|e| e.to_string())?;
            }
        }
    }

    // 隐藏悬浮面板窗口
    if let Some(window) = app_handle.get_webview_window("main") {
        window.hide().map_err(|e| e.to_string())?;
    }

    // 短暂延迟后模拟 Ctrl+V 粘贴到之前的活动窗口
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(100));
        simulate_paste();
    });

    Ok(())
}

/// 通过 XTest 扩展模拟 Ctrl+V 粘贴（enigo/x11rb 后端）
fn simulate_paste() {
    use enigo::{
        Direction::{Click, Press, Release},
        Enigo, Key, Keyboard, Settings,
    };

    match Enigo::new(&Settings::default()) {
        Ok(mut enigo) => {
            if let Err(e) = enigo
                .key(Key::Control, Press)
                .and_then(|_| enigo.key(Key::Unicode('v'), Click))
                .and_then(|_| enigo.key(Key::Control, Release))
            {
                log::warn!("模拟粘贴失败: {}", e);
            }
        }
        Err(e) => log::warn!("初始化 enigo 失败: {}", e),
    }
}

/// 按 id 获取图片数据，返回 base64 编码的 PNG
#[tauri::command]
pub fn get_clip_image(id: i64, state: State<AppState>) -> Result<Option<String>, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let data = storage.get_clip_image(id).map_err(|e| e.to_string())?;
    Ok(data.map(|bytes| STANDARD.encode(&bytes)))
}

/// 按 id 获取完整条目（含 html_content），用于预览面板按需加载
#[tauri::command]
pub fn get_clip_detail(id: i64, state: State<AppState>) -> Result<ClipItem, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut clip = storage.get_clip_by_id(id).map_err(|e| e.to_string())?;
    clip.image_data = None; // 不通过此接口传输图片二进制
    Ok(clip)
}

/// 切换预览面板：调整主窗口宽度
#[tauri::command]
pub fn set_preview_visible(
    visible: bool,
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    if let Ok(mut pv) = state.preview_visible.lock() {
        *pv = visible;
    }
    if let Some(window) = app_handle.get_webview_window("main") {
        let width = if visible {
            WINDOW_WIDTH_PREVIEW
        } else {
            WINDOW_WIDTH_DEFAULT
        };
        let height = WINDOW_HEIGHT;
        window
            .set_size(tauri::Size::Logical(tauri::LogicalSize { width, height }))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 读取当前应用配置
#[tauri::command]
pub fn get_config(state: State<AppState>) -> Result<AppConfig, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    Ok(config.clone())
}

/// 更新应用配置并持久化到磁盘
#[tauri::command]
pub fn update_config(
    new_config: AppConfig,
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    // 检测 Pin 快捷键变更
    let pin_changed = config.pin_shortcut != new_config.pin_shortcut;
    *config = new_config;
    save_config(&state.config_path, &config);
    // 广播配置变更事件，通知所有窗口（尤其是主窗口更新主题）
    use tauri::Emitter;
    let _ = app_handle.emit("config-changed", &*config);
    // Wayland 下动态更新 Pin 快捷键绑定
    if pin_changed && crate::gsettings_shortcuts::is_wayland() {
        if let Err(e) = crate::gsettings_shortcuts::update_pin_binding(&config.pin_shortcut) {
            log::warn!("更新 Pin 快捷键失败: {}", e);
        }
    }
    Ok(())
}

/// 动态更新全局快捷键：注销旧快捷键，注册新快捷键，并持久化到配置
/// 回调由 plugin 全局 handler 处理（lib.rs::toggle_main_window）。
#[tauri::command]
pub fn update_shortcut(
    new_shortcut: String,
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    log::info!("更新全局快捷键: {}", new_shortcut);

    if crate::gsettings_shortcuts::is_wayland() {
        crate::gsettings_shortcuts::update_binding(&new_shortcut)?;
    } else {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        let gs = app_handle.global_shortcut();
        gs.unregister_all().map_err(|e| e.to_string())?;
        gs.register(new_shortcut.as_str()).map_err(|e| {
            log::error!("快捷键注册失败: {} -> {}", new_shortcut, e);
            format!("快捷键注册失败: {}", e)
        })?;
        log::info!("快捷键注册成功: {}", new_shortcut);
    }

    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    config.global_shortcut = new_shortcut;
    save_config(&state.config_path, &config);

    Ok(())
}

/// 检查指定快捷键是否已被注册（用于冲突检测）
#[tauri::command]
pub fn check_shortcut_conflict(
    shortcut: String,
    app_handle: tauri::AppHandle,
) -> Result<bool, String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    Ok(app_handle
        .global_shortcut()
        .is_registered(shortcut.as_str()))
}

/// 打开或聚焦设置窗口（先销毁再重建，确保加载最新页面）
#[tauri::command]
pub fn show_settings(app_handle: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("settings") {
        let _ = window.close();
    }
    tauri::WebviewWindowBuilder::new(
        &app_handle,
        "settings",
        tauri::WebviewUrl::App("settings.html".into()),
    )
    .title("Clippy Settings")
    .inner_size(720.0, 560.0)
    .min_inner_size(480.0, 400.0)
    .center()
    .resizable(true)
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 暂停全局快捷键（录制新快捷键时调用，避免冲突）
#[tauri::command]
pub fn pause_shortcuts(app_handle: tauri::AppHandle) -> Result<(), String> {
    if crate::gsettings_shortcuts::is_wayland() {
        crate::gsettings_shortcuts::pause().map_err(|e| e.to_string())
    } else {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        app_handle
            .global_shortcut()
            .unregister_all()
            .map_err(|e| e.to_string())
    }
}

/// 恢复全局快捷键（录制结束后调用）
#[tauri::command]
pub fn resume_shortcuts(
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    let shortcut_str = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        config.global_shortcut.clone()
    };
    if crate::gsettings_shortcuts::is_wayland() {
        crate::gsettings_shortcuts::resume(&shortcut_str)
    } else {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        app_handle
            .global_shortcut()
            .register(shortcut_str.as_str())
            .map_err(|e| e.to_string())
    }
}

/// 检测安装类型：appimage 支持自动更新，deb/dev 不支持
#[tauri::command]
pub fn get_install_type() -> String {
    if std::env::var("APPIMAGE").is_ok() {
        "appimage".into()
    } else {
        "deb".into()
    }
}

/// 当前可执行文件是否位于 cargo target 产物目录（开发期产物）
///
/// 前端用于禁用"开机自启"toggle —— autostart 会以 current_exe 路径写入
/// .desktop 文件，dev 路径写入会在系统重启后产生指向已删除/已更新二进制的
/// 幽灵自启项（v0.1.6 已知问题）。
#[tauri::command]
pub fn is_dev_binary() -> bool {
    match std::env::current_exe() {
        Ok(p) => {
            let s = p.to_string_lossy();
            s.contains("/target/debug/") || s.contains("/target/release/")
        }
        Err(_) => false,
    }
}

// ── 贴图 Pin-to-Desktop ─────────────────────────────────────────────────────

/// 将指定条目钉到桌面：创建 always-on-top 无边框透明窗口
#[tauri::command]
pub fn pin_clip(id: i64, app_handle: tauri::AppHandle) -> Result<String, String> {
    let label = format!("pin-{}", id);

    // 如果已存在则聚焦
    if let Some(win) = app_handle.get_webview_window(&label) {
        let _ = win.set_focus();
        return Ok(label);
    }

    tauri::WebviewWindowBuilder::new(
        &app_handle,
        &label,
        tauri::WebviewUrl::App(format!("pin.html?id={}", id).into()),
    )
    .title("")
    .inner_size(400.0, 300.0)
    .decorations(false)
    .always_on_top(true)
    .transparent(true)
    .skip_taskbar(true)
    .resizable(true)
    .center()
    .build()
    .map_err(|e| e.to_string())?;

    Ok(label)
}

/// 关闭指定贴图窗口
#[tauri::command]
pub fn close_pin(label: String, app_handle: tauri::AppHandle) -> Result<(), String> {
    if !label.starts_with("pin-") {
        return Err("只能关闭贴图窗口".to_string());
    }
    if let Some(win) = app_handle.get_webview_window(&label) {
        win.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ── OCR 文字识别 ─────────────────────────────────────────────────────

/// 检查系统是否安装了 tesseract（前端据此决定是否显示 OCR 功能）
#[tauri::command]
pub fn ocr_available() -> bool {
    crate::ocr::is_available()
}

/// 对指定图片条目进行 OCR 识别，返回文字内容（带缓存，异步不阻塞）
#[tauri::command]
pub async fn ocr_image(id: i64, state: State<'_, AppState>) -> Result<String, String> {
    // 先查缓存
    {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        if let Some(cached) = storage.get_ocr_text(id).map_err(|e| e.to_string())? {
            return Ok(cached);
        }
    }

    let image_bytes = {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        storage
            .get_clip_image(id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "图片数据为空".to_string())?
    };
    // 在独立线程中执行 OCR，避免阻塞 Tauri 主线程
    let text = tauri::async_runtime::spawn_blocking(move || crate::ocr::recognize(&image_bytes))
        .await
        .map_err(|e| format!("OCR 线程异常: {}", e))??;

    // 缓存结果
    {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        let _ = storage.set_ocr_text(id, &text);
    }

    Ok(text)
}
