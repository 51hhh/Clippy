use crate::clipboard_watcher::ClipboardWatcher;
use crate::config::save_config;
use crate::models::{AppConfig, ClipItem, ContentType, UrlMeta};
use crate::paste::{PasteManager, PasteOutcome, PasteStatus};
use crate::storage::StorageEngine;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, State};

const WINDOW_WIDTH_DEFAULT: f64 = 380.0;
const WINDOW_WIDTH_PANEL: f64 = 400.0;
const WINDOW_HEIGHT: f64 = 500.0;

/// 全局应用状态，通过 Tauri 的 manage() 注入并在各命令中共享
pub struct AppState {
    pub storage: Arc<Mutex<StorageEngine>>,
    pub config: Arc<Mutex<AppConfig>>,
    pub config_path: PathBuf,
    pub watcher: ClipboardWatcher,
    pub preview_visible: Arc<Mutex<bool>>,
    pub codec_visible: Arc<Mutex<bool>>,
    pub latest_capture: Arc<Mutex<Option<crate::screenshot::CapturedScreenshot>>>,
    pub capture_manager: Arc<crate::capture::CaptureManager>,
    pub pin_manager: Arc<crate::pin::PinManager>,
    pub paste_manager: Arc<PasteManager>,
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
    state: State<'_, AppState>,
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

/// 将指定条目的内容写入系统剪贴板，隐藏面板，并按当前平台尝试自动粘贴。
#[tauri::command]
pub async fn select_clip(
    id: i64,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<PasteOutcome, String> {
    write_clip_to_clipboard(id, &state)?;

    // 置顶：更新 created_at 并通知前端移动到列表首位
    let updated_clip = {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        storage.touch_clip(id).map_err(|e| e.to_string())?
    };
    let _ = app_handle.emit("clip-added", &updated_clip);

    // 隐藏悬浮面板窗口，让 X11 恢复记录窗口、Wayland 恢复上一焦点。
    if let Some(window) = app_handle.get_webview_window("main") {
        window.hide().map_err(|e| e.to_string())?;
    }

    let auto_paste = state
        .config
        .lock()
        .map(|config| config.auto_paste)
        .unwrap_or(true);
    if !auto_paste {
        return Ok(PasteOutcome::copied_only(
            state.paste_manager.backend(),
            Some("Automatic paste is disabled".to_string()),
        ));
    }

    match state.paste_manager.paste().await {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            log::warn!("自动粘贴失败，内容已保留在剪贴板: {error}");
            let outcome =
                PasteOutcome::copied_only(state.paste_manager.backend(), Some(error.clone()));
            let _ = app_handle.emit("paste-fallback", &outcome);
            Ok(outcome)
        }
    }
}

/// 纯复制命令，供 Pin 和其他不应注入按键的入口使用。
#[tauri::command]
pub fn copy_clip(id: i64, state: State<AppState>) -> Result<(), String> {
    write_clip_to_clipboard(id, &state)
}

pub(crate) fn write_clip_to_clipboard(id: i64, state: &AppState) -> Result<(), String> {
    let clip = {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        storage.get_clip_by_id(id).map_err(|e| e.to_string())?
    };
    match clip.content_type {
        ContentType::Text => {
            if let Some(content) = clip.text_content {
                use sha2::{Digest, Sha256};
                let hash = format!(
                    "{:x}",
                    Sha256::new_with_prefix(content.as_bytes()).finalize()
                );
                state.watcher.set_skip_hash(hash);
                crate::clipboard_watcher::clipboard_set_text_with_retry(&content)?;
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
            crate::clipboard_watcher::clipboard_set_image_with_retry(img_data)?;
        }
        ContentType::Html => {
            if let Some(html) = &clip.html_content {
                use sha2::{Digest, Sha256};
                let hash = format!("{:x}", Sha256::new_with_prefix(html.as_bytes()).finalize());
                state.watcher.set_skip_hash(hash);
                let alt_text = clip.text_content.as_deref().or(Some(""));
                crate::clipboard_watcher::clipboard_set_html_with_retry(html.as_str(), alt_text)?;
            } else if let Some(content) = clip.text_content {
                use sha2::{Digest, Sha256};
                let hash = format!(
                    "{:x}",
                    Sha256::new_with_prefix(content.as_bytes()).finalize()
                );
                state.watcher.set_skip_hash(hash);
                crate::clipboard_watcher::clipboard_set_text_with_retry(&content)?;
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn get_paste_status(state: State<'_, AppState>) -> Result<PasteStatus, String> {
    let auto_paste = state.config.lock().map_err(|e| e.to_string())?.auto_paste;
    Ok(state.paste_manager.status(auto_paste).await)
}

#[tauri::command]
pub async fn request_paste_permission(state: State<'_, AppState>) -> Result<PasteStatus, String> {
    state.paste_manager.request_permission().await
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

/// 计算当前窗口宽度（根据面板可见状态）
fn calc_window_width(state: &State<AppState>) -> f64 {
    let pv = state.preview_visible.lock().map(|v| *v).unwrap_or(false);
    let cv = state.codec_visible.lock().map(|v| *v).unwrap_or(false);
    WINDOW_WIDTH_DEFAULT
        + if pv { WINDOW_WIDTH_PANEL } else { 0.0 }
        + if cv { WINDOW_WIDTH_PANEL } else { 0.0 }
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
    let width = calc_window_width(&state);
    crate::window_controller::resize_main_window(&app_handle, width, WINDOW_HEIGHT)?;
    Ok(())
}

/// 切换编解码面板：调整主窗口宽度
#[tauri::command]
pub fn set_codec_visible(
    visible: bool,
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    if let Ok(mut cv) = state.codec_visible.lock() {
        *cv = visible;
    }
    let width = calc_window_width(&state);
    crate::window_controller::resize_main_window(&app_handle, width, WINDOW_HEIGHT)?;
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
    let global_changed = config.global_shortcut != new_config.global_shortcut;
    let pin_changed = config.pin_shortcut != new_config.pin_shortcut;
    let capture_changed = config.capture_shortcut != new_config.capture_shortcut;
    *config = new_config;
    save_config(&state.config_path, &config);
    // 广播配置变更事件，通知所有窗口（尤其是主窗口更新主题）
    use tauri::Emitter;
    let _ = app_handle.emit("config-changed", &*config);
    if global_changed || pin_changed || capture_changed {
        if crate::gsettings_shortcuts::is_wayland() {
            if global_changed {
                if let Err(e) = crate::gsettings_shortcuts::update_binding(&config.global_shortcut)
                {
                    log::warn!("更新主窗口快捷键失败: {}", e);
                }
            }
            if pin_changed {
                if let Err(e) = crate::gsettings_shortcuts::update_pin_binding(&config.pin_shortcut)
                {
                    log::warn!("更新 Pin 快捷键失败: {}", e);
                }
            }
            if capture_changed {
                if let Err(e) =
                    crate::gsettings_shortcuts::update_capture_binding(&config.capture_shortcut)
                {
                    log::warn!("更新截图快捷键失败: {}", e);
                }
            }
        } else if let Err(e) = crate::register_x11_shortcuts(&app_handle, &config) {
            log::warn!("更新 X11 快捷键失败: {}", e);
        }
    }
    Ok(())
}

/// 切换 tmux 缓冲区捕获：启用时自动配置 tmux hook，禁用时清除
#[tauri::command]
pub fn toggle_tmux_capture(enabled: bool, state: State<AppState>) -> Result<(), String> {
    if enabled {
        setup_tmux_hook()?;
    } else {
        teardown_tmux_hook();
    }

    // hook 操作成功后再持久化配置
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    config.tmux_capture = enabled;
    save_config(&state.config_path, &config);

    Ok(())
}

/// 检测 tmux 是否可用（前端据此决定是否显示 tmux 选项）
#[tauri::command]
pub fn tmux_available() -> bool {
    std::process::Command::new("tmux")
        .arg("-V")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// 配置 tmux after-copy-mode hook（供 lib.rs 启动时调用）
pub(crate) fn setup_tmux_hook() -> Result<(), String> {
    let buf_path = tmux_buf_path();
    // 确保目录存在并设置安全权限
    if let Some(parent) = buf_path.parent() {
        let _ = std::fs::create_dir_all(parent);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }

    // 验证路径仅含安全字符，防止命令注入
    let path_str = buf_path.to_string_lossy();
    if path_str.contains(['"', '\'', ';', '&', '|']) {
        return Err("tmux 缓冲路径包含不安全字符".to_string());
    }

    // 使用 copy-pipe-and-cancel 直接管道复制内容到文件（绕过 paste buffer 时序问题）
    let pipe_cmd = format!("cat > {}", path_str);

    // 绑定 vi copy-mode 的 y 键
    let output = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode-vi",
            "y",
            "send-keys",
            "-X",
            "copy-pipe-and-cancel",
            &pipe_cmd,
        ])
        .output()
        .map_err(|e| format!("执行 tmux 失败: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("tmux copy-pipe 绑定失败 (y): {}", stderr.trim()));
    }

    // 绑定 Enter 键（部分用户习惯 Enter 复制）
    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode-vi",
            "Enter",
            "send-keys",
            "-X",
            "copy-pipe-and-cancel",
            &pipe_cmd,
        ])
        .output();

    // 绑定鼠标拖选释放
    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode-vi",
            "MouseDragEnd1Pane",
            "send-keys",
            "-X",
            "copy-pipe-and-cancel",
            &pipe_cmd,
        ])
        .output();

    // 同时配置 emacs copy-mode（兼容 mode-keys emacs 用户）
    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode",
            "M-w",
            "send-keys",
            "-X",
            "copy-pipe-and-cancel",
            &pipe_cmd,
        ])
        .output();
    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode",
            "Enter",
            "send-keys",
            "-X",
            "copy-pipe-and-cancel",
            &pipe_cmd,
        ])
        .output();
    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode",
            "MouseDragEnd1Pane",
            "send-keys",
            "-X",
            "copy-pipe-and-cancel",
            &pipe_cmd,
        ])
        .output();

    // 保留 after-copy-mode hook 作为兜底（延迟确保 buffer 已更新）
    let hook_cmd = format!("run-shell -b \"sleep 0.1; tmux save-buffer {}\"", path_str);
    let _ = std::process::Command::new("tmux")
        .args(["set-hook", "-g", "after-copy-mode", &hook_cmd])
        .output();

    log::info!("tmux copy-pipe 绑定和 after-copy-mode hook 已配置");
    Ok(())
}

