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
        }
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
