use super::manager::{check_budget, Channel, ViewerSession};
use super::model::*;
use super::ViewerManager;
use crate::models::ContentType;
use crate::pin::commands::{PinCanvasProject, PinCanvasSaveMode};
use crate::storage::{BoundedImageData, StorageEngine};
use std::sync::{Arc, Mutex};

#[test]
fn viewer_handle_matches_the_shared_json_fixture() {
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../src/tests/fixtures/ipc-contract/viewer-handle.json"
    ));
    let handle: ViewerHandle = serde_json::from_str(source).unwrap();
    let fixture: serde_json::Value = serde_json::from_str(source).unwrap();
    assert_eq!(serde_json::to_value(handle).unwrap(), fixture);
}

fn png() -> Vec<u8> {
    crate::screenshot::encode_png(&[10, 20, 30, 40, 255, 0, 0, 255], 2, 1).unwrap()
}
fn entry(id: i64) -> Arc<ViewerSession> {
    Arc::new(ViewerSession::new(
        id,
        format!("hash-{id}"),
        false,
        png(),
        (2, 1),
    ))
}
fn request(entry: &ViewerSession, id: u64) -> ViewerRequest {
    ViewerRequest {
        session_id: entry.payload.handle.session_id.clone(),
        snapshot_id: entry.payload.handle.snapshot_id.clone(),
        request_id: id,
    }
}
fn ocr(text: &str) -> crate::ocr::StructuredOcr {
    serde_json::from_value(serde_json::json!({"width":2,"height":1,"text":text,"lines":[],"paragraphs":[],
        "pipeline":{"id":"fixture","engine":"tesseract","featureSchema":null,"layoutExecuted":false,"layoutReason":"fallback"},
        "fallbackReason":"fixture"})).unwrap()
}
#[test]
fn bounded_snapshot_survives_clip_deletion_and_preserves_sensitive_identity() {
    let storage = StorageEngine::new_in_memory().unwrap();
    let bytes = png();
    let clip = storage
        .insert_clip(
            &ContentType::Image,
            None,
            None,
            Some(&bytes),
            "snapshot-test",
            bytes.len() as i64,
            true,
        )
        .unwrap();
    let snapshot = storage
        .get_bounded_image_snapshot(clip.id, bytes.len())
        .unwrap()
        .unwrap();
    assert!(snapshot.is_sensitive);
    assert_eq!(snapshot.content_hash, "snapshot-test");
    assert!(matches!(
        storage
            .get_bounded_image_snapshot(clip.id, 1)
            .unwrap()
            .unwrap()
            .image,
        BoundedImageData::TooLarge
    ));
    let BoundedImageData::Bytes(source) = snapshot.image else {
        panic!("expected bytes")
    };
    let session = ViewerSession::new(
        clip.id,
        snapshot.content_hash,
        snapshot.is_sensitive,
        source,
        (2, 1),
    );
    storage.delete_clip(clip.id).unwrap();
    assert!(storage
        .get_bounded_image_snapshot(clip.id, MAX_PNG_BYTES)
        .unwrap()
        .is_none());
    assert!(!storage.is_hash_sensitive("snapshot-test").unwrap());
    assert_eq!(session.png.as_ref(), &bytes);
    assert_eq!(
        session.protect_sensitive(false).unwrap_err().code,
        "sensitive_content"
    );
    assert_eq!(
        crate::pin::output::decode_source(&session.png)
            .unwrap()
            .get_pixel(0, 0)
            .0,
        [10, 20, 30, 40]
    );
}
#[test]
fn snapshot_query_distinguishes_text_missing_and_over_budget_images() {
    let storage = StorageEngine::new_in_memory().unwrap();
    let text = storage
        .insert_clip(&ContentType::Text, Some("x"), None, None, "text", 1, false)
        .unwrap();
    let missing = storage
        .insert_clip(&ContentType::Image, None, None, None, "missing", 0, false)
        .unwrap();
    assert!(matches!(
        storage
            .get_bounded_image_snapshot(text.id, 1)
            .unwrap()
            .unwrap()
            .image,
        BoundedImageData::NotImage
    ));
    assert!(matches!(
        storage
            .get_bounded_image_snapshot(missing.id, 1)
            .unwrap()
            .unwrap()
            .image,
        BoundedImageData::Missing
    ));
}
#[test]
fn owner_handle_and_protocol_reject_cross_window_or_old_sources() {
    let a = entry(1);
    let b = entry(2);
    assert!(a.authorize(&a.payload.label, &a.payload.handle).is_ok());
    assert!(a.authorize(&b.payload.label, &a.payload.handle).is_err());
    assert!(a.authorize(&a.payload.label, &b.payload.handle).is_err());
    let path = format!("/{}/{}", a.payload.label, a.payload.handle.snapshot_id);
    assert!(super::frame_protocol::parse_path(&a.payload.label, &path, None).is_some());
    assert!(super::frame_protocol::parse_path(&b.payload.label, &path, None).is_none());
    for bad in [
        format!("{path}/extra"),
        format!("/{}/../source", a.payload.label),
        format!("/{}/%2e%2e", a.payload.label),
    ] {
        assert!(super::frame_protocol::parse_path(&a.payload.label, &bad, None).is_none());
    }
    assert!(
        super::frame_protocol::parse_path(&a.payload.label, &path, Some("revision=0")).is_none()
    );
    a.deactivate();
    assert_eq!(a.payload().unwrap_err().code, "closed");
}
#[test]
fn manager_deduplicates_by_clip_and_hash_and_enforces_budgets() {
    let manager = ViewerManager::default();
    let a = entry(1);
    manager.insert(a.clone()).unwrap();
    assert!(Arc::ptr_eq(&manager.find(1, "hash-1").unwrap(), &a));
    assert!(manager.find(1, "other-version").is_none());
    assert!(manager.insert(a.clone()).is_err());
    for id in 2..=4 {
        manager.insert(entry(id)).unwrap();
    }
    assert_eq!(
        manager.insert(entry(5)).unwrap_err().code,
        "image_too_large"
    );
    manager.remove(&a.payload.label);
    assert!(a.payload().is_err());
    manager.insert(entry(5)).unwrap();
    assert!(check_budget(0, MAX_TOTAL_BYTES, 0).is_ok());
    assert!(check_budget(0, MAX_TOTAL_BYTES, 1).is_err());
    assert!(check_budget(0, usize::MAX, 1).is_err());
    assert!(validate_dimensions(16_384, 2_048).is_ok());
    assert!(validate_dimensions(16_384, 2_049).is_err());
    assert!(validate_dimensions(0, 2).is_err());
    assert!(validate_dimensions(16_385, 1).is_err());
}
#[test]
fn latest_results_are_per_tool_and_closed_or_superseded_ocr_never_publishes() {
    let a = entry(1);
    let old = request(&a, 1);
    let new = request(&a, 2);
    a.begin(Channel::Ocr, &old).unwrap();
    a.begin(Channel::Ocr, &new).unwrap();
    assert_eq!(
        a.publish_ocr(Channel::Ocr, &old, ocr("old"))
            .unwrap_err()
            .code,
        "stale_request"
    );
    a.begin(Channel::Scan, &old).unwrap();
    a.publish_ocr(Channel::Ocr, &new, ocr("recognized\r\nA  B"))
        .unwrap();
    assert_eq!(a.ocr().unwrap().text, "recognized\r\nA  B");
    assert!(a.begin(Channel::Ocr, &new).is_err());
    a.deactivate();
    assert_eq!(
        a.publish_ocr(Channel::Ocr, &new, ocr("late"))
            .unwrap_err()
            .code,
        "closed"
    );
    assert!(a.ocr().is_none());
}
#[test]
fn text_copy_only_uses_completed_session_results_and_preserves_exact_text() {
    let a = entry(1);
    let job = request(&a, 1);
    let copy = request(&a, 2);
    a.begin(Channel::Ocr, &job).unwrap();
    a.begin(Channel::Text, &copy).unwrap();
    assert!(a.copy_text(&copy, TextSource::Ocr, 0, |_| Ok(())).is_err());
    a.publish_ocr(Channel::Ocr, &job, ocr("line  1\r\nline 2"))
        .unwrap();
    let text = a
        .copy_text(&copy, TextSource::Ocr, 0, |text| Ok(text.to_owned()))
        .unwrap();
    assert_eq!(text, "line  1\r\nline 2");
    assert!(a.copy_text(&copy, TextSource::Ocr, 1, |_| Ok(())).is_err());
    a.deactivate();
    assert!(a
        .copy_text::<()>(&copy, TextSource::Ocr, 0, |_| panic!(
            "closed copy must not write"
        ))
        .is_err());
}
#[test]
fn closing_serializes_with_final_output_commit_and_cancels_prepared_output() {
    let a = entry(1);
    let job = request(&a, 1);
    a.begin(Channel::Output, &job).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let worker = {
        let a = a.clone();
        let events = events.clone();
        std::thread::spawn(move || {
            a.commit(Channel::Output, &job, || {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                events.lock().unwrap().push("write");
                Ok(())
            })
            .unwrap();
        })
    };
    entered_rx.recv().unwrap();
    let closer = {
        let a = a.clone();
        let events = events.clone();
        std::thread::spawn(move || {
            a.deactivate();
            events.lock().unwrap().push("closed");
        })
    };
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    closer.join().unwrap();
    assert_eq!(*events.lock().unwrap(), vec!["write", "closed"]);
    assert!(a
        .commit::<()>(Channel::Output, &request(&a, 1), || panic!("late output"))
        .is_err());
}
#[test]
fn translations_and_sensitive_state_are_isolated_per_window() {
    let a = entry(1);
    let b = entry(2);
    a.translation.register_request_id(1);
    b.translation.register_request_id(2);
    assert!(a.translation.is_latest(1));
    assert!(b.translation.is_latest(2));
    assert!(a.protect_sensitive(true).is_err());
    assert!(a.protect_sensitive(false).is_err());
    assert!(b.protect_sensitive(false).is_ok());
    a.deactivate();
    assert!(b.translation.is_latest(2));
}
#[test]
fn trusted_outputs_keep_canonical_pixels_and_editable_source_without_pin_entry() {
    let source = png();
    let (rendered, editable) =
        crate::pin::output::prepare_save(&source, None, PinCanvasSaveMode::Editable).unwrap();
    assert_eq!(
        crate::pin::output::decode_source(&rendered)
            .unwrap()
            .into_raw(),
        vec![10, 20, 30, 40, 255, 0, 0, 255]
    );
    assert!(editable
        .windows("clippy-project".len())
        .any(|part| part == b"clippy-project"));
    let (_, flat) =
        crate::pin::output::prepare_save(&editable, None, PinCanvasSaveMode::Flat).unwrap();
    assert!(!flat
        .windows("clippy-project".len())
        .any(|part| part == b"clippy-project"));
    let invalid = PinCanvasProject {
        renderer_version: 2,
        source_width: 3,
        source_height: 1,
        annotations: serde_json::json!([]),
        adjustments: serde_json::json!({"grayscale":false,"brightness":0,"contrast":0,"saturation":0,"cornerRadius":0}),
    };
    assert!(crate::pin::output::render_document(&source, Some(&invalid)).is_err());
    let color = ViewerColor::new(0, 0, [10, 20, 30, 40]);
    assert_eq!(color.hex, "#0A141E");
    assert_eq!(color.rgba[3], 40);
}

