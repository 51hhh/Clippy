//! Tauri command adapters for the Pin domain.
//!
//! Wire names and argument types live here. Window lifecycle, project I/O and
//! output preparation are implemented behind this boundary so they can be
//! exercised without registering a Tauri command.

use super::model::{PinPayload, PinState, PinUpdate};
use crate::commands::AppState;
use tauri::{Manager, State};

pub(crate) use super::lifecycle::{
    create_managed_project_pin, create_screenshot_pin_shared, lower_pins_for_capture,
    raise_focused_pin, restore_pins_after_capture, ScreenshotPinCreateError,
};
pub use super::output::{PinCanvasProject, PinCanvasSaveMode, PinCanvasSaveResult};

#[tauri::command]
pub async fn pin_clip(id: i64, app_handle: tauri::AppHandle) -> Result<String, String> {
    super::lifecycle::pin_clip(id, app_handle).await
}

#[tauri::command]
pub async fn get_pin_toolbar_bounds(
    label: String,
    app_handle: tauri::AppHandle,
) -> Result<super::window::ToolbarBounds, String> {
    super::lifecycle::get_pin_toolbar_bounds(label, app_handle).await
}

#[tauri::command]
pub fn get_pin_payload(label: String, state: State<'_, AppState>) -> Result<PinPayload, String> {
    super::lifecycle::get_pin_payload(label, state)
}

#[tauri::command]
pub async fn pin_ready(label: String, app_handle: tauri::AppHandle) -> Result<(), String> {
    super::lifecycle::pin_ready(label, app_handle).await
}

#[tauri::command]
pub async fn update_pin(
    label: String,
    update: PinUpdate,
    app_handle: tauri::AppHandle,
) -> Result<PinState, String> {
    super::lifecycle::update_pin(label, update, app_handle).await
}

#[tauri::command]
pub fn copy_pin(label: String, state: State<'_, AppState>) -> Result<(), String> {
    super::lifecycle::copy_pin(label, state)
}

#[tauri::command]
pub fn save_pin(label: String, state: State<'_, AppState>) -> Result<String, String> {
    super::lifecycle::save_pin(label, state)
}

#[tauri::command]
pub fn get_pin_source_image(
    label: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    super::lifecycle::get_pin_source_image(label, state)
}

#[tauri::command]
pub async fn save_pin_canvas(
    label: String,
    png_base64: Option<String>,
    to_clipboard: bool,
    mode: PinCanvasSaveMode,
    project: Option<PinCanvasProject>,
    state: State<'_, AppState>,
) -> Result<PinCanvasSaveResult, String> {
    super::lifecycle::save_pin_canvas(label, png_base64, to_clipboard, mode, project, state).await
}

#[tauri::command]
pub async fn copy_pin_canvas(
    label: String,
    png_base64: Option<String>,
    project: Option<PinCanvasProject>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    super::lifecycle::copy_pin_canvas(label, png_base64, project, state).await
}

#[tauri::command]
pub async fn open_pin_project_file(
    path: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    super::lifecycle::open_pin_project_file(path, app_handle, state).await
}

#[tauri::command]
pub async fn close_pin(label: String, app_handle: tauri::AppHandle) -> Result<(), String> {
    super::lifecycle::close_pin(label, app_handle).await
}

#[tauri::command]
pub async fn save_pin_to_workspace(
    label: String,
    group_id: Option<i64>,
    project: Option<PinCanvasProject>,
    app_handle: tauri::AppHandle,
) -> Result<super::workspace::PinWorkspaceStatus, String> {
    run_workspace_work(app_handle, move |app_handle, state| {
        super::workspace::save(&label, group_id, project.as_ref(), app_handle, state)
    })
    .await
}

#[tauri::command]
pub async fn remove_pin_from_workspace(
    label: String,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    run_workspace_work(app_handle, move |_app_handle, state| {
        super::workspace::remove(&label, state)
    })
    .await
}

#[tauri::command]
pub fn list_pin_workspace_groups(
    state: State<'_, AppState>,
) -> Result<Vec<crate::storage::PinWorkspaceGroup>, String> {
    super::workspace::list_groups(&state)
}

#[tauri::command]
pub fn create_pin_workspace_group(
    name: String,
    state: State<'_, AppState>,
) -> Result<crate::storage::PinWorkspaceGroup, String> {
    super::workspace::create_group(&name, &state)
}

#[tauri::command]
pub fn rename_pin_workspace_group(
    id: i64,
    name: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    super::workspace::rename_group(id, &name, &state)
}

#[tauri::command]
pub fn delete_pin_workspace_group(id: i64, state: State<'_, AppState>) -> Result<bool, String> {
    super::workspace::delete_group(id, &state)
}

#[tauri::command]
pub fn assign_pin_workspace_group(
    label: String,
    group_id: Option<i64>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    super::workspace::assign_group(&label, group_id, &state)
}

async fn run_workspace_work<T: Send + 'static>(
    app_handle: tauri::AppHandle,
    work: impl FnOnce(&tauri::AppHandle, &AppState) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle.state::<AppState>();
        work(&app_handle, &state)
    })
    .await
    .map_err(|error| format!("Pin 工作区任务异常: {error}"))?
}
