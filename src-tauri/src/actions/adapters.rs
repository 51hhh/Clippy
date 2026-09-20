use super::{ActionError, ActionInput, ActionRuntime, PreparedAction};
use crate::commands::AppState;
use std::future::Future;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(super) enum ActionRunError {
    #[error("{0}")]
    Lifecycle(ActionError),
    #[error("动作与领域适配器不匹配")]
    WrongAction,
    #[error("写入剪贴板失败")]
    ClipboardFailed,
    #[error("图片引用不可用")]
    ImageSourceUnavailable,
    #[error("文字识别失败")]
    OcrFailed,
    #[error("扫码服务忙碌")]
    CodeScanBusy,
    #[error("扫码失败")]
    CodeScanFailed,
    #[error("翻译失败")]
    TranslationFailed(&'static str),
    #[error("保存图片失败")]
    SaveFailed,
    #[error("创建贴图失败")]
    PinFailed,
    #[error("贴图创建结果不确定")]
    PinUncertain,
    #[error("截图启动失败")]
    CaptureFailed(&'static str),
}

impl ActionRunError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Lifecycle(error) => error.code(),
            Self::WrongAction => "action_wrong_adapter",
            Self::ClipboardFailed => "action_clipboard_failed",
            Self::ImageSourceUnavailable => "action_image_source_unavailable",
            Self::OcrFailed => "action_ocr_failed",
            Self::CodeScanBusy => "action_code_scan_busy",
            Self::CodeScanFailed => "action_code_scan_failed",
            Self::TranslationFailed(code) => code,
            Self::SaveFailed => "action_save_failed",
            Self::PinFailed => "action_pin_failed",
            Self::PinUncertain => "action_pin_uncertain",
            Self::CaptureFailed(code) => code,
        }
    }
}

fn capture_action_error_code(error: &crate::capture::CaptureError) -> &'static str {
    use crate::capture::CaptureError;
    match error {
        CaptureError::CaptureModeBusy => "action_capture_busy",
        CaptureError::CaptureModeGenerationExhausted => "action_capture_generation_exhausted",
        CaptureError::SessionBusy => "action_capture_session_busy",
        CaptureError::NoMonitorFrames => "action_capture_no_frames",
        CaptureError::Screenshot(_) => "action_capture_screenshot_failed",
        CaptureError::ThreadPanic(_) => "action_capture_worker_failed",
        CaptureError::OverlayCreate(_) | CaptureError::Window(_) => "action_capture_overlay_failed",
        CaptureError::StateLock(_) => "action_capture_internal",
        _ => "action_capture_failed",
    }
}

fn translation_action_error_code(
    error: &crate::translation::types::TranslationError,
) -> &'static str {
    use crate::translation::types::TranslationError;
    match error {
        TranslationError::EmptyInput => "action_translation_empty_input",
        TranslationError::InputTooLarge => "action_translation_input_too_large",
        TranslationError::SensitiveContent => "action_translation_sensitive_content",
        TranslationError::MissingApiKey => "action_translation_missing_api_key",
        TranslationError::IncompleteCredentials => "action_translation_incomplete_credentials",
        TranslationError::KeyringUnavailable => "action_translation_keyring_unavailable",
        TranslationError::ClipUnavailable => "action_translation_clip_unavailable",
        TranslationError::ImageUnavailable => "action_translation_image_unavailable",
        TranslationError::CaptureUnavailable => "action_translation_capture_unavailable",
        TranslationError::OcrFailed => "action_translation_ocr_failed",
        TranslationError::InvalidEndpoint => "action_translation_invalid_endpoint",
        TranslationError::UnsupportedProvider(_) => "action_translation_unsupported_provider",
        TranslationError::NoServiceEnabled => "action_translation_no_service_enabled",
        TranslationError::Timeout => "action_translation_timeout",
        TranslationError::Network => "action_translation_network",
        TranslationError::HttpStatus { .. } => "action_translation_http_status",
        TranslationError::InvalidCredentials => "action_translation_invalid_credentials",
        TranslationError::RateLimited => "action_translation_rate_limited",
        TranslationError::QuotaExceeded => "action_translation_quota_exceeded",
        TranslationError::ResponseTooLarge => "action_translation_response_too_large",
        TranslationError::InvalidResponse => "action_translation_invalid_response",
        TranslationError::ProviderEndpointBroken => "action_translation_provider_endpoint_broken",
        TranslationError::StaleRequest { .. } => "action_translation_stale_request",
        TranslationError::Internal => "action_translation_internal",
    }
}

/// Viewer 第一阶段只允许引用调用窗口自己持有的不可变快照；Capture、Pin 与 Launcher 在各自
/// 权威 source/version 合同接入前不会由这个适配器猜测或解析外部 ID。
pub(super) async fn ocr_image(
    runtime: &ActionRuntime,
    caller_label: &str,
    prepared: &PreparedAction,
    state: &AppState,
) -> Result<crate::ocr::StructuredOcr, ActionRunError> {
    execute_image_ocr(
        runtime,
        caller_label,
        prepared,
        |source_id, source_version| {
            state
                .viewer_manager
                .resolve_action_snapshot(caller_label, source_id, source_version)
                .map_err(|_| ActionRunError::ImageSourceUnavailable)
        },
        |png| async move {
            crate::ocr::recognize_snapshot_shared(png)
                .await
                .map_err(|_| {
                    // OCR 错误可能包含运行时路径；动作层只暴露稳定、无敏感值的领域码。
                    log::warn!("动作文字识别失败");
                    ActionRunError::OcrFailed
                })
        },
    )
    .await
}