#[test]
fn native_event_pump_does_not_wait_for_an_output_that_needs_ui_dispatch() {
    use std::sync::mpsc;
    use std::time::Duration;
    enum UiEvent {
        CloseRequested,
        FrameRequested,
        Destroyed,
        Build(mpsc::Sender<()>),
    }
    let a = entry(1);
    a.mark_ready().unwrap();
    let manager = Arc::new(ViewerManager::default());
    manager.insert(a.clone()).unwrap();
    let job = request(&a, 1);
    a.begin(Channel::Output, &job).unwrap();
    let (ui_tx, ui_rx) = mpsc::channel();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (build_tx, build_rx) = mpsc::channel();
    let (queued_tx, queued_rx) = mpsc::channel();
    let worker = {
        let a = a.clone();
        let ui_tx = ui_tx.clone();
        std::thread::spawn(move || {
            a.commit(Channel::Output, &job, || {
                entered_tx.send(()).unwrap();
                build_rx.recv().unwrap();
                let (ack_tx, ack_rx) = mpsc::channel();
                ui_tx.send(UiEvent::Build(ack_tx)).unwrap();
                queued_tx.send(()).unwrap();
                // 和 Wry builder 一样等 UI 执行；超时使失败用例仍可收敛，不挂测试进程。
                ack_rx
                    .recv_timeout(Duration::from_secs(2))
                    .map_err(|_| ViewerError::new("ui_deadlock"))?;
                Ok(())
            })
        })
    };
    entered_rx.recv().unwrap();
    ui_tx.send(UiEvent::CloseRequested).unwrap();
    ui_tx.send(UiEvent::FrameRequested).unwrap();
    ui_tx.send(UiEvent::Destroyed).unwrap();
    build_tx.send(()).unwrap();
    queued_rx.recv().unwrap();
    let ui = {
        let a = a.clone();
        let manager = manager.clone();
        std::thread::spawn(move || {
            let mut events = Vec::new();
            for _ in 0..4 {
                match ui_rx.recv().unwrap() {
                    UiEvent::CloseRequested => {
                        assert!(manager.is_ready(&a.payload.label));
                        events.push("prevent_close");
                    }
                    UiEvent::FrameRequested => {
                        assert!(manager.get(&a.payload.label).unwrap().is_active());
                        events.push("frame");
                    }
                    UiEvent::Destroyed => {
                        manager.remove(&a.payload.label);
                        events.push("destroyed");
                    }
                    UiEvent::Build(ack) => {
                        let _ = ack.send(());
                        events.push("build");
                    }
                }
            }
            events
        })
    };
    let result = worker.join().unwrap();
    let events = ui.join().unwrap();
    assert_eq!(events, ["prevent_close", "frame", "destroyed", "build"]);
    assert!(
        result.is_ok(),
        "UI event loop waited for output's state lock: {result:?}"
    );
    assert!(!a.is_active());
    assert!(!a.ready());
}

