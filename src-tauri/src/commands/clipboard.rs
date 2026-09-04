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
    // 事件里不带图片数据，前端按需取（见 `ClipItem::without_image_data`）。
    let _ = app_handle.emit("clip-added", &updated_clip.without_image_data());

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
            Some("auto_paste_disabled".to_string()),
            Some("Automatic paste is disabled".to_string()),
        ));
    }

    match state.paste_manager.paste().await {
        Ok(outcome) => {
            if !outcome.pasted {
                reveal_paste_fallback(&app_handle, &outcome);
            }
            Ok(outcome)
        }
        Err(error) => {
            // Wayland 尚未授权是正常路径：用户可以在设置里显式授权，这里只降级为纯复制。
            let context = "自动粘贴失败，内容已保留在剪贴板";
            let reason_code = error.code().to_string();
            let detail = if error.is_authorization_failure() {
                crate::error::note(context, error)
            } else {
                crate::error::report(context, error)
            };
            let outcome = PasteOutcome::copied_only(
                state.paste_manager.backend(),
                Some(reason_code),
                Some(detail),
            );
            reveal_paste_fallback(&app_handle, &outcome);
            Ok(outcome)
        }
    }
}

/// 自动粘贴失败时内容已安全进入剪贴板，但不能让主面板继续隐藏：
/// 否则用户既看不到降级原因，也不知道可以手动按 Ctrl/Cmd+V。
fn reveal_paste_fallback(app_handle: &tauri::AppHandle, outcome: &PasteOutcome) {
    if let Err(error) = crate::window_controller::show_main_window(app_handle) {
        log::warn!("自动粘贴降级后恢复主面板失败: {error}");
    }
    let _ = app_handle.emit("paste-fallback", outcome);
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
            // 上面那次 `get_clip_by_id` 已经把这张图整份读出来了，别再查一遍库：
            // 全屏截图是几 MB，读两遍就是两次几 MB 的拷贝，还多锁一次 storage。
            let image_bytes = clip.image_data.ok_or_else(|| "图片数据为空".to_string())?;
            // 图片这一路**没有** skip hash：watcher 哈希的是它自己从剪贴板 RGBA 重新编出来
            // 的 PNG，和库里这串字节几乎不可能一致，设了也永远匹配不上（白算一次全图
            // sha256）。文本那几路能生效是因为两边哈希的是同一串字节。后果只是这条图片会被
            // 顶到历史最前面——`insert_clip` 按哈希去重，不会多存一份。
            crate::image_io::copy_png_to_clipboard(&image_bytes)?;
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
        .map_err(|error| crate::error::report("请求自动粘贴授权失败", error))
}

/// 按 id 获取**原图**，返回 base64 编码的 PNG。只给预览面板与贴图用。
///
/// **列表行不要用这个，用 `get_clip_thumbnail`。** 一张全屏截图是几 MB，
/// 行里那格只有 48 px。
///
/// `async`：同步命令跑在 GTK 主线程上，而这条路要读一个几 MB 的 blob 再 base64 编一遍
/// （4 MB 的图约 3 ms，还要多分配 33% 的内存）。打开面板时前端会为每个图片条目各发一次，
/// 十几条串起来就是主线程上一段肉眼可见的卡顿。阻塞工作必须进 blocking pool：只把函数
/// 声明成 `async` 仍会占住 Tauri runtime worker，和其他异步命令争执行线程。
#[tauri::command]
pub async fn get_clip_image(id: i64, state: State<'_, AppState>) -> Result<Option<String>, String> {
    let storage = std::sync::Arc::clone(&state.storage);
    tauri::async_runtime::spawn_blocking(move || -> Result<Option<String>, String> {
        let data = {
            let storage = storage.lock().map_err(|e| e.to_string())?;
            storage.get_clip_image(id).map_err(|e| e.to_string())?
        };
        Ok(data.map(|bytes| STANDARD.encode(&bytes)))
    })
    .await
    .map_err(|error| format!("读取剪贴板原图线程异常: {error}"))?
}