/// Viewer 的扫码动作与 OCR 共享同一个权威快照合同，并复用产品唯一的本地扫码并发预算。
pub(super) async fn scan_image_codes(
    runtime: &ActionRuntime,
    caller_label: &str,
    prepared: &PreparedAction,
    state: &AppState,
) -> Result<crate::code_detection::CodeScanResponse, ActionRunError> {
    execute_image_scan(
        runtime,
        caller_label,
        prepared,
        |source_id, source_version| {
            state
                .viewer_manager
                .resolve_action_snapshot(caller_label, source_id, source_version)
                .map_err(|_| ActionRunError::ImageSourceUnavailable)
        },
        |png| async move {
            crate::commands::scan_snapshot_shared(png)
                .await
                .map_err(|error| {
                    if error == crate::code_detection::CodeScanError::Busy {
                        ActionRunError::CodeScanBusy
                    } else {
                        // 解码器内部错误和图片内容不进入动作错误或日志。
                        log::warn!("动作二维码/条码识别失败: {error}");
                        ActionRunError::CodeScanFailed
                    }
                })
        },
    )
    .await
}

/// 使用独立的领域 request-id 空间翻译动作文本，避免 Launcher、Viewer 和主窗口的并发请求
/// 通过 `TranslationService::latest_request_id` 互相淘汰；provider、配置与 keyring 路径保持唯一。
pub(super) async fn translate_text(
    runtime: &ActionRuntime,
    caller_label: &str,
    prepared: &PreparedAction,
    state: &AppState,
) -> Result<crate::translation::types::TranslationResult, ActionRunError> {
    execute_text_translation(
        runtime,
        caller_label,
        prepared,
        |text, source_language, target_language| {
            let service = Arc::new(crate::translation::TranslationService::new());
            let request_id = service.next_request_id();
            let config = Arc::clone(&state.config);
            async move {
                crate::translation::commands::translate_configured_text(
                    service,
                    config,
                    text,
                    source_language,
                    Some(target_language),
                    request_id,
                )
                .await
                .map_err(|error| {
                    let code = translation_action_error_code(&error);
                    // 仅记录稳定码；正文、译文、provider 响应和凭据都不进入日志。
                    log::warn!("动作翻译失败: {code}");
                    ActionRunError::TranslationFailed(code)
                })
            }
        },
    )
    .await
}

/// 复用托盘、快捷键与主窗口唯一的普通截图入口。动作等待者被丢弃时，运行时仍让
/// 截图 future 完成成功或失败补偿，避免源窗口保持隐藏或模式 gate 永久占用。
pub(super) async fn start_capture(
    runtime: &ActionRuntime,
    caller_label: &str,
    prepared: &PreparedAction,
    app_handle: &tauri::AppHandle,
) -> Result<String, ActionRunError> {
    let app_handle = app_handle.clone();
    execute_capture_start(runtime, caller_label, prepared, move || async move {
        crate::commands::start_capture_overlay(app_handle)
            .await
            .map_err(|error| {
                let code = capture_action_error_code(&error);
                // 平台截图错误可能携带窗口标题或后端细节；动作层只记录稳定码。
                log::warn!("动作截图启动失败: {code}");
                ActionRunError::CaptureFailed(code)
            })
    })
    .await
}

/// Viewer 动作只保存后端签发的不可变扁平快照；带画布文档和可编辑模式的保存仍由 Viewer
/// 专用命令处理。所有权复核与文件写入处于同一个不可取消提交阶段。
pub(super) fn save_image(
    runtime: &ActionRuntime,
    caller_label: &str,
    prepared: &PreparedAction,
    state: &AppState,
) -> Result<String, ActionRunError> {
    execute_image_save(
        runtime,
        caller_label,
        prepared,
        |source_id, source_version| {
            state
                .viewer_manager
                .resolve_action_snapshot(caller_label, source_id, source_version)
                .map_err(|_| ActionRunError::ImageSourceUnavailable)
        },
        |png| {
            crate::image_io::save_png(&png, "clippy-viewer", &state.save_target())
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|_| {
                    // 文件系统错误可能携带用户目录；动作层只记录和返回稳定码。
                    log::warn!("动作图片保存失败");
                    ActionRunError::SaveFailed
                })
        },
    )
}

/// Viewer 动作把调用窗口持有的精确扁平快照交给既有 Pin 领域服务。可编辑工程和当前
/// 未提交画布仍由 Viewer 专用命令处理，因为 `owned_image` 不携带可信文档或保存模式。
pub(super) fn pin_image(
    runtime: &ActionRuntime,
    caller_label: &str,
    prepared: &PreparedAction,
    app_handle: &tauri::AppHandle,
    state: &AppState,
) -> Result<String, ActionRunError> {
    execute_image_pin(
        runtime,
        caller_label,
        prepared,
        |source_id, source_version| {
            state
                .viewer_manager
                .pin_action_snapshot(caller_label, source_id, source_version, |png| {
                    crate::pin::create_screenshot_pin_shared(png, None, app_handle, state)
                })
                .map_err(|error| match error {
                    crate::viewer::ActionPinError::SourceUnavailable => {
                        ActionRunError::ImageSourceUnavailable
                    }
                    crate::viewer::ActionPinError::Failed => {
                        log::warn!("动作贴图创建失败");
                        ActionRunError::PinFailed
                    }
                    crate::viewer::ActionPinError::Uncertain => {
                        // 原生 builder 可能已经创建窗口；调用方必须停止自动重试。
                        log::warn!("动作贴图创建结果不确定，禁止自动重试");
                        ActionRunError::PinUncertain
                    }
                })
        },
    )
}

