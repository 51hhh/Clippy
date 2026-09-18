use super::manager::{Channel, ViewerSession};
use super::model::*;
use crate::commands::AppState;
use crate::pin::commands::{PinCanvasProject, PinCanvasSaveMode, PinCanvasSaveResult};
use crate::storage::BoundedImageData;
use std::sync::{Arc, OnceLock};
use tauri::{Manager, State, WebviewWindow};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

// 解码/渲染只允许一个工作集；先取许可，再读 BLOB 或克隆冻结 PNG。
fn image_permit() -> Result<OwnedSemaphorePermit, ViewerError> {
    static GATE: OnceLock<Arc<Semaphore>> = OnceLock::new();
    GATE.get_or_init(|| Arc::new(Semaphore::new(1)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| "busy".into())
}
fn session(
    window: &WebviewWindow,
    state: &AppState,
    handle: &ViewerHandle,
) -> Result<Arc<ViewerSession>, ViewerError> {
    let session = state.viewer_manager.get(window.label())?;
    session.authorize(window.label(), handle)?;
    Ok(session)
}
fn start(
    window: &WebviewWindow,
    state: &AppState,
    request: &ViewerRequest,
    channel: Channel,
) -> Result<Arc<ViewerSession>, ViewerError> {
    let session = session(window, state, &request.handle())?;
    session.begin(channel, request)?;
    Ok(session)
}
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ViewerError> + Send + 'static,
) -> Result<T, ViewerError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| ViewerError::new("worker_failed"))?
}
fn map_image_error(_: String) -> ViewerError {
    "image_invalid".into()
}