/// 移除 tmux 绑定和 hook，恢复默认行为
fn teardown_tmux_hook() {
    // 恢复 vi copy-mode 默认绑定
    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode-vi",
            "y",
            "send-keys",
            "-X",
            "copy-selection-and-cancel",
        ])
        .output();
    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode-vi",
            "Enter",
            "send-keys",
            "-X",
            "copy-selection-and-cancel",
        ])
        .output();
    let _ = std::process::Command::new("tmux")
        .args(["unbind-key", "-T", "copy-mode-vi", "MouseDragEnd1Pane"])
        .output();

    // 恢复 emacs copy-mode 默认绑定
    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode",
            "M-w",
            "send-keys",
            "-X",
            "copy-selection-and-cancel",
        ])
        .output();
    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode",
            "Enter",
            "send-keys",
            "-X",
            "copy-selection-and-cancel",
        ])
        .output();
    let _ = std::process::Command::new("tmux")
        .args(["unbind-key", "-T", "copy-mode", "MouseDragEnd1Pane"])
        .output();

    // 移除 hook
    let _ = std::process::Command::new("tmux")
        .args(["set-hook", "-gu", "after-copy-mode"])
        .output();

    // 清理缓冲文件
    let _ = std::fs::remove_file(tmux_buf_path());
    log::info!("tmux 绑定和 hook 已移除");
}