#[test]
fn pin_creation_uncertainty_is_sticky_but_pre_builder_failure_can_retry() {
    use crate::pin::commands::ScreenshotPinCreateError;
    let a = entry(1);
    let first = request(&a, 1);
    a.begin(Channel::Output, &first).unwrap();
    let error = a
        .commit_pin::<()>(&first, || {
            Err(ScreenshotPinCreateError::NotCreated {
                message: "invalid source".into(),
            })
        })
        .unwrap_err();
    assert_eq!(error.code, "pin_creation_failed");
    let retry = request(&a, 2);
    a.begin(Channel::Output, &retry).unwrap();
    assert!(a.commit_pin(&retry, || Ok("created")).is_ok());
    let uncertain = request(&a, 3);
    a.begin(Channel::Output, &uncertain).unwrap();
    assert_eq!(
        a.commit_pin::<()>(&uncertain, || Err(ScreenshotPinCreateError::Uncertain {
            attempted_label: "pin-attempted".into(),
            message: "native builder failed".into(),
        }))
        .unwrap_err()
        .code,
        "pin_creation_uncertain"
    );
    let late_retry = request(&a, 4);
    a.begin(Channel::Output, &late_retry).unwrap();
    assert_eq!(
        a.commit_pin::<()>(&late_retry, || panic!("must not build a duplicate pin"))
            .unwrap_err()
            .code,
        "pin_creation_uncertain"
    );
    assert!(a
        .commit(Channel::Output, &late_retry, || Ok(
            "save remains available"
        ))
        .is_ok());
}

