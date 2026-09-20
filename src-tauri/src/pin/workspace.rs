//! 用户明确保存的 Pin 工作区。
//!
//! 临时 Pin 只活在 `PinManager`；只有本模块的保存命令会写入 SQLite。图片内容引用
//! `image_revisions` 的根图与累计操作，工作区本身只保存展示状态和修订 id。

use super::model::{PinEntry, PinSource, SharpenSlot};
use super::output::{
    effective_project, identity_project, register_image_revision, render_document,
};
use super::window::{
    capture_workspace_placement, content_buffer_scale, content_device_scale, create_pin_window,
    outer_size, restore_workspace_position,
};
use crate::commands::AppState;
use crate::models::{ClipItem, ContentType};
use crate::storage::{
    PinWorkspaceGroup, PinWorkspaceItemWrite, PinWorkspacePresentation, StoredPinWorkspaceContent,
    StoredPinWorkspaceItem,
};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager};

const PRESENTATION_FLUSH_DELAY: Duration = Duration::from_millis(180);
pub(super) const PIN_WORKSPACE_CHANGED: &str = "pin-workspace-changed";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PinWorkspaceStatus {
    pub workspace_id: i64,
    pub group_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct PinWorkspaceChange {
    pub workspace_id: Option<i64>,
    pub group_id: Option<i64>,
}

pub(super) fn emit_change(
    app_handle: &tauri::AppHandle,
    label: &str,
    workspace_id: Option<i64>,
    group_id: Option<i64>,
) {
    let Some(window) = app_handle.get_webview_window(label) else {
        return;
    };
    if let Err(error) = window.emit(
        PIN_WORKSPACE_CHANGED,
        PinWorkspaceChange {
            workspace_id,
            group_id: workspace_id.and(group_id),
        },
    ) {
        log::debug!("同步 Pin 工作区状态到 {label} 失败: {error}");
    }
}

#[derive(Debug, Clone)]
struct PendingPresentation {
    label: String,
    presentation: PinWorkspacePresentation,
}

/// 缩放、透明度与拖动都可能高频触发；合并成每个工作区条目的最新状态再落库。
pub(crate) struct PinWorkspacePersistence {
    storage: Arc<Mutex<crate::storage::StorageEngine>>,
    pending: Mutex<HashMap<i64, PendingPresentation>>,
    worker_scheduled: AtomicBool,
}

impl PinWorkspacePersistence {
    pub(crate) fn new(storage: Arc<Mutex<crate::storage::StorageEngine>>) -> Self {
        Self {
            storage,
            pending: Mutex::new(HashMap::new()),
            worker_scheduled: AtomicBool::new(false),
        }
    }

    fn queue(self: &Arc<Self>, app_handle: tauri::AppHandle, entry: &PinEntry) {
        let Some(id) = entry.workspace_id else {
            return;
        };
        let pending = PendingPresentation {
            label: entry.label.clone(),
            presentation: presentation_from_entry(entry, None),
        };
        let Ok(mut items) = self.pending.lock() else {
            log::warn!("Pin 工作区待保存状态锁损坏，跳过本次展示状态保存");
            return;
        };
        items.insert(id, pending);
        drop(items);
        if self
            .worker_scheduled
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let persistence = Arc::clone(self);
        std::thread::spawn(move || persistence.flush_loop(app_handle));
    }

    fn flush_loop(&self, app_handle: tauri::AppHandle) {
        loop {
            std::thread::sleep(PRESENTATION_FLUSH_DELAY);
            let batch = self
                .pending
                .lock()
                .map(|mut pending| pending.drain().collect::<Vec<_>>())
                .unwrap_or_default();
            if !batch.is_empty() {
                let updates = batch
                    .into_iter()
                    .map(|(id, mut pending)| {
                        pending.presentation.placement =
                            capture_workspace_placement(&app_handle, &pending.label);
                        (id, pending.presentation)
                    })
                    .collect::<Vec<_>>();
                match self.storage.lock() {
                    Ok(storage) => {
                        for (id, presentation) in updates {
                            match storage.update_pin_workspace_presentation(id, &presentation) {
                                Ok(true) => {}
                                Ok(false) => {
                                    log::debug!("Pin 工作区记录 {id} 已删除，丢弃迟到的展示状态")
                                }
                                Err(error) => {
                                    log::warn!("保存 Pin 工作区展示状态 {id} 失败: {error}")
                                }
                            }
                        }
                    }
                    Err(error) => log::warn!("保存 Pin 工作区展示状态时数据库锁损坏: {error}"),
                }
            }

            self.worker_scheduled.store(false, Ordering::Release);
            let has_pending = self
                .pending
                .lock()
                .map(|pending| !pending.is_empty())
                .unwrap_or(false);
            if !has_pending
                || self
                    .worker_scheduled
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
            {
                return;
            }
        }
    }
}

pub(crate) fn queue_open_pin(app_handle: &tauri::AppHandle, state: &AppState, label: &str) {
    if let Ok(entry) = state.pin_manager.get(label) {
        state
            .pin_workspace_persistence
            .queue(app_handle.clone(), &entry);
    }
}

pub(crate) fn save(
    label: &str,
    group_id: Option<i64>,
    submitted: Option<&super::output::PinCanvasProject>,
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<PinWorkspaceStatus, String> {
    super::model::validate_label(label)?;
    let entry = state.pin_manager.get(label)?;
    let group_id = group_id.or(entry.workspace_group_id);
    let placement = capture_workspace_placement(app_handle, label);
    let storage = state.storage.lock().map_err(|error| error.to_string())?;
    let (revision_id, content_type, text, html, content_hash) = match &*entry.source {
        PinSource::Clip { item, image: None } => (
            None,
            item.content_type.clone(),
            item.text_content.as_deref(),
            item.html_content.as_deref(),
            item.content_hash.clone(),
        ),
        source => {
            let source_png = super::output::source_png(source)
                .ok_or_else(|| "Pin 工作区图片缺少原图".to_string())?;
            let project = effective_project(&entry, submitted)
                .map(Ok)
                .unwrap_or_else(|| identity_project(source_png))?;
            let rendered = render_document(source_png, Some(&project))?;
            let revision_id = register_image_revision(&storage, &entry, &project, &rendered)?;
            (
                Some(revision_id),
                ContentType::Image,
                None,
                None,
                crate::clipboard_watcher::content::compute_hash(&rendered),
            )
        }
    };
    let id = storage
        .upsert_pin_workspace_item(PinWorkspaceItemWrite {
            id: entry.workspace_id,
            group_id,
            revision_id,
            content_type,
            text_content: text,
            html_content: html,
            content_hash: &content_hash,
            content_width: entry.content_width,
            content_height: entry.content_height,
            scale: entry.scale,
            opacity: entry.opacity,
            locked: entry.locked,
            above: entry.above,
            placement: placement.as_ref(),
        })
        .map_err(|error| error.to_string())?;
    drop(storage);
    state.pin_manager.set_workspace(label, Some(id), group_id)?;
    super::workspace_library::notify_changed(app_handle);
    Ok(PinWorkspaceStatus {
        workspace_id: id,
        group_id,
    })
}

pub(crate) fn remove(
    label: &str,
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<(), String> {
    super::model::validate_label(label)?;
    let entry = state.pin_manager.get(label)?;
    if let Some(id) = entry.workspace_id {
        state
            .storage
            .lock()
            .map_err(|error| error.to_string())?
            .delete_pin_workspace_item(id)
            .map_err(|error| error.to_string())?;
    }
    state.pin_manager.set_workspace(label, None, None)?;
    super::workspace_library::notify_changed(app_handle);
    Ok(())
}

pub(crate) fn list_groups(state: &AppState) -> Result<Vec<PinWorkspaceGroup>, String> {
    state
        .storage
        .lock()
        .map_err(|error| error.to_string())?
        .list_pin_groups()
        .map_err(|error| error.to_string())
}

pub(crate) fn create_group(
    name: &str,
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<PinWorkspaceGroup, String> {
    let group = state
        .storage
        .lock()
        .map_err(|error| error.to_string())?
        .create_pin_group(name)
        .map_err(|error| error.to_string())?;
    super::workspace_library::notify_changed(app_handle);
    Ok(group)
}

pub(crate) fn rename_group(
    id: i64,
    name: &str,
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<bool, String> {
    let renamed = state
        .storage
        .lock()
        .map_err(|error| error.to_string())?
        .rename_pin_group(id, name)
        .map_err(|error| error.to_string())?;
    if renamed {
        super::workspace_library::notify_changed(app_handle);
    }
    Ok(renamed)
}

pub(crate) fn delete_group(
    id: i64,
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<bool, String> {
    let labels = state.pin_manager.labels_in_workspace_group(id)?;
    let deleted = state
        .storage
        .lock()
        .map_err(|error| error.to_string())?
        .delete_pin_group(id)
        .map_err(|error| error.to_string())?;
    if deleted {
        state.pin_manager.clear_workspace_group(id)?;
        for label in labels {
            let workspace_id = state.pin_manager.get(&label)?.workspace_id;
            emit_change(app_handle, &label, workspace_id, None);
        }
        super::workspace_library::notify_changed(app_handle);
    }
    Ok(deleted)
}

pub(crate) fn assign_group(
    label: &str,
    group_id: Option<i64>,
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<(), String> {
    super::model::validate_label(label)?;
    let entry = state.pin_manager.get(label)?;
    let id = entry
        .workspace_id
        .ok_or_else(|| "临时 Pin 尚未保存到工作区".to_string())?;
    let changed = state
        .storage
        .lock()
        .map_err(|error| error.to_string())?
        .set_pin_workspace_group(id, group_id)
        .map_err(|error| error.to_string())?;
    if !changed {
        return Err("Pin 工作区记录不存在".to_string());
    }
    state.pin_manager.set_workspace(label, Some(id), group_id)?;
    emit_change(app_handle, label, Some(id), group_id);
    super::workspace_library::notify_changed(app_handle);
    Ok(())
}

pub(crate) fn restore_saved(app_handle: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    let items = state
        .storage
        .lock()
        .map_err(|error| error.to_string())?
        .load_pin_workspace_items()
        .map_err(|error| error.to_string())?;
    for item in items {
        let id = item.id;
        if let Err(error) = restore_one(app_handle, state, item) {
            log::warn!("恢复 Pin 工作区记录 {id} 失败，已跳过: {error}");
        }
    }
    Ok(())
}

pub(super) fn restore_one(
    app_handle: &tauri::AppHandle,
    state: &AppState,
    item: StoredPinWorkspaceItem,
) -> Result<(), String> {
    let label = format!("pin-workspace-{}", item.id);
    if app_handle.get_webview_window(&label).is_some() {
        return Ok(());
    }
    let source = match item.content {
        StoredPinWorkspaceContent::Image { revision, .. } => {
            let document = super::output::PinCanvasProject {
                renderer_version: revision.renderer_version,
                source_width: revision.source_width,
                source_height: revision.source_height,
                annotations: revision.annotations.clone(),
                adjustments: revision.adjustments.clone(),
            };
            let preview_png = render_document(&revision.source_png, Some(&document))?;
            let project = super::project::RuntimeProject::from_managed(
                &revision.source_png,
                &preview_png,
                revision.renderer_version,
                revision.source_width,
                revision.source_height,
                revision.annotations,
                revision.adjustments,
            )?;
            PinSource::Project {
                source_png: revision.source_png,
                preview_png,
                project,
            }
        }
        StoredPinWorkspaceContent::Snapshot {
            content_type,
            text_content,
            html_content,
            content_hash,
        } => PinSource::Clip {
            item: ClipItem {
                id: -item.id,
                content_type,
                text_content,
                html_content,
                image_data: None,
                content_hash,
                is_favorite: false,
                is_sensitive: false,
                created_at: 0,
                byte_size: 0,
            },
            image: None,
        },
    };
    let restore_position = item.placement.as_ref().map(|placement| {
        let (width, height) = outer_size(item.content_width, item.content_height, item.scale);
        restore_workspace_position(app_handle, placement, width, height)
    });
    let scale_origin = restore_position.map(|position| super::model::PinOrigin {
        x: position.x + 12.0,
        y: position.y + 12.0,
        width: item.content_width,
        height: item.content_height,
    });
    let entry = PinEntry {
        label: label.clone(),
        source: Arc::new(source),
        content_width: item.content_width,
        content_height: item.content_height,
        scale: item.scale,
        opacity: item.opacity,
        locked: item.locked,
        above: item.above,
        workspace_id: Some(item.id),
        workspace_group_id: item.group_id,
        position: None,
        restore_position,
        origin: None,
        device_scale: content_device_scale(app_handle, scale_origin),
        buffer_scale: content_buffer_scale(app_handle, scale_origin),
        sharpen: Arc::new(SharpenSlot::default()),
    };
    state.pin_manager.insert(entry.clone())?;
    super::lifecycle::spawn_sharpen(app_handle, &entry);
    if let Err(error) = create_pin_window(app_handle, &entry) {
        let _ = state.pin_manager.remove(&label);
        return Err(crate::error::report("恢复 Pin 工作区窗口失败", error));
    }
    Ok(())
}

fn presentation_from_entry(
    entry: &PinEntry,
    placement: Option<crate::storage::StoredPinPlacement>,
) -> PinWorkspacePresentation {
    PinWorkspacePresentation {
        scale: entry.scale,
        opacity: entry.opacity,
        locked: entry.locked,
        above: entry.above,
        placement,
    }
}