/// tmux 缓冲区文件路径
pub fn tmux_buf_path() -> std::path::PathBuf {
    std::path::PathBuf::from(
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string()),
    )
    .join("clippy")
    .join("tmux-buf")
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
        let mut next = {
            let config = state.config.lock().map_err(|e| e.to_string())?;
            config.clone()
        };
        next.global_shortcut = new_shortcut.clone();
        crate::register_x11_shortcuts(&app_handle, &next)?;
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
    let config = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        config.clone()
    };
    if crate::gsettings_shortcuts::is_wayland() {
        crate::gsettings_shortcuts::resume(
            &config.global_shortcut,
            &config.pin_shortcut,
            &config.capture_shortcut,
        )
    } else {
        crate::register_x11_shortcuts(&app_handle, &config)
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

// ── 截图编辑 ────────────────────────────────────────────────────────────────

/// 启动冻结屏幕截图覆盖层。
#[tauri::command]
pub async fn show_capture_editor(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    crate::capture::show_capture_overlay_for_app(app_handle, &state).await
}

pub(crate) async fn show_capture_editor_for_app(
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let state = app_handle.state::<AppState>();
    crate::capture::show_capture_overlay_for_app(app_handle.clone(), &state).await
}

pub(crate) fn open_capture_window(app_handle: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("capture") {
        window.show().map_err(|error| error.to_string())?;
        let _ = window.set_focus();
        let _ = window.emit("capture-loaded", ());
        return Ok(());
    }

    let window = tauri::WebviewWindowBuilder::new(
        app_handle,
        "capture",
        tauri::WebviewUrl::App("capture.html".into()),
    )
    .title("Clippy Screenshot")
    .inner_size(1180.0, 760.0)
    .min_inner_size(820.0, 560.0)
    .center()
    .resizable(true)
    .build()
    .map_err(|error| error.to_string())?;
    let _ = window.set_focus();
    let _ = window.emit("capture-loaded", ());
    Ok(())
}

/// 返回最近一次由 show_capture_editor 捕获的截图。
#[tauri::command]
pub fn get_pending_capture(
    state: State<AppState>,
) -> Result<crate::screenshot::CapturedScreenshot, String> {
    state
        .latest_capture
        .lock()
        .map_err(|e| e.to_string())?
        .take()
        .ok_or_else(|| "没有待编辑截图".to_string())
}

/// 清理未消费的截图缓存。
#[tauri::command]
pub fn clear_pending_capture(state: State<AppState>) -> Result<(), String> {
    clear_latest_capture(&state);
    Ok(())
}

pub fn clear_latest_capture(state: &AppState) {
    if let Ok(mut latest) = state.latest_capture.lock() {
        *latest = None;
    }
}

/// 将前端生成的 PNG 写入系统剪贴板。
#[tauri::command]
pub async fn copy_screenshot_image(png_base64: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let png = crate::screenshot::decode_png_base64(&png_base64).map_err(|e| e.to_string())?;
        crate::image_io::copy_png_to_clipboard(&png)
    })
    .await
    .map_err(|e| format!("截图复制线程异常: {e}"))?
}

