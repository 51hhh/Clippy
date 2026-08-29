use super::content::{
    cache_ocr_text, load_clip_input, prepare_clip_text, ClipTranslationInput, PreparedClipText,
};
use super::secrets;
use super::types::{TranslationError, TranslationProvider, TranslationRequest, TranslationResult};
use crate::commands::AppState;
use crate::models::AppConfig;
use crate::storage::StorageEngine;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tauri::State;

fn provider_from_config(config: &AppConfig) -> Result<TranslationProvider, TranslationError> {
    TranslationProvider::from_str(&config.translation_provider)
}

fn request_from_config(
    config: &AppConfig,
    provider: TranslationProvider,
    text: String,
    source_language: Option<String>,
    target_language: Option<String>,
    request_id: Option<u64>,
    service: &super::service::TranslationService,
) -> TranslationRequest {
    let source = source_language
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| config.translation_source_language.clone());
    let target = target_language
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| config.translation_target_language.clone());
    let model =
        (!config.translation_model.trim().is_empty()).then(|| config.translation_model.clone());
    TranslationRequest::new(
        text,
        source,
        target,
        config.translation_endpoint.clone(),
        provider,
        model,
        request_id.unwrap_or_else(|| service.next_request_id()),
    )
}

/// 显式触发文本翻译。不会由剪贴板 watcher 自动调用。
#[tauri::command]
pub async fn translate_text(
    text: String,
    source_language: Option<String>,
    target_language: Option<String>,
    request_id: Option<u64>,
    state: State<'_, AppState>,
) -> Result<TranslationResult, String> {
    let request_id = reserve_request_id(&state.translation, request_id);
    translate_configured_text(
        state.translation.clone(),
        state.config.clone(),
        text,
        source_language,
        target_language,
        request_id,
    )
    .await
    .map_err(ipc_error)
}

/// 根据剪贴板条目翻译文本、HTML 纯文本或图片的本地 OCR 结果。
#[tauri::command]
pub async fn translate_clip(
    id: i64,
    source_language: Option<String>,
    target_language: Option<String>,
    request_id: Option<u64>,
    state: State<'_, AppState>,
) -> Result<TranslationResult, String> {
    let request_id = reserve_request_id(&state.translation, request_id);
    let storage = state.storage.clone();
    let input = run_blocking({
        let storage = storage.clone();
        move || load_clip_input(&storage, id)
    })
    .await
    .map_err(ipc_error)?;
    let text = resolve_clip_text(input, storage).await.map_err(ipc_error)?;
    translate_configured_text(
        state.translation.clone(),
        state.config.clone(),
        text,
        source_language,
        target_language,
        request_id,
    )
    .await
    .map_err(ipc_error)
}

/// 保存指定 provider 的凭据。密钥只进入系统 keyring。
/// `api_secret` 只有双字段服务（有道 appSecret）需要，其余服务传 null。
#[tauri::command]
pub async fn set_translation_api_key(
    provider: String,
    api_key: String,
    api_secret: Option<String>,
) -> Result<(), String> {
    let provider = TranslationProvider::from_str(&provider).map_err(ipc_error)?;
    run_blocking(move || secrets::set_credentials(provider, &api_key, api_secret.as_deref()))
        .await
        .map_err(ipc_error)
}

#[tauri::command]
pub async fn has_translation_api_key(provider: String) -> Result<bool, String> {
    let provider = TranslationProvider::from_str(&provider).map_err(ipc_error)?;
    run_blocking(move || secrets::has_credentials(provider))
        .await
        .map_err(ipc_error)
}

#[tauri::command]
pub async fn delete_translation_api_key(provider: String) -> Result<(), String> {
    let provider = TranslationProvider::from_str(&provider).map_err(ipc_error)?;
    run_blocking(move || secrets::delete_credentials(provider))
        .await
        .map_err(ipc_error)
}

/// 同步 keyring、SQLite、OCR 与 HTTP 操作统一离开异步 IPC 执行线程。
async fn run_blocking<T, F>(task: F) -> Result<T, TranslationError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, TranslationError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|_| TranslationError::Internal)?
}

/// 前端拿到的仍然是不泄漏底层上下文的 `ipc_message()`，完整原因只留在本地日志。
pub(crate) fn ipc_error(error: TranslationError) -> String {
    let message = error.ipc_message();
    // 空输入和被新请求取代都是正常交互结果，不该出现在 warn 级日志里。
    let expected = matches!(
        error,
        TranslationError::EmptyInput | TranslationError::StaleRequest { .. }
    );
    if expected {
        crate::error::note("翻译请求被跳过", error);
    } else {
        crate::error::report("翻译请求失败", error);
    }
    message
}

/// spawn_blocking 闭包只携带最小状态，避免把 Tauri State 引用跨线程移动。
struct TranslationCommandState {
    service: Arc<super::service::TranslationService>,
    config: Arc<Mutex<AppConfig>>,
}

/// 在 OCR 等前置工作开始前登记 request-id，让较新的显式请求可以淘汰旧请求。
pub(crate) fn reserve_request_id(
    service: &super::service::TranslationService,
    request_id: Option<u64>,
) -> u64 {
    service.register_request_id(request_id.unwrap_or_default())
}

/// 使用当前配置、系统 keyring 和 TranslationService 翻译显式文本。
/// 截图、剪贴板和普通文本入口共享此处，避免不同入口形成凭据或 provider 分支。
pub(crate) async fn translate_configured_text(
    service: Arc<super::service::TranslationService>,
    config: Arc<Mutex<AppConfig>>,
    text: String,
    source_language: Option<String>,
    target_language: Option<String>,
    request_id: u64,
) -> Result<TranslationResult, TranslationError> {
    let command_state = TranslationCommandState { service, config };
    run_blocking(move || {
        translate_with_state(
            &command_state,
            text,
            source_language,
            target_language,
            Some(request_id),
        )
    })
    .await
}

fn translate_with_state(
    state: &TranslationCommandState,
    text: String,
    source_language: Option<String>,
    target_language: Option<String>,
    request_id: Option<u64>,
) -> Result<TranslationResult, TranslationError> {
    let config = state
        .config
        .lock()
        .map_err(|_| TranslationError::Internal)?
        .clone();
    let provider = provider_from_config(&config)?;
    let request = request_from_config(
        &config,
        provider,
        text,
        source_language,
        target_language,
        request_id,
        &state.service,
    );
    let credentials = secrets::get_credentials(provider)?;
    state.service.translate(request, credentials)
}

async fn resolve_clip_text(
    input: ClipTranslationInput,
    storage: Arc<Mutex<StorageEngine>>,
) -> Result<String, TranslationError> {
    match prepare_clip_text(input)? {
        PreparedClipText::Ready(text) => Ok(text),
        PreparedClipText::NeedsOcr { clip_id, image } => {
            let text = run_blocking(move || {
                crate::ocr::recognize(&image).map_err(|_| TranslationError::OcrFailed)
            })
            .await?;
            if text.trim().is_empty() {
                return Err(TranslationError::EmptyInput);
            }

            let cached_text = text.clone();
            let cache_result =
                run_blocking(move || cache_ocr_text(&storage, clip_id, &cached_text)).await;
            if cache_result.is_err() {
                log::warn!("翻译 OCR 缓存写入失败");
            }
            Ok(text)
        }
    }
}