/// 列表行缩略图的最长边（像素）。行里那一格是 48 CSS px，2x 屏上 96 物理像素，
/// 128 留了余量。
const THUMBNAIL_MAX_EDGE: u32 = 128;

/// 缩略图缓存的条数上限。一条 128 px 的 PNG base64 大约 3~8 KB，64 条不到半兆。
const THUMBNAIL_CACHE_CAPACITY: usize = 64;

/// 列表行用的缩略图，base64 编码的 PNG；非图片条目返回 `None`。
///
/// 为什么不复用 `get_clip_image`：行里那格是 48×48，库里存的是原图。为了画 48 px 把整张
/// 原图送进 webview 再解码，一次开面板十几个图片条目就是几十 MB IPC 加十几次全尺寸 PNG
/// 解码，全部落在 webview 那一个线程上。缩到 128 px 之后一条几 KB，base64 那 33% 的膨胀
/// 也就无所谓了，不必为它把这条路改成二进制 IPC。
///
/// 结果按 id 缓存：缩一次要解一遍原图（2560×1600 约 50 ms），而同一条记录反复出现在
/// 每次开面板的列表里，内容又是不可变的（`clips.id` 自增不复用），所以缓存永不失效。
#[tauri::command]
pub async fn get_clip_thumbnail(
    id: i64,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    if let Some(hit) = thumbnail_cache().get(id) {
        return Ok(Some(hit));
    }
    let storage = std::sync::Arc::clone(&state.storage);
    tauri::async_runtime::spawn_blocking(move || -> Result<Option<String>, String> {
        // 数据库 blob、整图 PNG 解码、缩放与重编码都是阻塞/CPU 工作。冷缓存同时出现多张
        // 图片时不能占满 Tauri async worker，否则设置、翻译等无关异步命令也会一起迟钝。
        let data = {
            let storage = storage.lock().map_err(|e| e.to_string())?;
            storage.get_clip_image(id).map_err(|e| e.to_string())?
        };
        let Some(png) = data else {
            return Ok(None);
        };
        let encoded = STANDARD.encode(crate::image_io::thumbnail_png(&png, THUMBNAIL_MAX_EDGE)?);
        thumbnail_cache().put(id, encoded.clone());
        Ok(Some(encoded))
    })
    .await
    .map_err(|error| format!("生成剪贴板缩略图线程异常: {error}"))?
}

fn thumbnail_cache() -> &'static ThumbnailCache {
    static CACHE: std::sync::OnceLock<ThumbnailCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(ThumbnailCache::default)
}

/// 先进先出的缩略图缓存。
///
/// 进程级静态而不是挂在 `AppState` 上：这里存的是"某个 id 的图缩小之后长什么样"，
/// 与配置、窗口、会话都无关，而 id 不复用意味着它永远不会答错。
#[derive(Default)]
struct ThumbnailCache {
    entries: std::sync::Mutex<std::collections::VecDeque<(i64, String)>>,
}

impl ThumbnailCache {
    fn get(&self, id: i64) -> Option<String> {
        let entries = self.entries.lock().ok()?;
        entries
            .iter()
            .find(|(key, _)| *key == id)
            .map(|(_, value)| value.clone())
    }

    fn put(&self, id: i64, value: String) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        // 并发的两次请求可能都没命中就各自算了一遍，别让同一个 id 占两格。
        if entries.iter().any(|(key, _)| *key == id) {
            return;
        }
        if entries.len() >= THUMBNAIL_CACHE_CAPACITY {
            entries.pop_front();
        }
        entries.push_back((id, value));
    }
}

/// 按 id 获取完整条目（含 html_content），用于预览面板按需加载。
#[tauri::command]
pub fn get_clip_detail(id: i64, state: State<AppState>) -> Result<ClipItem, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    let clip = storage.get_clip_by_id(id).map_err(|e| e.to_string())?;
    Ok(clip.without_image_data())
}

/// 获取剪贴板统计信息。
#[tauri::command]
pub fn get_stats(state: State<AppState>) -> Result<serde_json::Value, String> {
    let storage = state.storage.lock().map_err(|e| e.to_string())?;
    storage.get_stats().map_err(|e| e.to_string())
}