async fn execute_image_ocr<Resolve, Recognize, RecognizeFuture>(
    runtime: &ActionRuntime,
    caller_label: &str,
    prepared: &PreparedAction,
    resolve: Resolve,
    recognize: Recognize,
) -> Result<crate::ocr::StructuredOcr, ActionRunError>
where
    Resolve: FnOnce(&str, u64) -> Result<Arc<Vec<u8>>, ActionRunError>,
    Recognize: FnOnce(Arc<Vec<u8>>) -> RecognizeFuture,
    RecognizeFuture: Future<Output = Result<crate::ocr::StructuredOcr, ActionRunError>>,
{
    execute_owned_image_action(
        "image.ocr",
        runtime,
        caller_label,
        prepared,
        resolve,
        recognize,
    )
    .await
}

async fn execute_image_scan<Resolve, Scan, ScanFuture>(
    runtime: &ActionRuntime,
    caller_label: &str,
    prepared: &PreparedAction,
    resolve: Resolve,
    scan: Scan,
) -> Result<crate::code_detection::CodeScanResponse, ActionRunError>
where
    Resolve: FnOnce(&str, u64) -> Result<Arc<Vec<u8>>, ActionRunError>,
    Scan: FnOnce(Arc<Vec<u8>>) -> ScanFuture,
    ScanFuture: Future<Output = Result<crate::code_detection::CodeScanResponse, ActionRunError>>,
{
    execute_owned_image_action(
        "image.scan_codes",
        runtime,
        caller_label,
        prepared,
        resolve,
        scan,
    )
    .await
}

async fn execute_owned_image_action<T, Resolve, Run, RunFuture>(
    expected_action: &str,
    runtime: &ActionRuntime,
    caller_label: &str,
    prepared: &PreparedAction,
    resolve: Resolve,
    run: Run,
) -> Result<T, ActionRunError>
where
    Resolve: FnOnce(&str, u64) -> Result<Arc<Vec<u8>>, ActionRunError>,
    Run: FnOnce(Arc<Vec<u8>>) -> RunFuture,
    RunFuture: Future<Output = Result<T, ActionRunError>>,
{
    if prepared.descriptor().id != expected_action {
        return Err(ActionRunError::WrongAction);
    }
    let ActionInput::OwnedImage {
        source_id,
        source_version,
    } = prepared.input()
    else {
        return Err(ActionRunError::WrongAction);
    };
    runtime
        .ensure_current(caller_label, prepared.handle())
        .map_err(ActionRunError::Lifecycle)?;
    let png = match resolve(source_id, *source_version) {
        Ok(png) => png,
        Err(error) => {
            return runtime
                .publish(caller_label, prepared.handle(), || Err(error))
                .map_err(ActionRunError::Lifecycle)?;
        }
    };
    let cancellation = prepared.cancellation();
    let result = tokio::select! {
        biased;
        () = cancellation.cancelled() => Err(ActionRunError::Lifecycle(ActionError::Cancelled)),
        result = run(png) => result,
    };
    runtime
        .publish(caller_label, prepared.handle(), || result)
        .map_err(ActionRunError::Lifecycle)?
}

async fn execute_text_translation<Translate, TranslateFuture>(
    runtime: &ActionRuntime,
    caller_label: &str,
    prepared: &PreparedAction,
    translate: Translate,
) -> Result<crate::translation::types::TranslationResult, ActionRunError>
where
    Translate: FnOnce(String, Option<String>, String) -> TranslateFuture,
    TranslateFuture:
        Future<Output = Result<crate::translation::types::TranslationResult, ActionRunError>>,
{
    if prepared.descriptor().id != "text.translate" {
        return Err(ActionRunError::WrongAction);
    }
    let ActionInput::Translation {
        text,
        source_language,
        target_language,
    } = prepared.input()
    else {
        return Err(ActionRunError::WrongAction);
    };
    runtime
        .ensure_current(caller_label, prepared.handle())
        .map_err(ActionRunError::Lifecycle)?;
    let cancellation = prepared.cancellation();
    let result = tokio::select! {
        biased;
        () = cancellation.cancelled() => Err(ActionRunError::Lifecycle(ActionError::Cancelled)),
        result = translate(text.clone(), source_language.clone(), target_language.clone()) => result,
    };
    runtime
        .publish(caller_label, prepared.handle(), || result)
        .map_err(ActionRunError::Lifecycle)?
}

async fn execute_capture_start<Start, StartFuture>(
    runtime: &ActionRuntime,
    caller_label: &str,
    prepared: &PreparedAction,
    start: Start,
) -> Result<String, ActionRunError>
where
    Start: FnOnce() -> StartFuture + Send + 'static,
    StartFuture: Future<Output = Result<String, ActionRunError>> + Send + 'static,
{
    if prepared.descriptor().id != "capture.start" {
        return Err(ActionRunError::WrongAction);
    }
    if !matches!(prepared.input(), ActionInput::Unit) {
        return Err(ActionRunError::WrongAction);
    }
    runtime
        .commit_noncancellable_async(caller_label, prepared.handle(), start)
        .await
        .map_err(ActionRunError::Lifecycle)?
}

fn execute_image_save<Resolve, Save>(
    runtime: &ActionRuntime,
    caller_label: &str,
    prepared: &PreparedAction,
    resolve: Resolve,
    save: Save,
) -> Result<String, ActionRunError>
where
    Resolve: FnOnce(&str, u64) -> Result<Arc<Vec<u8>>, ActionRunError>,
    Save: FnOnce(Arc<Vec<u8>>) -> Result<String, ActionRunError>,
{
    if prepared.descriptor().id != "image.save" {
        return Err(ActionRunError::WrongAction);
    }
    let ActionInput::OwnedImage {
        source_id,
        source_version,
    } = prepared.input()
    else {
        return Err(ActionRunError::WrongAction);
    };
    runtime
        .commit_noncancellable(caller_label, prepared.handle(), || {
            let png = resolve(source_id, *source_version)?;
            save(png)
        })
        .map_err(ActionRunError::Lifecycle)?
}