/// 将前端生成的 PNG 保存到 Pictures/Clippy。
#[tauri::command]
pub fn save_screenshot_image(png_base64: String) -> Result<String, String> {
    let png = crate::screenshot::decode_png_base64(&png_base64).map_err(|e| e.to_string())?;
    let path = crate::image_io::save_png(&png, "clippy-screenshot")?;
    Ok(path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_capability_includes_capture_window() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("capabilities")
            .join("default.json");
        let json = std::fs::read_to_string(path).expect("default capability should be readable");
        let value: serde_json::Value =
            serde_json::from_str(&json).expect("default capability should be valid JSON");
        let windows = value
            .get("windows")
            .and_then(serde_json::Value::as_array)
            .expect("default capability should list windows");

        assert!(windows.iter().any(|item| item == "capture"));
    }
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

/// 通过 pkexec 安装 tesseract-ocr，返回安装结果
#[tauri::command]
pub async fn ocr_install() -> Result<String, String> {
    let output = tauri::async_runtime::spawn_blocking(|| {
        std::process::Command::new("pkexec")
            .args([
                "apt-get",
                "install",
                "-y",
                "tesseract-ocr",
                "tesseract-ocr-chi-sim",
            ])
            .output()
    })
    .await
    .map_err(|e| format!("线程异常: {}", e))?
    .map_err(|e| format!("启动 pkexec 失败: {}", e))?;

    if output.status.success() {
        Ok("ok".to_string())
    } else {
        // pkexec exit 126 = 用户取消授权
        let code = output.status.code().unwrap_or(-1);
        if code == 126 {
            Err("cancelled".to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("安装失败: {}", stderr.trim()))
        }
    }
}

/// 获取剪贴板统计信息
#[tauri::command]
pub fn get_stats(state: State<AppState>) -> Result<serde_json::Value, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    storage.get_stats().map_err(|e| e.to_string())
}

/// 抓取 URL 的 Open Graph 元数据（标题、描述、favicon），带 SQLite 缓存
#[tauri::command]
pub async fn fetch_url_meta(url: String, state: State<'_, AppState>) -> Result<UrlMeta, String> {
    // 校验 URL 格式
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("仅支持 http/https URL".to_string());
    }

    // SSRF 防护：拒绝内网/回环地址
    if is_private_url(&url) {
        return Err("不允许请求内网地址".to_string());
    }

    // 查缓存
    {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        if let Ok(Some(cached)) = storage.get_url_meta(&url) {
            return Ok(cached);
        }
    }

    // 在独立线程中执行网络请求，避免阻塞 IPC 线程池
    let url_clone = url.clone();
    let meta = tauri::async_runtime::spawn_blocking(move || -> Result<UrlMeta, String> {
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_global(Some(std::time::Duration::from_secs(5)))
                .build(),
        );
        let resp = agent
            .get(&url_clone)
            .header("User-Agent", "Clippy/0.1 (Link Preview)")
            .call()
            .map_err(|e| format!("请求失败: {}", e))?;

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if !content_type.contains("text/html") {
            return Err("非 HTML 页面".to_string());
        }

        let body = resp
            .into_body()
            .with_config()
            .limit(1_048_576) // 1MB 上限防止恶意响应耗尽内存
            .read_to_string()
            .map_err(|e| format!("读取失败: {}", e))?;

        Ok(parse_og_meta(&url_clone, &body))
    })
    .await
    .map_err(|e| format!("线程异常: {}", e))??;

    // 写缓存
    {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        let _ = storage.set_url_meta(&meta);
    }

    Ok(meta)
}