#[test]
fn failed_close_restores_only_a_window_that_has_not_been_destroyed() {
    let a = entry(1);
    a.mark_ready().unwrap();
    a.deactivate();
    a.restore_after_close_failure();
    assert!(a.ready());
    a.deactivate();
    a.invalidate();
    a.restore_after_close_failure();
    assert!(!a.is_active());
    assert!(a.mark_ready().is_err());
}

#[test]
fn closed_sources_remain_budgeted_until_the_last_ocr_png_arc_is_released() {
    let manager = ViewerManager::default();
    let a = entry(1);
    let bytes = a.png.len();
    manager.insert(a.clone()).unwrap();
    let ocr_source = Arc::clone(&a.png);
    manager.remove(&a.payload.label);
    drop(a);
    assert_eq!(
        manager.remaining_source_budget().unwrap(),
        MAX_TOTAL_BYTES - bytes
    );
    // 新窗不能复用尚在后台持有的额度；计费不依赖ViewerSession仍存在。
    let b = entry(2);
    manager.insert(b.clone()).unwrap();
    assert_eq!(
        manager.remaining_source_budget().unwrap(),
        MAX_TOTAL_BYTES - bytes - b.png.len()
    );
    drop(ocr_source);
    assert_eq!(
        manager.remaining_source_budget().unwrap(),
        MAX_TOTAL_BYTES - b.png.len()
    );
    manager.remove(&b.payload.label);
    drop(b);
    assert_eq!(manager.remaining_source_budget().unwrap(), MAX_TOTAL_BYTES);
}