fn execute_image_pin<Pin>(
    runtime: &ActionRuntime,
    caller_label: &str,
    prepared: &PreparedAction,
    pin: Pin,
) -> Result<String, ActionRunError>
where
    Pin: FnOnce(&str, u64) -> Result<String, ActionRunError>,
{
    if prepared.descriptor().id != "image.pin" {
        return Err(ActionRunError::WrongAction);
    }
    let ActionInput::OwnedImage {
        source_id,
        source_version,
    } = prepared.input()
    else {
        return Err(ActionRunError::WrongAction);
    };
    runtime
        .commit_noncancellable(caller_label, prepared.handle(), || {
            pin(source_id, *source_version)
        })
        .map_err(ActionRunError::Lifecycle)?
}

/// 复用产品唯一的文本剪贴板路径：同样的 watcher 抑制、平台重试和 wake 语义。
pub(super) fn copy_text(
    runtime: &ActionRuntime,
    caller_label: &str,
    prepared: &PreparedAction,
    state: &AppState,
) -> Result<(), ActionRunError> {
    execute_text_copy(runtime, caller_label, prepared, |text| {
        crate::commands::copy_text_suppressed(text, state).map_err(|error| {
            log::warn!("动作写入剪贴板失败: {error}");
            ActionRunError::ClipboardFailed
        })
    })
}

