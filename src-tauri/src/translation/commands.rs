use super::content::{
    cache_ocr_text, load_clip_input, prepare_clip_text, ClipTranslationInput, PreparedClipText,
};
use super::secrets;
use super::types::{
    ProviderOptions, ServiceTranslation, TranslationBatch, TranslationError, TranslationProvider,
    TranslationRequest, TranslationResult,
};
use crate::commands::AppState;
use crate::models::{AppConfig, TranslationServiceConfig};
use crate::storage::StorageEngine;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tauri::State;

/// 当前启用的第一个服务。截图选区的浮层只有一张卡的位置，只用主服务。
fn primary_service(config: &AppConfig) -> Result<&TranslationServiceConfig, TranslationError> {
    config
        .enabled_translation_services()
        .into_iter()
        .next()
        .ok_or(TranslationError::NoServiceEnabled)
}

/// 参与本次请求的服务。`providers` 非空时只保留其中的服务，供单服务重试使用。
/// 配置里认不出的 provider 名（例如更新版本写入的服务）跳过而不是让整批失败。
fn selected_services(
    config: &AppConfig,
    providers: &[TranslationProvider],
) -> Result<Vec<(TranslationProvider, TranslationServiceConfig)>, TranslationError> {
    let selected: Vec<(TranslationProvider, TranslationServiceConfig)> = config
        .enabled_translation_services()
        .into_iter()
        .filter_map(
            |service| match TranslationProvider::from_str(&service.provider) {
                Ok(provider) => Some((provider, service.clone())),
                Err(_) => {
                    log::warn!("配置中存在无法识别的翻译服务名，已跳过");
                    None
                }
            },
        )
        .filter(|(provider, _)| providers.is_empty() || providers.contains(provider))
        .collect();
    if selected.is_empty() {
        return Err(TranslationError::NoServiceEnabled);
    }
    Ok(selected)
}

fn parse_providers(providers: Option<Vec<String>>) -> Result<Vec<TranslationProvider>, String> {
    providers
        .unwrap_or_default()
        .iter()
        .map(|provider| TranslationProvider::from_str(provider).map_err(ipc_error))
        .collect()
}

/// 服务配置里的空字符串一律表示「用 provider 默认值」，在此统一折叠成 None。
fn provider_options(service: &TranslationServiceConfig) -> ProviderOptions {
    fn non_empty(value: &str) -> Option<String> {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    }

    ProviderOptions {
        endpoint: service.endpoint.trim().to_string(),
        // web 回退端点不开放给用户配置，始终用 provider 内置默认值。
        web_endpoint: String::new(),
        model: non_empty(&service.model),
        region: non_empty(&service.region),
        project: non_empty(&service.project),
    }
}

/// 调用方给的一次翻译输入。语言与 request-id 为 None 时回落到配置或服务分配，
/// 打包成结构是为了不在命令层传一长串同类型的 `Option<String>`。
#[derive(Clone)]
struct TranslationInputs {
    text: String,
    source_language: Option<String>,
    target_language: Option<String>,
    request_id: Option<u64>,
}

fn request_from_config(
    config: &AppConfig,
    provider: TranslationProvider,
    options: ProviderOptions,
    inputs: TranslationInputs,
    service: &super::service::TranslationService,
) -> TranslationRequest {
    let source = inputs
        .source_language
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| config.translation_source_language.clone());
    let target = inputs
        .target_language
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| config.translation_target_language.clone());
    TranslationRequest::with_options(
        inputs.text,
        source,
        target,
        provider,
        options,
        inputs
            .request_id
            .unwrap_or_else(|| service.next_request_id()),
    )
}