#[test]
fn viewer_custom_ipc_gate_rejects_legacy_data_mutations_and_global_config() {
    for command in [
        "get_clip_image",
        "get_clips",
        "delete_clip",
        "update_config",
        "save_config",
        "get_config",
        "translate_text",
        "pin_clip",
        "open_image_viewer",
        "copy_text",
        "plugin:window|set_fullscreen",
        "plugin:window|start_dragging",
    ] {
        assert!(
            !crate::ipc_access::allowed("image-viewer-one", command),
            "{command}"
        );
        assert!(crate::ipc_access::allowed("main", command));
    }
    for command in [
        "get_viewer_payload",
        "get_viewer_settings",
        "recognize_viewer",
        "close_image_viewer",
        "get_viewer_fullscreen",
        "set_viewer_fullscreen",
        "minimize_image_viewer",
        "start_viewer_drag",
    ] {
        assert!(crate::ipc_access::allowed("image-viewer-one", command));
    }
    let a = entry(1);
    let b = entry(2);
    assert!(a.authorize(&b.payload.label, &a.payload.handle).is_err());
    assert!(ViewerManager::default().get("main").is_err());
}

#[test]
fn owned_window_controls_reject_foreign_closed_and_late_queries() {
    let a = entry(1);
    let b = entry(2);
    for (caller, handle) in [
        ("main", &a.payload.handle),
        (b.payload.label.as_str(), &a.payload.handle),
        (a.payload.label.as_str(), &b.payload.handle),
    ] {
        let result: Result<(), _> = super::window::with_owned(&a, caller, handle, || {
            panic!("拒绝的窗口身份不得触发原生操作")
        });
        assert_eq!(result.unwrap_err().code, "forbidden");
    }
    assert!(
        super::window::with_owned(&a, &a.payload.label, &a.payload.handle, || Ok(true)).unwrap()
    );
    let late = super::window::with_owned(&a, &a.payload.label, &a.payload.handle, || {
        a.invalidate();
        Ok(true)
    });
    assert_eq!(late.unwrap_err().code, "closed");
    let closed: Result<(), _> =
        super::window::with_owned(&a, &a.payload.label, &a.payload.handle, || {
            panic!("排队期间已关闭的窗口不得触发原生操作")
        });
    assert_eq!(closed.unwrap_err().code, "closed");
}

#[test]
fn window_control_authorization_does_not_wait_for_the_output_lock() {
    use std::sync::mpsc;
    use std::time::Duration;
    let a = entry(1);
    let job = request(&a, 1);
    a.begin(Channel::Output, &job).unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker = {
        let a = a.clone();
        std::thread::spawn(move || {
            a.commit(Channel::Output, &job, || {
                entered_tx.send(()).unwrap();
                release_rx
                    .recv_timeout(Duration::from_secs(2))
                    .map_err(|_| ViewerError::new("test_timeout"))
            })
        })
    };
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let (result_tx, result_rx) = mpsc::channel();
    let control = {
        let a = a.clone();
        std::thread::spawn(move || {
            let result =
                super::window::with_owned(&a, &a.payload.label, &a.payload.handle, || Ok(true));
            result_tx.send(result).unwrap();
        })
    };
    let before_release = result_rx.recv_timeout(Duration::from_secs(1));
    // 即使断言失败也先释放worker，避免回归死锁挂住整个测试进程。
    let _ = release_tx.send(());
    worker.join().unwrap().unwrap();
    control.join().unwrap();
    assert!(before_release.expect("窗口控制不应等待输出锁").unwrap());
}