fn execute_text_copy(
    runtime: &ActionRuntime,
    caller_label: &str,
    prepared: &PreparedAction,
    write: impl FnOnce(&str) -> Result<(), ActionRunError>,
) -> Result<(), ActionRunError> {
    if prepared.descriptor().id != "text.copy" {
        return Err(ActionRunError::WrongAction);
    }
    let ActionInput::Text(text) = prepared.input() else {
        return Err(ActionRunError::WrongAction);
    };
    runtime
        .commit_noncancellable(caller_label, prepared.handle(), || write(text))
        .map_err(ActionRunError::Lifecycle)??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn ocr_result(text: &str) -> crate::ocr::StructuredOcr {
        serde_json::from_value(serde_json::json!({
            "width": 2,
            "height": 1,
            "text": text,
            "lines": [],
            "paragraphs": [],
            "pipeline": {
                "id": "fixture",
                "engine": "tesseract",
                "featureSchema": null,
                "layoutExecuted": false,
                "layoutReason": "unstructured_backend"
            },
            "fallbackReason": "fixture"
        }))
        .unwrap()
    }

    fn scan_result(text: &str) -> crate::code_detection::CodeScanResponse {
        crate::code_detection::CodeScanResponse {
            results: vec![crate::code_detection::CodeScanResult {
                format: "qr_code".to_string(),
                text: text.to_string(),
                points: Vec::new(),
            }],
            limited: false,
        }
    }

    fn translation_result(text: &str) -> crate::translation::types::TranslationResult {
        crate::translation::types::TranslationResult {
            request_id: 1,
            provider: crate::translation::types::TranslationProvider::LibreTranslate,
            translated_text: text.to_string(),
            detected_source_language: Some("en".to_string()),
            target_language: "zh-CN".to_string(),
        }
    }

    struct DropSignal(Arc<AtomicBool>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[tokio::test]
    async fn image_ocr_resolves_the_exact_owned_snapshot_and_retires_the_slot() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "image-viewer-owner",
                "image.ocr",
                "analysis",
                json!({"sourceId": "snapshot-secret", "sourceVersion": 0}),
            )
            .unwrap();
        let result = execute_image_ocr(
            &runtime,
            "image-viewer-owner",
            &prepared,
            |source_id, source_version| {
                assert_eq!(source_id, "snapshot-secret");
                assert_eq!(source_version, 0);
                Ok(Arc::new(vec![1, 2, 3]))
            },
            |png| async move {
                assert_eq!(png.as_slice(), &[1, 2, 3]);
                Ok(ocr_result("recognized  text"))
            },
        )
        .await
        .unwrap();
        assert_eq!(result.text, "recognized  text");
        assert_eq!(
            runtime.ensure_current("image-viewer-owner", prepared.handle()),
            Err(ActionError::Superseded)
        );
    }

    #[tokio::test]
    async fn image_ocr_cancellation_drops_the_domain_waiter_before_publication() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "image-viewer-owner",
                "image.ocr",
                "analysis",
                json!({"sourceId": "snapshot-secret", "sourceVersion": 0}),
            )
            .unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let worker = execute_image_ocr(
            &runtime,
            "image-viewer-owner",
            &prepared,
            |_, _| Ok(Arc::new(vec![1])),
            {
                let entered = Arc::clone(&entered);
                let dropped = Arc::clone(&dropped);
                move |_| async move {
                    let _drop = DropSignal(dropped);
                    entered.notify_one();
                    std::future::pending::<Result<crate::ocr::StructuredOcr, ActionRunError>>()
                        .await
                }
            },
        );
        let cancel = async {
            entered.notified().await;
            runtime
                .cancel("image-viewer-owner", prepared.handle())
                .unwrap();
        };
        let (result, ()) = tokio::join!(worker, cancel);
        assert!(matches!(
            result,
            Err(ActionRunError::Lifecycle(ActionError::Cancelled))
        ));
        assert!(dropped.load(Ordering::Acquire));
        assert_eq!(
            runtime.ensure_current("image-viewer-owner", prepared.handle()),
            Err(ActionError::Superseded)
        );
    }

    #[tokio::test]
    async fn replacing_image_ocr_drops_old_work_and_blocks_its_late_result() {
        let runtime = ActionRuntime::default();
        let old = runtime
            .begin(
                "image-viewer-owner",
                "image.ocr",
                "analysis",
                json!({"sourceId": "snapshot-secret", "sourceVersion": 0}),
            )
            .unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let worker = execute_image_ocr(
            &runtime,
            "image-viewer-owner",
            &old,
            |_, _| Ok(Arc::new(vec![1])),
            {
                let entered = Arc::clone(&entered);
                let dropped = Arc::clone(&dropped);
                move |_| async move {
                    let _drop = DropSignal(dropped);
                    entered.notify_one();
                    std::future::pending::<Result<crate::ocr::StructuredOcr, ActionRunError>>()
                        .await
                }
            },
        );
        let replace = async {
            entered.notified().await;
            runtime
                .begin(
                    "image-viewer-owner",
                    "image.ocr",
                    "analysis",
                    json!({"sourceId": "snapshot-new", "sourceVersion": 0}),
                )
                .unwrap()
        };
        let (result, current) = tokio::join!(worker, replace);
        assert!(matches!(
            result,
            Err(ActionRunError::Lifecycle(ActionError::Superseded))
        ));
        assert!(dropped.load(Ordering::Acquire));
        assert_eq!(
            runtime.ensure_current("image-viewer-owner", current.handle()),
            Ok(())
        );
    }

    #[tokio::test]
    async fn image_ocr_source_and_domain_failures_are_stable_and_redacted() {
        for (source_error, expected) in [
            (true, ActionRunError::ImageSourceUnavailable),
            (false, ActionRunError::OcrFailed),
        ] {
            let runtime = ActionRuntime::default();
            let prepared = runtime
                .begin(
                    "image-viewer-owner",
                    "image.ocr",
                    "analysis",
                    json!({"sourceId": "do-not-log-source", "sourceVersion": 0}),
                )
                .unwrap();
            let result = execute_image_ocr(
                &runtime,
                "image-viewer-owner",
                &prepared,
                |_, _| {
                    if source_error {
                        Err(ActionRunError::ImageSourceUnavailable)
                    } else {
                        Ok(Arc::new(vec![1]))
                    }
                },
                |_| async { Err(ActionRunError::OcrFailed) },
            )
            .await
            .unwrap_err();
            assert_eq!(result, expected);
            assert!(!format!("{result:?}").contains("do-not-log-source"));
            assert_eq!(
                runtime.ensure_current("image-viewer-owner", prepared.handle()),
                Err(ActionError::Superseded)
            );
        }
    }

    #[tokio::test]
    async fn image_scan_resolves_the_exact_owned_snapshot_and_retires_the_slot() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "image-viewer-owner",
                "image.scan_codes",
                "analysis",
                json!({"sourceId": "snapshot-secret", "sourceVersion": 0}),
            )
            .unwrap();
        let result = execute_image_scan(
            &runtime,
            "image-viewer-owner",
            &prepared,
            |source_id, source_version| {
                assert_eq!(source_id, "snapshot-secret");
                assert_eq!(source_version, 0);
                Ok(Arc::new(vec![4, 5, 6]))
            },
            |png| async move {
                assert_eq!(png.as_slice(), &[4, 5, 6]);
                Ok(scan_result("https://example.invalid/code"))
            },
        )
        .await
        .unwrap();
        assert_eq!(result.results[0].text, "https://example.invalid/code");
        assert_eq!(
            runtime.ensure_current("image-viewer-owner", prepared.handle()),
            Err(ActionError::Superseded)
        );
    }

    #[tokio::test]
    async fn image_scan_cancellation_drops_the_domain_waiter_before_publication() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "image-viewer-owner",
                "image.scan_codes",
                "analysis",
                json!({"sourceId": "snapshot-secret", "sourceVersion": 0}),
            )
            .unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let worker = execute_image_scan(
            &runtime,
            "image-viewer-owner",
            &prepared,
            |_, _| Ok(Arc::new(vec![1])),
            {
                let entered = Arc::clone(&entered);
                let dropped = Arc::clone(&dropped);
                move |_| async move {
                    let _drop = DropSignal(dropped);
                    entered.notify_one();
                    std::future::pending::<
                        Result<crate::code_detection::CodeScanResponse, ActionRunError>,
                    >()
                    .await
                }
            },
        );
        let cancel = async {
            entered.notified().await;
            runtime
                .cancel("image-viewer-owner", prepared.handle())
                .unwrap();
        };
        let (result, ()) = tokio::join!(worker, cancel);
        assert!(matches!(
            result,
            Err(ActionRunError::Lifecycle(ActionError::Cancelled))
        ));
        assert!(dropped.load(Ordering::Acquire));
        assert_eq!(
            runtime.ensure_current("image-viewer-owner", prepared.handle()),
            Err(ActionError::Superseded)
        );
    }

    #[tokio::test]
    async fn replacing_image_scan_drops_old_work_and_blocks_its_late_result() {
        let runtime = ActionRuntime::default();
        let old = runtime
            .begin(
                "image-viewer-owner",
                "image.scan_codes",
                "analysis",
                json!({"sourceId": "snapshot-secret", "sourceVersion": 0}),
            )
            .unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let worker = execute_image_scan(
            &runtime,
            "image-viewer-owner",
            &old,
            |_, _| Ok(Arc::new(vec![1])),
            {
                let entered = Arc::clone(&entered);
                let dropped = Arc::clone(&dropped);
                move |_| async move {
                    let _drop = DropSignal(dropped);
                    entered.notify_one();
                    std::future::pending::<
                        Result<crate::code_detection::CodeScanResponse, ActionRunError>,
                    >()
                    .await
                }
            },
        );
        let replace = async {
            entered.notified().await;
            runtime
                .begin(
                    "image-viewer-owner",
                    "image.scan_codes",
                    "analysis",
                    json!({"sourceId": "snapshot-new", "sourceVersion": 0}),
                )
                .unwrap()
        };
        let (result, current) = tokio::join!(worker, replace);
        assert!(matches!(
            result,
            Err(ActionRunError::Lifecycle(ActionError::Superseded))
        ));
        assert!(dropped.load(Ordering::Acquire));
        assert_eq!(
            runtime.ensure_current("image-viewer-owner", current.handle()),
            Ok(())
        );
    }

    #[tokio::test]
    async fn image_scan_source_and_domain_failures_are_stable_and_redacted() {
        for (source_error, domain_error, expected) in [
            (
                true,
                ActionRunError::CodeScanFailed,
                ActionRunError::ImageSourceUnavailable,
            ),
            (
                false,
                ActionRunError::CodeScanBusy,
                ActionRunError::CodeScanBusy,
            ),
            (
                false,
                ActionRunError::CodeScanFailed,
                ActionRunError::CodeScanFailed,
            ),
        ] {
            let runtime = ActionRuntime::default();
            let prepared = runtime
                .begin(
                    "image-viewer-owner",
                    "image.scan_codes",
                    "analysis",
                    json!({"sourceId": "do-not-log-source", "sourceVersion": 0}),
                )
                .unwrap();
            let result = execute_image_scan(
                &runtime,
                "image-viewer-owner",
                &prepared,
                |_, _| {
                    if source_error {
                        Err(ActionRunError::ImageSourceUnavailable)
                    } else {
                        Ok(Arc::new(vec![1]))
                    }
                },
                |_| async move { Err(domain_error) },
            )
            .await
            .unwrap_err();
            assert_eq!(result, expected);
            assert!(!format!("{result:?}").contains("do-not-log-source"));
            assert_eq!(
                runtime.ensure_current("image-viewer-owner", prepared.handle()),
                Err(ActionError::Superseded)
            );
        }
        assert_eq!(ActionRunError::CodeScanBusy.code(), "action_code_scan_busy");
        assert_eq!(
            ActionRunError::CodeScanFailed.code(),
            "action_code_scan_failed"
        );
    }

    #[tokio::test]
    async fn text_translation_uses_exact_validated_input_and_retires_the_slot() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "launcher",
                "text.translate",
                "translate",
                json!({
                    "text": "private  text\nwith spacing",
                    "sourceLanguage": "auto",
                    "targetLanguage": "zh-CN"
                }),
            )
            .unwrap();
        let result = execute_text_translation(
            &runtime,
            "launcher",
            &prepared,
            |text, source_language, target_language| async move {
                assert_eq!(text, "private  text\nwith spacing");
                assert_eq!(source_language.as_deref(), Some("auto"));
                assert_eq!(target_language, "zh-CN");
                Ok(translation_result("翻译结果"))
            },
        )
        .await
        .unwrap();
        assert_eq!(result.translated_text, "翻译结果");
        assert_eq!(
            runtime.ensure_current("launcher", prepared.handle()),
            Err(ActionError::Superseded)
        );
    }

    #[tokio::test]
    async fn text_translation_cancellation_drops_the_domain_waiter_before_publication() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "launcher",
                "text.translate",
                "translate",
                json!({"text": "private", "targetLanguage": "zh-CN"}),
            )
            .unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let worker = execute_text_translation(&runtime, "launcher", &prepared, {
            let entered = Arc::clone(&entered);
            let dropped = Arc::clone(&dropped);
            move |_, _, _| async move {
                let _drop = DropSignal(dropped);
                entered.notify_one();
                std::future::pending::<
                    Result<crate::translation::types::TranslationResult, ActionRunError>,
                >()
                .await
            }
        });
        let cancel = async {
            entered.notified().await;
            runtime.cancel("launcher", prepared.handle()).unwrap();
        };
        let (result, ()) = tokio::join!(worker, cancel);
        assert!(matches!(
            result,
            Err(ActionRunError::Lifecycle(ActionError::Cancelled))
        ));
        assert!(dropped.load(Ordering::Acquire));
        assert_eq!(
            runtime.ensure_current("launcher", prepared.handle()),
            Err(ActionError::Superseded)
        );
    }

    #[tokio::test]
    async fn replacing_text_translation_drops_old_work_and_blocks_its_late_result() {
        let runtime = ActionRuntime::default();
        let old = runtime
            .begin(
                "launcher",
                "text.translate",
                "translate",
                json!({"text": "old private", "targetLanguage": "zh-CN"}),
            )
            .unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let worker = execute_text_translation(&runtime, "launcher", &old, {
            let entered = Arc::clone(&entered);
            let dropped = Arc::clone(&dropped);
            move |_, _, _| async move {
                let _drop = DropSignal(dropped);
                entered.notify_one();
                std::future::pending::<
                    Result<crate::translation::types::TranslationResult, ActionRunError>,
                >()
                .await
            }
        });
        let replace = async {
            entered.notified().await;
            runtime
                .begin(
                    "launcher",
                    "text.translate",
                    "translate",
                    json!({"text": "new private", "targetLanguage": "ja"}),
                )
                .unwrap()
        };
        let (result, current) = tokio::join!(worker, replace);
        assert!(matches!(
            result,
            Err(ActionRunError::Lifecycle(ActionError::Superseded))
        ));
        assert!(dropped.load(Ordering::Acquire));
        assert_eq!(runtime.ensure_current("launcher", current.handle()), Ok(()));
    }

    #[tokio::test]
    async fn text_translation_failure_is_stable_and_redacts_input_and_provider_details() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "launcher",
                "text.translate",
                "translate",
                json!({"text": "do-not-log-text", "targetLanguage": "zh-CN"}),
            )
            .unwrap();
        let result = execute_text_translation(&runtime, "launcher", &prepared, |_, _, _| async {
            Err(ActionRunError::TranslationFailed(
                "action_translation_network",
            ))
        })
        .await
        .unwrap_err();
        assert_eq!(
            result,
            ActionRunError::TranslationFailed("action_translation_network")
        );
        assert_eq!(result.code(), "action_translation_network");
        assert!(!format!("{result:?}").contains("do-not-log-text"));
        assert_eq!(
            runtime.ensure_current("launcher", prepared.handle()),
            Err(ActionError::Superseded)
        );

        let provider_error = crate::translation::types::TranslationError::UnsupportedProvider(
            "do-not-log-provider".to_string(),
        );
        assert_eq!(
            translation_action_error_code(&provider_error),
            "action_translation_unsupported_provider"
        );
    }

    #[tokio::test]
    async fn capture_start_returns_the_domain_session_and_retires_the_slot() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin("launcher", "capture.start", "capture", json!({}))
            .unwrap();
        assert_eq!(
            runtime.cancel("launcher", prepared.handle()),
            Err(ActionError::NotCancellable)
        );
        let session = execute_capture_start(&runtime, "launcher", &prepared, || async {
            Ok("capture-session-exact".to_string())
        })
        .await
        .unwrap();
        assert_eq!(session, "capture-session-exact");
        assert_eq!(
            runtime.ensure_current("launcher", prepared.handle()),
            Err(ActionError::Superseded)
        );
    }

    #[tokio::test]
    async fn capture_start_blocks_replacement_during_domain_startup() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin("launcher", "capture.start", "capture", json!({}))
            .unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let worker = execute_capture_start(&runtime, "launcher", &prepared, {
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            move || async move {
                entered.notify_one();
                release.notified().await;
                Ok("capture-session-exact".to_string())
            }
        });
        let replace = async {
            entered.notified().await;
            assert!(matches!(
                runtime.begin("launcher", "capture.start", "capture", json!({})),
                Err(ActionError::Busy)
            ));
            release.notify_one();
        };
        let (result, ()) = tokio::join!(worker, replace);
        assert_eq!(result.unwrap(), "capture-session-exact");
        assert!(runtime
            .begin("launcher", "capture.start", "capture", json!({}))
            .is_ok());
    }

    #[tokio::test]
    async fn capture_start_failure_is_stable_and_redacts_platform_details() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin("launcher", "capture.start", "capture", json!({}))
            .unwrap();
        let expected = ActionRunError::CaptureFailed("action_capture_screenshot_failed");
        let result = execute_capture_start(&runtime, "launcher", &prepared, move || async move {
            Err(expected)
        })
        .await
        .unwrap_err();
        assert_eq!(result, expected);
        assert_eq!(result.code(), "action_capture_screenshot_failed");

        let platform_error = crate::capture::CaptureError::Screenshot(
            "do-not-log-window-or-backend-detail".to_string(),
        );
        assert_eq!(
            capture_action_error_code(&platform_error),
            "action_capture_screenshot_failed"
        );
        assert!(!format!("{result:?}").contains("do-not-log-window-or-backend-detail"));
        assert_eq!(
            runtime.ensure_current("launcher", prepared.handle()),
            Err(ActionError::Superseded)
        );
    }

    #[test]
    fn image_save_resolves_and_writes_the_exact_snapshot_once() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "image-viewer-owner",
                "image.save",
                "output",
                json!({"sourceId": "snapshot-secret", "sourceVersion": 0}),
            )
            .unwrap();
        assert_eq!(
            runtime.cancel("image-viewer-owner", prepared.handle()),
            Err(ActionError::NotCancellable)
        );
        let result = execute_image_save(
            &runtime,
            "image-viewer-owner",
            &prepared,
            |source_id, source_version| {
                assert_eq!(source_id, "snapshot-secret");
                assert_eq!(source_version, 0);
                Ok(Arc::new(vec![7, 8, 9]))
            },
            |png| {
                assert_eq!(png.as_slice(), &[7, 8, 9]);
                Ok("/private/output/image.png".to_string())
            },
        )
        .unwrap();
        assert_eq!(result, "/private/output/image.png");
        assert_eq!(
            runtime.ensure_current("image-viewer-owner", prepared.handle()),
            Err(ActionError::Superseded)
        );
    }

    #[test]
    fn image_save_source_and_file_failures_are_stable_and_redacted() {
        for (source_error, expected) in [
            (true, ActionRunError::ImageSourceUnavailable),
            (false, ActionRunError::SaveFailed),
        ] {
            let runtime = ActionRuntime::default();
            let prepared = runtime
                .begin(
                    "image-viewer-owner",
                    "image.save",
                    "output",
                    json!({"sourceId": "do-not-log-source", "sourceVersion": 0}),
                )
                .unwrap();
            let save_called = Cell::new(false);
            let result = execute_image_save(
                &runtime,
                "image-viewer-owner",
                &prepared,
                |_, _| {
                    if source_error {
                        Err(ActionRunError::ImageSourceUnavailable)
                    } else {
                        Ok(Arc::new(vec![1]))
                    }
                },
                |_| {
                    save_called.set(true);
                    Err(ActionRunError::SaveFailed)
                },
            )
            .unwrap_err();
            assert_eq!(result, expected);
            assert_eq!(save_called.get(), !source_error);
            assert!(!format!("{result:?}").contains("do-not-log-source"));
            assert_eq!(result.code(), expected.code());
            assert_eq!(
                runtime.ensure_current("image-viewer-owner", prepared.handle()),
                Err(ActionError::Superseded)
            );
        }
    }

    #[test]
    fn image_save_blocks_same_slot_replacement_during_the_file_commit() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "image-viewer-owner",
                "image.save",
                "output",
                json!({"sourceId": "snapshot-secret", "sourceVersion": 0}),
            )
            .unwrap();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let runtime_ref = &runtime;
            let prepared_ref = &prepared;
            let worker = scope.spawn(move || {
                execute_image_save(
                    runtime_ref,
                    "image-viewer-owner",
                    prepared_ref,
                    |_, _| Ok(Arc::new(vec![1])),
                    |_| {
                        entered_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        Ok("/private/output/image.png".to_string())
                    },
                )
            });
            entered_rx.recv().unwrap();
            assert!(matches!(
                runtime.begin(
                    "image-viewer-owner",
                    "image.save",
                    "output",
                    json!({"sourceId": "snapshot-new", "sourceVersion": 0}),
                ),
                Err(ActionError::Busy)
            ));
            release_tx.send(()).unwrap();
            assert_eq!(worker.join().unwrap().unwrap(), "/private/output/image.png");
        });
        assert!(runtime
            .begin(
                "image-viewer-owner",
                "image.save",
                "output",
                json!({"sourceId": "snapshot-new", "sourceVersion": 0}),
            )
            .is_ok());
    }

    #[test]
    fn image_pin_uses_the_exact_owned_snapshot_and_retires_the_slot() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "image-viewer-owner",
                "image.pin",
                "output",
                json!({"sourceId": "snapshot-secret", "sourceVersion": 0}),
            )
            .unwrap();
        assert_eq!(
            runtime.cancel("image-viewer-owner", prepared.handle()),
            Err(ActionError::NotCancellable)
        );
        let label = execute_image_pin(
            &runtime,
            "image-viewer-owner",
            &prepared,
            |source_id, source_version| {
                assert_eq!(source_id, "snapshot-secret");
                assert_eq!(source_version, 0);
                Ok("pin-image-created".to_string())
            },
        )
        .unwrap();
        assert_eq!(label, "pin-image-created");
        assert_eq!(
            runtime.ensure_current("image-viewer-owner", prepared.handle()),
            Err(ActionError::Superseded)
        );
    }

    #[test]
    fn image_pin_failures_are_stable_and_redacted() {
        for expected in [
            ActionRunError::ImageSourceUnavailable,
            ActionRunError::PinFailed,
            ActionRunError::PinUncertain,
        ] {
            let runtime = ActionRuntime::default();
            let prepared = runtime
                .begin(
                    "image-viewer-owner",
                    "image.pin",
                    "output",
                    json!({"sourceId": "do-not-log-source", "sourceVersion": 0}),
                )
                .unwrap();
            let result = execute_image_pin(&runtime, "image-viewer-owner", &prepared, |_, _| {
                Err(expected)
            })
            .unwrap_err();
            assert_eq!(result, expected);
            assert_eq!(result.code(), expected.code());
            assert!(!format!("{result:?}").contains("do-not-log-source"));
            assert_eq!(
                runtime.ensure_current("image-viewer-owner", prepared.handle()),
                Err(ActionError::Superseded)
            );
        }
    }

    #[test]
    fn image_pin_blocks_same_slot_replacement_during_window_creation() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "image-viewer-owner",
                "image.pin",
                "output",
                json!({"sourceId": "snapshot-secret", "sourceVersion": 0}),
            )
            .unwrap();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let runtime_ref = &runtime;
            let prepared_ref = &prepared;
            let worker = scope.spawn(move || {
                execute_image_pin(runtime_ref, "image-viewer-owner", prepared_ref, |_, _| {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok("pin-image-created".to_string())
                })
            });
            entered_rx.recv().unwrap();
            assert!(matches!(
                runtime.begin(
                    "image-viewer-owner",
                    "image.pin",
                    "output",
                    json!({"sourceId": "snapshot-new", "sourceVersion": 0}),
                ),
                Err(ActionError::Busy)
            ));
            release_tx.send(()).unwrap();
            assert_eq!(worker.join().unwrap().unwrap(), "pin-image-created");
        });
        assert!(runtime
            .begin(
                "image-viewer-owner",
                "image.pin",
                "output",
                json!({"sourceId": "snapshot-new", "sourceVersion": 0}),
            )
            .is_ok());
    }

    #[test]
    fn text_copy_uses_the_validated_exact_text_and_retires_the_slot() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "launcher",
                "text.copy",
                "copy",
                json!({"text": "private text\nwith spacing"}),
            )
            .unwrap();
        let writes = Cell::new(0);
        execute_text_copy(&runtime, "launcher", &prepared, |text| {
            assert_eq!(text, "private text\nwith spacing");
            writes.set(writes.get() + 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(writes.get(), 1);
        assert_eq!(
            runtime.ensure_current("launcher", prepared.handle()),
            Err(ActionError::Superseded)
        );
    }

    #[test]
    fn wrong_adapter_and_domain_failure_have_stable_redacted_errors() {
        let runtime = ActionRuntime::default();
        let wrong = runtime
            .begin(
                "launcher",
                "text.translate",
                "translate",
                json!({"text": "secret", "targetLanguage": "zh-CN"}),
            )
            .unwrap();
        assert_eq!(
            execute_text_copy(&runtime, "launcher", &wrong, |_| Ok(())),
            Err(ActionRunError::WrongAction)
        );

        let copy = runtime
            .begin(
                "launcher",
                "text.copy",
                "copy",
                json!({"text": "do-not-log"}),
            )
            .unwrap();
        let error = execute_text_copy(&runtime, "launcher", &copy, |_| {
            Err(ActionRunError::ClipboardFailed)
        })
        .unwrap_err();
        assert_eq!(error.code(), "action_clipboard_failed");
        assert!(!format!("{error:?}").contains("do-not-log"));
        assert_eq!(
            runtime.ensure_current("launcher", copy.handle()),
            Err(ActionError::Superseded)
        );
    }
}
