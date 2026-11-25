use clippy_storage::{
    read_archive, write_archive, ArchiveExportResult, ArchiveScope, ImportSummary,
};
use tauri::{Emitter, Manager};

fn default_archive_name() -> String {
    format!(
        "clippy-{}.clippy.zip",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    )
}

#[tauri::command]
pub async fn export_clippy_archive(
    scope: ArchiveScope,
    include_sensitive: bool,
    app_handle: tauri::AppHandle,
) -> Result<Option<ArchiveExportResult>, String> {
    let start = app_handle
        .state::<crate::commands::AppState>()
        .save_target()
        .directory;
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) =
            crate::dialogs::choose_archive_save(&app_handle, &start, &default_archive_name())
        else {
            return Ok(None);
        };
        let state = app_handle.state::<crate::commands::AppState>();
        let snapshot = state
            .storage
            .lock()
            .map_err(|error| error.to_string())?
            .export_archive_snapshot(scope, include_sensitive)
            .map_err(|error| error.to_string())?;
        write_archive(&path, &snapshot).map(Some)
    })
    .await
    .map_err(|error| format!("归档导出线程异常: {error}"))?
}

#[tauri::command]
pub async fn import_clippy_archive(
    app_handle: tauri::AppHandle,
) -> Result<Option<ImportSummary>, String> {
    let start = app_handle
        .state::<crate::commands::AppState>()
        .save_target()
        .directory;
    let result = tauri::async_runtime::spawn_blocking({
        let app_handle = app_handle.clone();
        move || -> Result<Option<ImportSummary>, String> {
            let Some(path) = crate::dialogs::choose_archive_open(&app_handle, &start) else {
                return Ok(None);
            };
            let snapshot = read_archive(&path)?;
            let state = app_handle.state::<crate::commands::AppState>();
            let imported = {
                let storage = state.storage.lock().map_err(|error| error.to_string())?;
                storage
                    .import_archive_snapshot(&snapshot)
                    .map_err(|error| error.to_string())?
            };
            Ok(Some(imported))
        }
    })
    .await
    .map_err(|error| format!("归档导入线程异常: {error}"))??;
    if result.is_some() {
        if result
            .as_ref()
            .is_some_and(|summary| summary.workspaces_added > 0)
        {
            let restore_app = app_handle.clone();
            match tauri::async_runtime::spawn_blocking(move || {
                let state = restore_app.state::<crate::commands::AppState>();
                crate::pin::restore_saved(&restore_app, &state)
            })
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => log::warn!("导入后恢复 Pin 工作区失败: {error}"),
                Err(error) => log::warn!("导入后恢复 Pin 工作区线程异常: {error}"),
            }
        }
        let _ = app_handle.emit("archive-imported", ());
    }
    Ok(result)
}
