//! 录屏结果与恢复库。
//!
//! WebView 只持有会话和产物的不透明身份。真实路径、manifest、哈希校验、文件对话框和删除都留在
//! Rust 边界内，避免结果页变成任意文件读写入口。

use super::manifest::{self, RecordingLibraryItem};
use crate::commands::AppState;
use serde::Serialize;
use std::sync::Arc;
use tauri::{Manager, WebviewWindow};
use tauri_plugin_opener::OpenerExt;

const LIBRARY_LABEL: &str = "recordings";
const LIBRARY_PAGE: &str = "recordings.html";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordingLibraryError {
    code: &'static str,
    message: String,
}

impl std::fmt::Display for RecordingLibraryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl RecordingLibraryError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn forbidden() -> Self {
        Self::new("recording_library_forbidden", "录屏结果库调用窗口无效")
    }

    fn storage(message: impl Into<String>) -> Self {
        Self::new("recording_library_storage_failed", message)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordingLibrarySettings {
    language: String,
    theme: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordingPlaybackLease {
    token: String,
    mime_type: &'static str,
}

fn ensure_library(window: &WebviewWindow) -> Result<(), RecordingLibraryError> {
    if window.label() == LIBRARY_LABEL {
        Ok(())
    } else {
        Err(RecordingLibraryError::forbidden())
    }
}

pub(crate) fn open(app: &tauri::AppHandle) -> Result<(), RecordingLibraryError> {
    if let Some(window) = app.get_webview_window(LIBRARY_LABEL) {
        if window.is_visible().unwrap_or(false) {
            window
                .unminimize()
                .and_then(|_| window.show())
                .and_then(|_| window.set_focus())
                .map_err(|error| {
                    RecordingLibraryError::new("recording_library_window_failed", error.to_string())
                })?;
        }
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(
        app,
        LIBRARY_LABEL,
        tauri::WebviewUrl::App(LIBRARY_PAGE.into()),
    )
    .title("Clippy — Recordings")
    .inner_size(780.0, 620.0)
    .min_inner_size(420.0, 360.0)
    .center()
    .resizable(true)
    .decorations(false)
    .always_on_top(false)
    .skip_taskbar(false)
    .visible(false)
    .build()
    .map_err(|error| {
        RecordingLibraryError::new("recording_library_window_failed", error.to_string())
    })?;
    Ok(())
}

#[tauri::command]
pub(crate) fn open_recording_library(window: WebviewWindow) -> Result<(), RecordingLibraryError> {
    if window.label() != "main" {
        return Err(RecordingLibraryError::forbidden());
    }
    open(window.app_handle())
}

#[tauri::command]
pub(crate) fn get_recording_library_settings(
    window: WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<RecordingLibrarySettings, RecordingLibraryError> {
    ensure_library(&window)?;
    let config = state
        .config
        .lock()
        .map_err(|error| RecordingLibraryError::storage(error.to_string()))?;
    Ok(RecordingLibrarySettings {
        language: config.language.clone(),
        theme: config.theme.clone(),
    })
}

#[tauri::command]
pub(crate) async fn list_recordings(
    window: WebviewWindow,
    app: tauri::AppHandle,
) -> Result<Vec<RecordingLibraryItem>, RecordingLibraryError> {
    ensure_library(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|error| RecordingLibraryError::storage(error.to_string()))?;
        manifest::list_library(&app_data_dir).map_err(RecordingLibraryError::storage)
    })
    .await
    .map_err(|error| RecordingLibraryError::storage(error.to_string()))?
}

#[tauri::command]
pub(crate) fn recording_library_ready(window: WebviewWindow) -> Result<(), RecordingLibraryError> {
    ensure_library(&window)?;
    window
        .show()
        .and_then(|_| window.set_focus())
        .map_err(|error| {
            RecordingLibraryError::new("recording_library_window_failed", error.to_string())
        })
}

#[tauri::command]
pub(crate) fn start_recording_library_drag(
    window: WebviewWindow,
) -> Result<(), RecordingLibraryError> {
    ensure_library(&window)?;
    window.start_dragging().map_err(|error| {
        RecordingLibraryError::new("recording_library_window_failed", error.to_string())
    })
}

#[tauri::command]
pub(crate) fn close_recording_library(
    window: WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<(), RecordingLibraryError> {
    ensure_library(&window)?;
    state
        .recording_media
        .clear()
        .map_err(RecordingLibraryError::storage)?;
    window.destroy().map_err(|error| {
        RecordingLibraryError::new("recording_library_window_failed", error.to_string())
    })
}

#[tauri::command]
pub(crate) async fn prepare_recording_playback(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    session_id: String,
    artifact_id: String,
) -> Result<RecordingPlaybackLease, RecordingLibraryError> {
    ensure_library(&window)?;
    let media = Arc::clone(&state.recording_media);
    let generation = media.generation().map_err(RecordingLibraryError::storage)?;
    tauri::async_runtime::spawn_blocking(move || {
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|error| RecordingLibraryError::storage(error.to_string()))?;
        let artifact = manifest::resolve_library_artifact(&app_data_dir, &session_id, &artifact_id)
            .map_err(RecordingLibraryError::storage)?;
        let lease = media
            .issue(generation, &session_id, &artifact)
            .map_err(RecordingLibraryError::storage)?;
        Ok(RecordingPlaybackLease {
            token: lease.token,
            mime_type: lease.mime_type,
        })
    })
    .await
    .map_err(|error| RecordingLibraryError::storage(error.to_string()))?
}

#[tauri::command]
pub(crate) fn release_recording_playback(
    window: WebviewWindow,
    state: tauri::State<'_, AppState>,
    token: String,
) -> Result<(), RecordingLibraryError> {
    ensure_library(&window)?;
    state
        .recording_media
        .revoke(&token)
        .map_err(RecordingLibraryError::storage)
}

#[tauri::command]
pub(crate) async fn export_recording_artifact(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    session_id: String,
    artifact_id: String,
) -> Result<bool, RecordingLibraryError> {
    ensure_library(&window)?;
    let start = state.default_screenshot_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|error| RecordingLibraryError::storage(error.to_string()))?;
        let artifact = manifest::resolve_library_artifact(&app_data_dir, &session_id, &artifact_id)
            .map_err(RecordingLibraryError::storage)?;
        let extension = std::path::Path::new(&artifact.suggested_file_name)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("webm");
        let Some(destination) = crate::dialogs::choose_recording_export(
            &app,
            &start,
            &artifact.suggested_file_name,
            extension,
        ) else {
            return Ok(false);
        };
        manifest::export_library_artifact(&artifact, &destination)
            .map_err(RecordingLibraryError::storage)?;
        Ok(true)
    })
    .await
    .map_err(|error| RecordingLibraryError::storage(error.to_string()))?
}