#[test]
fn reopen_restores_ready_viewer_in_order_and_retains_completed_tools() {
    use super::window::{restore_with, RestoreStep};
    let a = entry(1);
    let mut steps = Vec::new();
    restore_with(&a, |step| {
        steps.push(step);
        Ok(())
    })
    .unwrap();
    assert!(steps.is_empty(), "首帧未就绪不得显示空白窗口");
    a.mark_ready().unwrap();
    let job = request(&a, 1);
    a.begin(Channel::Ocr, &job).unwrap();
    a.publish_ocr(Channel::Ocr, &job, ocr("retained OCR"))
        .unwrap();
    let handle = a.payload.handle.clone();
    restore_with(&a, |step| {
        steps.push(step);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        steps,
        [
            RestoreStep::Show,
            RestoreStep::Unminimize,
            RestoreStep::Focus
        ]
    );
    assert_eq!(a.payload.handle, handle);
    assert_eq!(a.ocr().unwrap().text, "retained OCR");
    steps.clear();
    let failed = restore_with(&a, |step| {
        steps.push(step);
        if step == RestoreStep::Unminimize {
            Err(ViewerError::new("window_failed"))
        } else {
            Ok(())
        }
    });
    assert_eq!(failed.unwrap_err().code, "window_failed");
    assert_eq!(steps, [RestoreStep::Show, RestoreStep::Unminimize]);
    steps.clear();
    let closed = restore_with(&a, |step| {
        steps.push(step);
        a.invalidate();
        Ok(())
    });
    assert_eq!(closed.unwrap_err().code, "closed");
    assert_eq!(steps, [RestoreStep::Show]);
}

#[test]
fn viewer_settings_exposes_only_display_fields_and_endpoint_origin() {
    let mut config = crate::models::AppConfig {
        screenshot_save_dir: "/private/output".into(),
        global_shortcut: "PrivateShortcut".into(),
        ..crate::models::AppConfig::default()
    };
    config.translation_services[0].endpoint =
        "https://user:password@example.com:9443/private-token?api_key=secret#hidden".into();
    config.translation_services[0].project = "private-project".into();
    let value = serde_json::to_value(ViewerSettings::from(&config)).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 5);
    assert_eq!(
        value["translation_services"][0]["endpoint"],
        "https://example.com:9443"
    );
    let text = value.to_string();
    for secret in [
        "private",
        "password",
        "api_key",
        "hidden",
        "PrivateShortcut",
        "screenshot_save_dir",
    ] {
        assert!(!text.contains(secret), "leaked {secret}");
    }
}

#[test]
fn translation_rechecks_sensitive_closed_and_superseded_sessions_after_credentials() {
    use crate::translation::commands::guarded_credentials;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    for invalidation in ["sensitive", "closed", "superseded"] {
        let a = entry(1);
        let request = request(&a, 1);
        a.begin(Channel::Translation, &request).unwrap();
        a.translation.register_request_id(1);
        let storage = Arc::new(Mutex::new(StorageEngine::new_in_memory().unwrap()));
        let guard = super::commands::translation_guard(a.clone(), storage.clone(), request.clone());
        guard().unwrap();
        let sends = Arc::new(AtomicUsize::new(0));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = {
            let sends = sends.clone();
            std::thread::spawn(move || {
                guarded_credentials(
                    || {
                        entered_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        Ok(())
                    },
                    Some(&guard),
                    |_| {
                        sends.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    },
                )
            })
        };
        entered_rx.recv().unwrap();
        match invalidation {
            "sensitive" => {
                storage
                    .lock()
                    .unwrap()
                    .insert_clip(
                        &ContentType::Image,
                        None,
                        None,
                        Some(&png()),
                        &a.payload.source.content_hash,
                        a.png.len() as i64,
                        true,
                    )
                    .unwrap();
            }
            "closed" => a.deactivate(),
            _ => {
                let mut newer = request;
                newer.request_id = 2;
                a.begin(Channel::Translation, &newer).unwrap();
                a.translation.register_request_id(2);
            }
        }
        release_tx.send(()).unwrap();
        let error = worker.join().unwrap().unwrap_err();
        assert_eq!(
            error.code(),
            if invalidation == "sensitive" {
                "sensitive_content"
            } else {
                "stale_request"
            }
        );
        assert_eq!(
            sends.load(Ordering::SeqCst),
            0,
            "{invalidation}: credentials completed after authorization changed"
        );
    }
}
