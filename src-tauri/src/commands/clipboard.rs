use super::AppState;
use crate::models::{ClipItem, ContentType};
use crate::paste::{PasteOutcome, PasteStatus};
use base64::{engine::general_purpose::STANDARD, Engine};
use tauri::{Emitter, Manager, State};

/// 查询剪贴板历史列表，支持全文搜索和收藏过滤。
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

/// 删除指定 id 的剪贴板条目。
#[tauri::command]
pub fn delete_clip(id: i64, state: State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    storage.delete_clip(id).map_err(|e| e.to_string())
}

/// 切换指定条目的收藏状态，返回新的收藏状态。
#[tauri::command]
pub fn toggle_favorite(id: i64, state: State<AppState>) -> Result<bool, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    storage.toggle_favorite(id).map_err(|e| e.to_string())
}

/// 清空所有历史（保留收藏条目）。
#[tauri::command]
pub fn clear_history(state: State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    storage.clear_history().map_err(|e| e.to_string())
}

/// 写入系统剪贴板，隐藏面板，并按当前平台尝试自动粘贴。
#[tauri::command]
pub async fn select_clip(
    id: i64,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<PasteOutcome, String> {
    write_clip_to_clipboard(id, &state)?;

    let updated_clip = {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        storage.touch_clip(id).map_err(|e| e.to_string())?
    };
    let _ = app_handle.emit("clip-added", &updated_clip);

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

/// 仅将用户明确请求的文本写入系统剪贴板。
///
/// 该路径不会创建历史条目，也不会触发自动粘贴；watcher 会跳过本次写入，
/// 因此翻译/OCR/编解码结果不会反过来污染剪贴板历史。
#[tauri::command]
pub fn copy_text(text: String, state: State<AppState>) -> Result<(), String> {
    use sha2::{Digest, Sha256};

    let hash = format!("{:x}", Sha256::new_with_prefix(text.as_bytes()).finalize());
    state.watcher.set_skip_hash(hash);
    crate::clipboard_watcher::clipboard_set_text_with_retry(&text)
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
            let image_bytes = {
                let storage = state.storage.lock().map_err(|e| e.to_string())?;
                storage
                    .get_clip_image(id)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "图片数据为空".to_string())?
            };

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
    let auto_paste_enabled = state.config.lock().map_err(|e| e.to_string())?.auto_paste;
    state
        .paste_manager
        .request_permission(auto_paste_enabled)
        .await
}

/// 按 id 获取图片数据，返回 base64 编码的 PNG。
#[tauri::command]
pub fn get_clip_image(id: i64, state: State<AppState>) -> Result<Option<String>, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let data = storage.get_clip_image(id).map_err(|e| e.to_string())?;
    Ok(data.map(|bytes| STANDARD.encode(&bytes)))
}

/// 按 id 获取完整条目（含 html_content），用于预览面板按需加载。
#[tauri::command]
pub fn get_clip_detail(id: i64, state: State<AppState>) -> Result<ClipItem, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let mut clip = storage.get_clip_by_id(id).map_err(|e| e.to_string())?;
    clip.image_data = None;
    Ok(clip)
}

/// 获取剪贴板统计信息。
#[tauri::command]
pub fn get_stats(state: State<AppState>) -> Result<serde_json::Value, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    storage.get_stats().map_err(|e| e.to_string())
}