/// 显式触发文本翻译。不会由剪贴板 watcher 自动调用。
/// `providers` 为空表示所有启用的服务，指定单个服务即为该服务的重试。
#[tauri::command]
pub async fn translate_text(
    text: String,
    source_language: Option<String>,
    target_language: Option<String>,
    request_id: Option<u64>,
    providers: Option<Vec<String>>,
    state: State<'_, AppState>,
) -> Result<TranslationBatch, String> {
    let providers = parse_providers(providers)?;
    let request_id = reserve_request_id(&state.translation, request_id);
    translate_configured_batch(
        state.translation.clone(),
        state.config.clone(),
        TranslationInputs {
            text,
            source_language,
            target_language,
            request_id: Some(request_id),
        },
        providers,
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
    providers: Option<Vec<String>>,
    state: State<'_, AppState>,
) -> Result<TranslationBatch, String> {
    let providers = parse_providers(providers)?;
    let request_id = reserve_request_id(&state.translation, request_id);
    let storage = state.storage.clone();
    let input = run_blocking({
        let storage = storage.clone();
        move || load_clip_input(&storage, id)
    })
    .await
    .map_err(ipc_error)?;
    let text = resolve_clip_text(input, storage).await.map_err(ipc_error)?;
    translate_configured_batch(
        state.translation.clone(),
        state.config.clone(),
        TranslationInputs {
            text,
            source_language,
            target_language,
            request_id: Some(request_id),
        },
        providers,
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
    log_translation_error("翻译请求", error);
    message
}

/// 空输入和被新请求取代都是正常交互结果，不该出现在 warn 级日志里。
fn log_translation_error(context: &str, error: TranslationError) {
    let expected = matches!(
        error,
        TranslationError::EmptyInput | TranslationError::StaleRequest { .. }
    );
    if expected {
        crate::error::note(&format!("{context}被跳过"), error);
    } else {
        crate::error::report(&format!("{context}失败"), error);
    }
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
    let inputs = TranslationInputs {
        text,
        source_language,
        target_language,
        request_id: Some(request_id),
    };
    run_blocking(move || translate_with_state(&command_state, inputs)).await
}

fn translate_with_state(
    state: &TranslationCommandState,
    inputs: TranslationInputs,
) -> Result<TranslationResult, TranslationError> {
    let config = state
        .config
        .lock()
        .map_err(|_| TranslationError::Internal)?
        .clone();
    let service_config = primary_service(&config)?;
    let provider = TranslationProvider::from_str(&service_config.provider)?;
    translate_one_service(&state.service, &config, provider, service_config, inputs)
}

/// 单个服务的完整同步流程。凭据读取也在这里，keyring 调用同样是阻塞操作，
/// 放进各自的 spawn_blocking 任务里才不会串行化。
fn translate_one_service(
    service: &super::service::TranslationService,
    config: &AppConfig,
    provider: TranslationProvider,
    service_config: &TranslationServiceConfig,
    inputs: TranslationInputs,
) -> Result<TranslationResult, TranslationError> {
    let options = provider_options(service_config);
    let request = request_from_config(config, provider, options, inputs, service);
    let credentials = secrets::get_credentials(provider)?;
    service.translate(request, credentials)
}

/// 并行执行所有参与服务。任一服务失败只影响它自己的结果卡，
/// 整批仅在输入非法、没有启用服务或请求已被更新的请求取代时才失败。
async fn translate_configured_batch(
    service: Arc<super::service::TranslationService>,
    config: Arc<Mutex<AppConfig>>,
    inputs: TranslationInputs,
    providers: Vec<TranslationProvider>,
) -> Result<TranslationBatch, TranslationError> {
    let snapshot = config
        .lock()
        .map_err(|_| TranslationError::Internal)?
        .clone();
    let selected = selected_services(&snapshot, &providers)?;
    let request_id = inputs
        .request_id
        .unwrap_or_else(|| service.next_request_id());

    let mut tasks = Vec::with_capacity(selected.len());
    for (provider, service_config) in selected {
        let service = service.clone();
        let snapshot = snapshot.clone();
        let mut inputs = inputs.clone();
        inputs.request_id = Some(request_id);
        tasks.push((
            provider,
            tauri::async_runtime::spawn_blocking(move || {
                translate_one_service(&service, &snapshot, provider, &service_config, inputs)
            }),
        ));
    }

    let mut services = Vec::with_capacity(tasks.len());
    for (provider, task) in tasks {
        let result = task.await.map_err(|_| TranslationError::Internal)?;
        if let Err(error) = &result {
            // 失败原因只留在本地日志，前端只拿到稳定错误码。
            log_translation_error(&format!("{} 翻译", provider.as_str()), error.clone());
        }
        services.push(ServiceTranslation::from_result(provider, result));
    }

    // 请求已被更新的请求取代时整批作废，避免旧结果覆盖新结果。
    if !service.is_latest(request_id) {
        return Err(TranslationError::StaleRequest {
            request_id,
            latest_request_id: service.latest_request_id(),
        });
    }

    Ok(TranslationBatch {
        request_id,
        services,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(enabled: &[&str]) -> AppConfig {
        let mut config = AppConfig::default();
        for service in &mut config.translation_services {
            service.enabled = enabled.contains(&service.provider.as_str());
        }
        config
    }

    #[test]
    fn all_enabled_services_participate_in_the_default_selection() {
        let config = config_with(&["libretranslate", "deepl", "youdao"]);
        let selected = selected_services(&config, &[]).unwrap();
        let providers: Vec<&str> = selected
            .iter()
            .map(|(provider, _)| provider.as_str())
            .collect();
        // 顺序跟随配置顺序，结果卡的排列才是稳定的。
        assert_eq!(providers, ["libretranslate", "deepl", "youdao"]);
    }

    #[test]
    fn a_provider_filter_narrows_the_selection_to_one_service() {
        let config = config_with(&["libretranslate", "deepl"]);
        let selected = selected_services(&config, &[TranslationProvider::DeepL]).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].0, TranslationProvider::DeepL);
    }

    #[test]
    fn filtering_a_disabled_service_reports_no_service_enabled() {
        let config = config_with(&["libretranslate"]);
        assert_eq!(
            selected_services(&config, &[TranslationProvider::Bing]),
            Err(TranslationError::NoServiceEnabled)
        );
        assert_eq!(
            selected_services(&config_with(&[]), &[]),
            Err(TranslationError::NoServiceEnabled)
        );
    }

    #[test]
    fn unknown_service_names_are_skipped_instead_of_failing_the_batch() {
        let mut config = config_with(&["libretranslate"]);
        config
            .translation_services
            .push(TranslationServiceConfig::new(
                "service-from-a-newer-version",
                true,
            ));
        let selected = selected_services(&config, &[]).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].0, TranslationProvider::LibreTranslate);
    }

    #[test]
    fn provider_names_from_the_frontend_are_validated() {
        assert_eq!(
            parse_providers(Some(vec!["deepl".to_string(), "microsoft".to_string()])).unwrap(),
            [TranslationProvider::DeepL, TranslationProvider::Bing]
        );
        assert!(parse_providers(None).unwrap().is_empty());
        assert!(parse_providers(Some(vec!["not-a-service".to_string()])).is_err());
    }
}
