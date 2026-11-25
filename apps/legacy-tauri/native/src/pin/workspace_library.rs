//! 用户明确保存的 Pin 工作区浏览器。
//!
//! WebView 只看到不透明的工作区 id 和轻量摘要。原图、修订文档、磁盘路径与窗口 label
//! 都留在 Rust 边界内；图片缩略图按需从受管根图重放后生成。

use super::output::{render_document, PinCanvasProject};
use super::workspace;
use crate::commands::AppState;
use crate::models::ContentType;
use crate::storage::{PinWorkspaceGroup, StoredPinWorkspaceContent, StoredPinWorkspaceSummary};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Serialize;
use std::collections::{HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};
use tauri::{Emitter, Manager, WebviewWindow};

const LIBRARY_LABEL: &str = "pin-workspaces";
const LIBRARY_PAGE: &str = "pin-workspaces.html";
const THUMBNAIL_MAX_EDGE: u32 = 256;
const THUMBNAIL_CACHE_CAPACITY: usize = 64;
const THUMBNAIL_MAX_CONCURRENCY: usize = 2;
pub(crate) const LIBRARY_CHANGED: &str = "pin-workspace-library-changed";

pub(crate) fn notify_changed(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window(LIBRARY_LABEL) else {
        return;
    };
    if let Err(error) = window.emit(LIBRARY_CHANGED, ()) {
        log::debug!("通知 Pin 工作区浏览器刷新失败: {error}");
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PinWorkspaceLibrarySettings {
    language: String,
    theme: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PinWorkspaceLibrarySnapshot {
    groups: Vec<PinWorkspaceGroup>,
    items: Vec<PinWorkspaceLibraryItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PinWorkspaceLibraryItem {
    id: i64,
    group_id: Option<i64>,
    content_type: ContentType,
    preview_text: Option<String>,
    content_width: f64,
    content_height: f64,
    scale: f64,
    opacity: f64,
    locked: bool,
    above: bool,
    updated_at: i64,
    open: bool,
}

impl PinWorkspaceLibraryItem {
    fn from_summary(summary: StoredPinWorkspaceSummary, open_ids: &HashSet<i64>) -> Self {
        Self {
            id: summary.id,
            group_id: summary.group_id,
            content_type: summary.content_type,
            preview_text: summary.preview_text,
            content_width: summary.content_width,
            content_height: summary.content_height,
            scale: summary.scale,
            opacity: summary.opacity,
            locked: summary.locked,
            above: summary.above,
            updated_at: summary.updated_at,
            open: open_ids.contains(&summary.id),
        }
    }
}

fn ensure_library(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == LIBRARY_LABEL {
        Ok(())
    } else {
        Err("Pin 工作区浏览器调用窗口无效".to_string())
    }
}

pub(crate) fn open(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(LIBRARY_LABEL) {
        window
            .unminimize()
            .and_then(|_| window.show())
            .and_then(|_| window.set_focus())
            .map_err(|error| format!("打开 Pin 工作区浏览器失败: {error}"))?;
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(
        app,
        LIBRARY_LABEL,
        tauri::WebviewUrl::App(LIBRARY_PAGE.into()),
    )
    .title("Clippy — Pin Workspaces")
    .inner_size(820.0, 640.0)
    .min_inner_size(420.0, 360.0)
    .center()
    .resizable(true)
    .decorations(false)
    .always_on_top(false)
    .skip_taskbar(false)
    .visible(false)
    .build()
    .map_err(|error| format!("创建 Pin 工作区浏览器失败: {error}"))?;
    Ok(())
}

#[tauri::command]
pub(crate) fn get_pin_workspace_library_settings(
    window: WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<PinWorkspaceLibrarySettings, String> {
    ensure_library(&window)?;
    let config = state.config.lock().map_err(|error| error.to_string())?;
    Ok(PinWorkspaceLibrarySettings {
        language: config.language.clone(),
        theme: config.theme.clone(),
    })
}

#[tauri::command]
pub(crate) fn list_pin_workspace_library(
    window: WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<PinWorkspaceLibrarySnapshot, String> {
    ensure_library(&window)?;
    let (groups, summaries) = {
        let storage = state.storage.lock().map_err(|error| error.to_string())?;
        (
            storage
                .list_pin_groups()
                .map_err(|error| error.to_string())?,
            storage
                .list_pin_workspace_summaries()
                .map_err(|error| error.to_string())?,
        )
    };
    let open_ids = state
        .pin_manager
        .open_workspace_ids()?
        .into_iter()
        .collect::<HashSet<_>>();
    Ok(PinWorkspaceLibrarySnapshot {
        groups,
        items: summaries
            .into_iter()
            .map(|summary| PinWorkspaceLibraryItem::from_summary(summary, &open_ids))
            .collect(),
    })
}

#[tauri::command]
pub(crate) async fn get_pin_workspace_thumbnail(
    window: WebviewWindow,
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<Option<String>, String> {
    ensure_library(&window)?;
    let identity = state
        .storage
        .lock()
        .map_err(|error| error.to_string())?
        .get_pin_workspace_content_identity(id)
        .map_err(|error| error.to_string())?;
    let Some((content_type, content_hash)) = identity else {
        return Err("Pin 工作区记录不存在".to_string());
    };
    if content_type != ContentType::Image {
        return Ok(None);
    }
    let key = ThumbnailKey { id, content_hash };
    if let Some(hit) = thumbnail_cache().get(&key) {
        return Ok(Some(hit));
    }
    let _permit = thumbnail_gate()
        .acquire()
        .await
        .map_err(|_| "Pin 缩略图生成队列已关闭".to_string())?;
    if let Some(hit) = thumbnail_cache().get(&key) {
        return Ok(Some(hit));
    }
    let storage = std::sync::Arc::clone(&state.storage);
    tauri::async_runtime::spawn_blocking(move || {
        let item = storage
            .lock()
            .map_err(|error| error.to_string())?
            .load_pin_workspace_item(id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Pin 工作区记录不存在".to_string())?;
        let StoredPinWorkspaceContent::Image { revision, .. } = item.content else {
            return Ok(None);
        };
        let document = PinCanvasProject {
            renderer_version: revision.renderer_version,
            source_width: revision.source_width,
            source_height: revision.source_height,
            annotations: revision.annotations,
            adjustments: revision.adjustments,
        };
        let rendered = render_document(&revision.source_png, Some(&document))?;
        let thumbnail = crate::image_io::thumbnail_png(&rendered, THUMBNAIL_MAX_EDGE)?;
        let encoded = STANDARD.encode(thumbnail);
        thumbnail_cache().put(key, encoded.clone());
        Ok(Some(encoded))
    })
    .await
    .map_err(|error| format!("生成 Pin 工作区缩略图线程异常: {error}"))?
}

#[tauri::command]
pub(crate) async fn show_pin_workspace_item(
    window: WebviewWindow,
    app: tauri::AppHandle,
    id: i64,
) -> Result<bool, String> {
    ensure_library(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _transition = state
            .pin_transition
            .lock()
            .map_err(|error| error.to_string())?;
        if let Some(label) = state.pin_manager.label_for_workspace(id)? {
            if let Some(pin_window) = app.get_webview_window(&label) {
                pin_window.show().map_err(|error| error.to_string())?;
                let _ = pin_window.set_focus();
                let _ = pin_window.emit("pin-already-open", ());
                return Ok(true);
            }
            let _ = state.pin_manager.remove(&label);
        }
        let item = state
            .storage
            .lock()
            .map_err(|error| error.to_string())?
            .load_pin_workspace_item(id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Pin 工作区记录不存在".to_string())?;
        workspace::restore_one(&app, &state, item)?;
        Ok(false)
    })
    .await
    .map_err(|error| format!("打开 Pin 工作区线程异常: {error}"))?
}

#[tauri::command]
pub(crate) fn assign_pin_workspace_library_group(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: i64,
    group_id: Option<i64>,
) -> Result<(), String> {
    ensure_library(&window)?;
    let _transition = state
        .pin_transition
        .lock()
        .map_err(|error| error.to_string())?;
    let changed = state
        .storage
        .lock()
        .map_err(|error| error.to_string())?
        .set_pin_workspace_group(id, group_id)
        .map_err(|error| error.to_string())?;
    if !changed {
        return Err("Pin 工作区记录不存在".to_string());
    }
    if let Some(label) = state.pin_manager.label_for_workspace(id)? {
        state
            .pin_manager
            .set_workspace(&label, Some(id), group_id)?;
        workspace::emit_change(&app, &label, Some(id), group_id);
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn remove_pin_workspace_library_item(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    ensure_library(&window)?;
    let _transition = state
        .pin_transition
        .lock()
        .map_err(|error| error.to_string())?;
    let label = state.pin_manager.label_for_workspace(id)?;
    let removed = state
        .storage
        .lock()
        .map_err(|error| error.to_string())?
        .delete_pin_workspace_item(id)
        .map_err(|error| error.to_string())?;
    if !removed {
        return Err("Pin 工作区记录不存在".to_string());
    }
    if let Some(label) = label {
        state.pin_manager.set_workspace(&label, None, None)?;
        workspace::emit_change(&app, &label, None, None);
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn create_pin_workspace_library_group(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<PinWorkspaceGroup, String> {
    ensure_library(&window)?;
    workspace::create_group(&name, &app, &state)
}

#[tauri::command]
pub(crate) fn rename_pin_workspace_library_group(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: i64,
    name: String,
) -> Result<bool, String> {
    ensure_library(&window)?;
    workspace::rename_group(id, &name, &app, &state)
}

#[tauri::command]
pub(crate) fn delete_pin_workspace_library_group(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<bool, String> {
    ensure_library(&window)?;
    let _transition = state
        .pin_transition
        .lock()
        .map_err(|error| error.to_string())?;
    workspace::delete_group(id, &app, &state)
}

#[tauri::command]
pub(crate) fn pin_workspace_library_ready(window: WebviewWindow) -> Result<(), String> {
    ensure_library(&window)?;
    window
        .show()
        .and_then(|_| window.set_focus())
        .map_err(|error| format!("显示 Pin 工作区浏览器失败: {error}"))
}

#[tauri::command]
pub(crate) fn start_pin_workspace_library_drag(window: WebviewWindow) -> Result<(), String> {
    ensure_library(&window)?;
    window
        .start_dragging()
        .map_err(|error| format!("拖动 Pin 工作区浏览器失败: {error}"))
}

#[tauri::command]
pub(crate) fn close_pin_workspace_library(window: WebviewWindow) -> Result<(), String> {
    ensure_library(&window)?;
    window
        .destroy()
        .map_err(|error| format!("关闭 Pin 工作区浏览器失败: {error}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ThumbnailKey {
    id: i64,
    content_hash: String,
}

#[derive(Default)]
struct ThumbnailCache {
    entries: Mutex<VecDeque<(ThumbnailKey, String)>>,
}

impl ThumbnailCache {
    fn get(&self, key: &ThumbnailKey) -> Option<String> {
        self.entries
            .lock()
            .ok()?
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.clone())
    }

    fn put(&self, key: ThumbnailKey, value: String) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        if entries.iter().any(|(candidate, _)| candidate == &key) {
            return;
        }
        if entries.len() >= THUMBNAIL_CACHE_CAPACITY {
            entries.pop_front();
        }
        entries.push_back((key, value));
    }
}

fn thumbnail_cache() -> &'static ThumbnailCache {
    static CACHE: OnceLock<ThumbnailCache> = OnceLock::new();
    CACHE.get_or_init(ThumbnailCache::default)
}

fn thumbnail_gate() -> &'static tokio::sync::Semaphore {
    static GATE: tokio::sync::Semaphore =
        tokio::sync::Semaphore::const_new(THUMBNAIL_MAX_CONCURRENCY);
    &GATE
}