#[tauri::command]
pub(crate) async fn reveal_recording_artifact(
    window: WebviewWindow,
    app: tauri::AppHandle,
    session_id: String,
    artifact_id: String,
) -> Result<(), RecordingLibraryError> {
    ensure_library(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|error| RecordingLibraryError::storage(error.to_string()))?;
        let artifact = manifest::resolve_library_artifact(&app_data_dir, &session_id, &artifact_id)
            .map_err(RecordingLibraryError::storage)?;
        manifest::verify_library_artifact(&artifact).map_err(RecordingLibraryError::storage)?;
        app.opener()
            .reveal_item_in_dir(&artifact.path)
            .map_err(|error| {
                RecordingLibraryError::new("recording_library_reveal_failed", error.to_string())
            })
    })
    .await
    .map_err(|error| RecordingLibraryError::storage(error.to_string()))?
}

#[tauri::command]
pub(crate) async fn delete_recording_session(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<(), RecordingLibraryError> {
    ensure_library(&window)?;
    let media = Arc::clone(&state.recording_media);
    tauri::async_runtime::spawn_blocking(move || {
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|error| RecordingLibraryError::storage(error.to_string()))?;
        media
            .revoke_session(&session_id)
            .map_err(RecordingLibraryError::storage)?;
        manifest::delete_library_session(&app_data_dir, &session_id)
            .map_err(RecordingLibraryError::storage)
    })
    .await
    .map_err(|error| RecordingLibraryError::storage(error.to_string()))?
}