/// 从 HTML 中解析 Open Graph / 常规 meta 标签
fn parse_og_meta(url: &str, html: &str) -> UrlMeta {
    // 简单解析：不依赖 DOM 库，用正则提取 <meta> 和 <title>
    let get_meta = |property: &str| -> Option<String> {
        // og:xxx 格式
        let pattern = format!(
            r#"<meta[^>]+(?:property|name)=["']{}["'][^>]+content=["']([^"']*)["']"#,
            regex_lite::escape(property)
        );
        if let Ok(re) = regex_lite::Regex::new(&pattern) {
            if let Some(caps) = re.captures(html) {
                let val = caps.get(1)?.as_str().trim().to_string();
                if !val.is_empty() {
                    return Some(html_decode(&val));
                }
            }
        }
        // 反向属性顺序：content 在前
        let pattern2 = format!(
            r#"<meta[^>]+content=["']([^"']*)["'][^>]+(?:property|name)=["']{}["']"#,
            regex_lite::escape(property)
        );
        if let Ok(re) = regex_lite::Regex::new(&pattern2) {
            if let Some(caps) = re.captures(html) {
                let val = caps.get(1)?.as_str().trim().to_string();
                if !val.is_empty() {
                    return Some(html_decode(&val));
                }
            }
        }
        None
    };

    let title = get_meta("og:title").or_else(|| {
        // fallback: <title>...</title>
        let re = regex_lite::Regex::new(r"<title[^>]*>([^<]+)</title>").ok()?;
        let caps = re.captures(html)?;
        let val = caps.get(1)?.as_str().trim().to_string();
        if val.is_empty() {
            None
        } else {
            Some(html_decode(&val))
        }
    });

    let description = get_meta("og:description").or_else(|| get_meta("description"));

    let site_name = get_meta("og:site_name");

    // favicon：优先 <link rel="icon">，fallback /favicon.ico
    let favicon = {
        let re = regex_lite::Regex::new(
            r#"<link[^>]+rel=["'](?:icon|shortcut icon)["'][^>]+href=["']([^"']+)["']"#,
        )
        .ok();
        re.and_then(|r| r.captures(html))
            .and_then(|c| c.get(1))
            .map(|m| {
                let href = m.as_str().trim();
                if href.starts_with("http") {
                    href.to_string()
                } else if href.starts_with("//") {
                    format!("https:{}", href)
                } else {
                    // 相对路径 → 绝对路径
                    let base = url.split('/').take(3).collect::<Vec<_>>().join("/");
                    if href.starts_with('/') {
                        format!("{}{}", base, href)
                    } else {
                        format!("{}/{}", base, href)
                    }
                }
            })
            .or_else(|| {
                let base = url.split('/').take(3).collect::<Vec<_>>().join("/");
                Some(format!("{}/favicon.ico", base))
            })
    };

    UrlMeta {
        url: url.to_string(),
        title,
        description,
        favicon,
        site_name,
    }
}

/// 简单 HTML 实体解码
fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
}

/// SSRF 防护：检测 URL 的主机部分是否为内网/回环/链路本地地址
fn is_private_url(url: &str) -> bool {
    // 提取 host 部分（scheme://host[:port]/path）
    let after_scheme = url.split("://").nth(1).unwrap_or("");
    let host_port = after_scheme.split('/').next().unwrap_or("");
    let host = if host_port.starts_with('[') {
        // IPv6: [::1]:port
        host_port
            .split(']')
            .next()
            .unwrap_or("")
            .trim_start_matches('[')
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    let h = host.to_lowercase();
    h == "localhost"
        || h == "::1"
        || h == "0.0.0.0"
        || h.starts_with("127.")
        || h.starts_with("10.")
        || h.starts_with("192.168.")
        || h.starts_with("172.16.")
        || h.starts_with("172.17.")
        || h.starts_with("172.18.")
        || h.starts_with("172.19.")
        || h.starts_with("172.20.")
        || h.starts_with("172.21.")
        || h.starts_with("172.22.")
        || h.starts_with("172.23.")
        || h.starts_with("172.24.")
        || h.starts_with("172.25.")
        || h.starts_with("172.26.")
        || h.starts_with("172.27.")
        || h.starts_with("172.28.")
        || h.starts_with("172.29.")
        || h.starts_with("172.30.")
        || h.starts_with("172.31.")
        || h.starts_with("169.254.")
        || h.starts_with("fd")
        || h.starts_with("fe80")
}
