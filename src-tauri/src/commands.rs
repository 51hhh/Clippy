use crate::clipboard_watcher::ClipboardWatcher;
use crate::config::save_config;
use crate::models::{AppConfig, ClipItem};
use crate::storage::StorageEngine;
use arboard::Clipboard;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{Manager, State};

/// 全局应用状态，通过 Tauri 的 manage() 注入并在各命令中共享
pub struct AppState {
    pub storage: Arc<Mutex<StorageEngine>>,
    pub config: Arc<Mutex<AppConfig>>,
    pub config_path: PathBuf,
    pub watcher: ClipboardWatcher,
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

/// 将指定条目的文本内容写入系统剪贴板，并隐藏悬浮面板
#[tauri::command]
pub fn select_clip(
    id: i64,
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    // 从存储中读取条目
    let text = {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        let clip = storage.get_clip_by_id(id).map_err(|e| e.to_string())?;
        clip.text_content
    };

    // 将文本写入系统剪贴板，并通知 watcher 跳过此内容
    if let Some(content) = text {
        use sha2::{Digest, Sha256};
        let hash = format!(
            "{:x}",
            Sha256::new_with_prefix(content.as_bytes()).finalize()
        );
        state.watcher.set_skip_hash(hash);

        let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
        clipboard.set_text(content).map_err(|e| e.to_string())?;
    }

    // 隐藏悬浮面板窗口
    if let Some(window) = app_handle.get_webview_window("main") {
        window.hide().map_err(|e| e.to_string())?;
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
    *config = new_config;
    save_config(&state.config_path, &config);
    // 广播配置变更事件，通知所有窗口（尤其是主窗口更新主题）
    use tauri::Emitter;
    let _ = app_handle.emit("config-changed", &*config);
    Ok(())
}

/// 动态更新全局快捷键：注销旧快捷键，注册新快捷键，并持久化到配置
#[tauri::command]
pub fn update_shortcut(
    new_shortcut: String,
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

    // 注销所有已注册的快捷键
    app_handle
        .global_shortcut()
        .unregister_all()
        .map_err(|e| e.to_string())?;

    // 注册新快捷键（切换主窗口可见性）
    let handle = app_handle.clone();
    app_handle
        .global_shortcut()
        .on_shortcut(new_shortcut.as_str(), move |_app, _shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            if let Some(window) = handle.get_webview_window("main") {
                if window.is_visible().unwrap_or(false) {
                    let _ = window.hide();
                } else {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .map_err(|e| e.to_string())?;

    // 更新配置并持久化
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
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    app_handle
        .global_shortcut()
        .unregister_all()
        .map_err(|e| e.to_string())
}

/// 恢复全局快捷键（录制结束后调用）
#[tauri::command]
pub fn resume_shortcuts(
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    let config = state.config.lock().map_err(|e| e.to_string())?;
    let shortcut_str = config.global_shortcut.clone();
    drop(config);

    let handle = app_handle.clone();
    app_handle
        .global_shortcut()
        .on_shortcut(shortcut_str.as_str(), move |_app, _shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            if let Some(window) = handle.get_webview_window("main") {
                if window.is_visible().unwrap_or(false) {
                    let _ = window.hide();
                } else {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .map_err(|e| e.to_string())
}