#[tauri::command]
pub async fn open_image_viewer(
    id: i64,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<ViewerPayload, ViewerError> {
    if window.label() != "main" {
        return Err("forbidden".into());
    }
    // 重复打开只读元数据，不为了聚焦现有窗口再次分配/解码原图。
    {
        let metadata = state
            .storage
            .lock()
            .map_err(|_| ViewerError::new("storage_failed"))?
            .get_bounded_image_snapshot(id, 0)
            .map_err(|_| ViewerError::new("storage_failed"))?;
        if let Some(metadata) = metadata {
            if let Some(existing) = state.viewer_manager.find(id, &metadata.content_hash) {
                if let Some(existing_window) = window
                    .app_handle()
                    .get_webview_window(&existing.payload.label)
                {
                    let _ = existing.protect_sensitive(metadata.is_sensitive);
                    super::window::restore(&existing_window, &existing)?;
                    return existing.payload();
                }
            }
        }
    }
    let _permit = image_permit()?;
    let byte_limit = state
        .viewer_manager
        .remaining_source_budget()?
        .min(MAX_PNG_BYTES);
    let storage = state.storage.clone();
    let entry = blocking(move || {
        let storage = storage
            .lock()
            .map_err(|_| ViewerError::new("storage_failed"))?;
        let snapshot = storage
            .get_bounded_image_snapshot(id, byte_limit)
            .map_err(|_| ViewerError::new("storage_failed"))?
            .ok_or_else(|| ViewerError::new("not_found"))?;
        let revision = match storage.get_image_revision_for_clip(id) {
            Ok(revision) => revision,
            Err(error) => {
                log::warn!("读取查看器内部图片修订失败，按普通图片打开: {error}");
                None
            }
        };
        drop(storage);
        let bytes = match snapshot.image {
            BoundedImageData::Bytes(bytes) => bytes,
            BoundedImageData::NotImage => return Err("not_image".into()),
            BoundedImageData::TooLarge => return Err("image_too_large".into()),
            _ => return Err("source_unavailable".into()),
        };
        let dimensions = crate::screenshot::png_dimensions(&bytes)
            .map_err(|_| ViewerError::new("image_invalid"))?;
        validate_dimensions(dimensions.0, dimensions.1)?;
        // 全量校验在独立worker中完成，PNG解压工作区由Pin共用验证器限定。
        crate::pin::output::decode_source(&bytes).map_err(map_image_error)?;
        let managed = match revision {
            Some(revision) => {
                match crate::pin::output::restore_managed_revision(&bytes, revision) {
                    Ok(managed) => Some(managed),
                    Err(error) => {
                        log::warn!("查看器内部图片修订校验失败，按普通图片打开: {error}");
                        None
                    }
                }
            }
            None => None,
        };
        Ok(Arc::new(ViewerSession::new(
            id,
            snapshot.content_hash,
            snapshot.is_sensitive,
            bytes,
            dimensions,
            managed,
        )))
    })
    .await?;
    let transition = state
        .viewer_transition
        .lock()
        .map_err(|_| ViewerError::new("internal"))?;
    if let Some(existing) = state
        .viewer_manager
        .find(id, &entry.payload.source.content_hash)
    {
        if let Some(existing_window) = window
            .app_handle()
            .get_webview_window(&existing.payload.label)
        {
            // 已定位现有会话，无需再占有创建锁；原生恢复不能跨 transition 等待 UI。
            drop(transition);
            super::window::restore(&existing_window, &existing)?;
            return existing.payload();
        }
        state.viewer_manager.remove(&existing.payload.label);
    }
    state.viewer_manager.insert(entry.clone())?;
    if let Err(error) = super::window::create(window.app_handle(), &entry) {
        state.viewer_manager.remove(&entry.payload.label);
        if let Some(orphan) = window.app_handle().get_webview_window(&entry.payload.label) {
            let _ = orphan.destroy();
        }
        return Err(error);
    }
    entry.payload()
}

#[tauri::command]
pub async fn get_viewer_payload(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<ViewerPayload, ViewerError> {
    let entry = state.viewer_manager.get(window.label())?;
    blocking(move || entry.payload()).await
}
#[tauri::command]
pub async fn get_viewer_settings(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<ViewerSettings, ViewerError> {
    let entry = state.viewer_manager.get(window.label())?;
    if !entry.is_active() {
        return Err("closed".into());
    }
    let config = state
        .config
        .lock()
        .map_err(|_| ViewerError::new("internal"))?;
    Ok(ViewerSettings::from(&*config))
}
#[tauri::command]
pub async fn viewer_ready(
    handle: ViewerHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), ViewerError> {
    let entry = session(&window, &state, &handle)?;
    window
        .show()
        .map_err(|_| ViewerError::new("window_failed"))?;
    entry.mark_ready()?;
    let _ = window.set_focus();
    Ok(())
}

#[tauri::command]
pub async fn get_viewer_fullscreen(
    handle: ViewerHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<bool, ViewerError> {
    let entry = session(&window, &state, &handle)?;
    blocking(move || {
        super::window::with_owned(&entry, window.label(), &handle, || {
            window
                .is_fullscreen()
                .map_err(|_| ViewerError::new("window_failed"))
        })
    })
    .await
}

#[tauri::command]
pub async fn set_viewer_fullscreen(
    handle: ViewerHandle,
    fullscreen: bool,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), ViewerError> {
    let entry = session(&window, &state, &handle)?;
    blocking(move || {
        super::window::with_owned(&entry, window.label(), &handle, || {
            window
                .set_fullscreen(fullscreen)
                .map_err(|_| ViewerError::new("window_failed"))
        })
    })
    .await
}

#[tauri::command]
pub async fn minimize_image_viewer(
    handle: ViewerHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), ViewerError> {
    let entry = session(&window, &state, &handle)?;
    blocking(move || {
        super::window::with_owned(&entry, window.label(), &handle, || {
            window
                .minimize()
                .map_err(|_| ViewerError::new("window_failed"))
        })
    })
    .await
}

#[tauri::command]
pub async fn start_viewer_drag(
    handle: ViewerHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), ViewerError> {
    let entry = session(&window, &state, &handle)?;
    blocking(move || {
        super::window::with_owned(&entry, window.label(), &handle, || {
            window
                .start_dragging()
                .map_err(|_| ViewerError::new("window_failed"))
        })
    })
    .await
}

#[tauri::command]
pub async fn close_image_viewer(
    handle: ViewerHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<(), ViewerError> {
    let entry = session(&window, &state, &handle)?;
    let manager = state.viewer_manager.clone();
    // 原生 destroy 会派发 UI 事件；等待提交的互斥锁只能发生在工作线程。
    blocking(move || {
        entry.deactivate();
        if window.destroy().is_err() {
            entry.restore_after_close_failure();
            return Err("window_failed".into());
        }
        manager.remove(window.label());
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn recognize_viewer(
    request: ViewerRequest,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<ViewerReply<crate::ocr::StructuredOcr>, ViewerError> {
    let entry = start(&window, &state, &request, Channel::Ocr)?;
    let result = crate::ocr::recognize_snapshot_shared(Arc::clone(&entry.png))
        .await
        .map_err(|_| ViewerError::new("ocr_failed"))?;
    entry.publish_ocr(Channel::Ocr, &request, result.clone())?;
    Ok(request.reply(result))
}
#[tauri::command]
pub async fn detect_viewer_codes(
    request: ViewerRequest,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<ViewerReply<crate::code_detection::CodeScanResponse>, ViewerError> {
    let permit = crate::commands::acquire_scan_permit().map_err(|_| ViewerError::new("busy"))?;
    let entry = start(&window, &state, &request, Channel::Scan)?;
    blocking(move || {
        let _permit = permit;
        entry.commit(Channel::Scan, &request, || Ok(()))?;
        let result = crate::code_detection::scan_png(entry.png.as_ref().clone())
            .map_err(|e| ViewerError::new(&e.to_string()))?;
        entry.publish_scan(&request, result.clone())?;
        Ok(request.reply(result))
    })
    .await
}
fn check_sensitive(entry: &ViewerSession, state: &AppState) -> Result<(), ViewerError> {
    let sensitive = state
        .storage
        .lock()
        .map_err(|_| ViewerError::new("storage_failed"))?
        .is_hash_sensitive(&entry.payload.source.content_hash)
        .map_err(|_| ViewerError::new("storage_failed"))?;
    entry.protect_sensitive(sensitive)
}
pub(super) fn translation_guard(
    entry: Arc<ViewerSession>,
    storage: Arc<std::sync::Mutex<crate::storage::StorageEngine>>,
    request: ViewerRequest,
) -> crate::translation::commands::TranslationGuard {
    Arc::new(move || {
        use crate::translation::types::TranslationError;
        let sensitive = storage
            .lock()
            .map_err(|_| TranslationError::Internal)?
            .is_hash_sensitive(&entry.payload.source.content_hash)
            .map_err(|_| TranslationError::Internal)?;
        let check = entry
            .protect_sensitive(sensitive)
            .and_then(|()| entry.commit(Channel::Translation, &request, || Ok(())));
        check.map_err(|error| match error.code.as_str() {
            "sensitive_content" => TranslationError::SensitiveContent,
            "closed" | "stale_request" => TranslationError::StaleRequest {
                request_id: request.request_id,
                latest_request_id: entry.translation.latest_request_id(),
            },
            _ => TranslationError::Internal,
        })
    })
}
#[tauri::command]
pub async fn translate_viewer(
    request: ViewerRequest,
    options: ViewerTranslationOptions,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<ViewerReply<crate::translation::types::TranslationBatch>, ViewerError> {
    let entry = start(&window, &state, &request, Channel::Translation)?;
    check_sensitive(&entry, &state)?;
    entry.translation.register_request_id(request.request_id);
    let ocr = if let Some(result) = entry.ocr() {
        result
    } else {
        let result = crate::ocr::recognize_snapshot_shared(Arc::clone(&entry.png))
            .await
            .map_err(|_| ViewerError::new("ocr_failed"))?;
        entry.publish_ocr(Channel::Translation, &request, result.clone())?;
        result
    };
    // OCR等待期间标敏感/关闭/新翻译请求均要在联网之前再次复核。
    check_sensitive(&entry, &state)?;
    entry.commit(Channel::Translation, &request, || Ok(()))?;
    let result = crate::translation::commands::translate_viewer_text(
        entry.translation.clone(),
        state.config.clone(),
        crate::translation::commands::TranslationInputs {
            text: ocr.text,
            source_language: options.source_language,
            target_language: options.target_language,
            request_id: Some(request.request_id),
        },
        options.providers,
        translation_guard(entry.clone(), state.storage.clone(), request.clone()),
    )
    .await
    .map_err(|error| ViewerError::new(error.code()))?;
    entry.publish_translation(&request, result.clone())?;
    Ok(request.reply(result))
}
#[tauri::command]
pub async fn sample_viewer_color(
    request: ViewerRequest,
    x: u32,
    y: u32,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<ViewerReply<ViewerColor>, ViewerError> {
    let permit = image_permit()?;
    let entry = start(&window, &state, &request, Channel::Color)?;
    if x >= entry.payload.source.width || y >= entry.payload.source.height {
        return Err("invalid_coordinate".into());
    }
    blocking(move || {
        let _permit = permit;
        entry.commit(Channel::Color, &request, || Ok(()))?;
        let image = crate::pin::output::decode_source(&entry.png).map_err(map_image_error)?;
        let color = ViewerColor::new(x, y, image.get_pixel(x, y).0);
        entry.publish_color(&request, color.clone())?;
        Ok(request.reply(color))
    })
    .await
}
#[tauri::command]
pub async fn copy_viewer_image(
    request: ViewerRequest,
    document: Option<PinCanvasProject>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<ViewerReply<()>, ViewerError> {
    let permit = image_permit()?;
    let entry = start(&window, &state, &request, Channel::Output)?;
    let storage = state.storage.clone();
    blocking(move || {
        let _permit = permit;
        entry.commit(Channel::Output, &request, || Ok(()))?;
        let document = entry.effective_project(document.as_ref());
        let png = crate::pin::output::render_document(&entry.source_png, document.as_ref())
            .map_err(map_image_error)?;
        if let Some(document) = document.as_ref() {
            let storage = storage.lock().map_err(|_| ViewerError::new("internal"))?;
            crate::pin::output::register_source_revision(
                &storage,
                &entry.source_png,
                entry.root_clip_id,
                document,
                &png,
            )
            .map_err(map_image_error)?;
        }
        entry.commit(Channel::Output, &request, || {
            crate::image_io::copy_png_to_clipboard(&png)
                .map_err(|_| ViewerError::new("clipboard_failed"))
        })?;
        Ok(request.reply(()))
    })
    .await
}
#[tauri::command]
pub async fn save_viewer_image(
    request: ViewerRequest,
    mode: PinCanvasSaveMode,
    document: Option<PinCanvasProject>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<ViewerReply<PinCanvasSaveResult>, ViewerError> {
    let permit = image_permit()?;
    let entry = start(&window, &state, &request, Channel::Output)?;
    let target = state.save_target();
    let storage = state.storage.clone();
    blocking(move || {
        let _permit = permit;
        entry.commit(Channel::Output, &request, || Ok(()))?;
        let to_clipboard = matches!(mode, PinCanvasSaveMode::Editable);
        let mut document = entry.effective_project(document.as_ref());
        if matches!(mode, PinCanvasSaveMode::Editable) && document.is_none() {
            document = Some(
                crate::pin::output::identity_project(&entry.source_png).map_err(map_image_error)?,
            );
        }
        let (png, disk) =
            crate::pin::output::prepare_save(&entry.source_png, document.as_ref(), mode)
                .map_err(map_image_error)?;
        if matches!(mode, PinCanvasSaveMode::Editable) {
            let document = document
                .as_ref()
                .ok_or_else(|| ViewerError::new("invalid_document"))?;
            let storage = storage.lock().map_err(|_| ViewerError::new("internal"))?;
            crate::pin::output::register_source_revision(
                &storage,
                &entry.source_png,
                entry.root_clip_id,
                document,
                &png,
            )
            .map_err(map_image_error)?;
        }
        let result = entry.commit(Channel::Output, &request, || {
            let path = crate::image_io::save_png(&disk, "clippy-viewer", &target)
                .map_err(|_| ViewerError::new("save_failed"))?;
            let clipboard_error = if to_clipboard {
                crate::image_io::copy_png_to_clipboard(&png)
                    .err()
                    .map(|_| "clipboard_failed".to_string())
            } else {
                None
            };
            Ok(PinCanvasSaveResult {
                path: path.to_string_lossy().into_owned(),
                clipboard_written: to_clipboard && clipboard_error.is_none(),
                clipboard_error,
            })
        })?;
        Ok(request.reply(result))
    })
    .await
}
#[tauri::command]
pub async fn pin_viewer_image(
    request: ViewerRequest,
    document: Option<PinCanvasProject>,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<ViewerReply<String>, ViewerError> {
    let permit = image_permit()?;
    let entry = start(&window, &state, &request, Channel::Output)?;
    let app = window.app_handle().clone();
    blocking(move || {
        let _permit = permit;
        entry.commit(Channel::Output, &request, || Ok(()))?;
        let document = entry.effective_project(document.as_ref());
        let png = crate::pin::output::render_document(&entry.source_png, document.as_ref())
            .map_err(map_image_error)?;
        let label = match document {
            Some(document) => entry.commit_pin(&request, || {
                crate::pin::commands::create_managed_project_pin(
                    entry.source_png.as_ref().clone(),
                    png,
                    document,
                    &app,
                    app.state::<AppState>().inner(),
                )
                .map_err(crate::pin::commands::ScreenshotPinCreateError::not_created)
            })?,
            None => entry.commit_pin(&request, || {
                crate::pin::create_screenshot_pin_shared(
                    Arc::new(png),
                    None,
                    &app,
                    app.state::<AppState>().inner(),
                )
            })?,
        };
        Ok(request.reply(label))
    })
    .await
}
#[tauri::command]
pub async fn copy_viewer_text(
    request: ViewerRequest,
    source: TextSource,
    index: usize,
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<ViewerReply<()>, ViewerError> {
    let entry = start(&window, &state, &request, Channel::Text)?;
    let app = window.app_handle().clone();
    blocking(move || {
        entry.copy_text(&request, source, index, |text| {
            crate::commands::copy_text(text.to_string(), app.state())
                .map_err(|_| ViewerError::new("clipboard_failed"))
        })?;
        Ok(request.reply(()))
    })
    .await
}
