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
    /// 持有监听器以维持其生命周期（停止信号在 Drop 时无需显式触发）
    #[allow(dead_code)]
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

    // 将文本写入系统剪贴板
    if let Some(content) = text {
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
pub fn update_config(new_config: AppConfig, state: State<AppState>) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    *config = new_config;
    save_config(&state.config_path, &config);
    Ok(())
}
