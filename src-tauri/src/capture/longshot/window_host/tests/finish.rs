use super::*;

#[tokio::test]
async fn finish_copy_success_orders_effects_and_commits_once() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let windows = TestWindows::default();
    windows.add_alive(&label);
    let trace = Arc::new(Mutex::new(Vec::<String>::new()));
    let finish_trace = Arc::clone(&trace);
    let copy_trace = Arc::clone(&trace);
    let png = vec![1, 2, 3, 4, 5];

    let result = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Copy,
        &windows,
        FinishOperations::new(
            move |claimed| {
                assert_eq!(claimed, token);
                finish_trace.lock().expect("trace").push("finish".into());
                std::future::ready(Ok(lifecycle_artifact(png)))
            },
            |_| false,
            move |requested_action, artifact| {
                assert_eq!(requested_action, LongshotOutputAction::Copy);
                assert_eq!(artifact.png.as_slice(), &[1, 2, 3, 4, 5]);
                assert_eq!(artifact.origin, test_origin());
                copy_trace.lock().expect("trace").push("copy".into());
                std::future::ready(Ok(OutputValue::None))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect("finish copy");

    assert_eq!(result.action, LongshotOutputAction::Copy);
    assert_eq!(result.path, None);
    assert_eq!(result.pin_label, None);
    assert_eq!(*trace.lock().expect("trace"), vec!["finish", "copy"]);
    assert_eq!(windows.destroy_count(), 1);
    assert!(registry
        .reserve("capture-overlay-next-7".to_string(), selection())
        .is_ok());
}

#[tokio::test]
async fn finish_copy_failure_retries_same_arc_without_reencoding() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let windows = TestWindows::default();
    windows.add_alive(&label);
    let finishes = AtomicUsize::new(0);
    let first_seen = Arc::new(Mutex::new(None::<Arc<LongshotOutputArtifact>>));
    let first_slot = Arc::clone(&first_seen);

    let first = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Copy,
        &windows,
        FinishOperations::new(
            |_| {
                finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(vec![9, 8, 7])))
            },
            |_| false,
            move |_, artifact| {
                *first_slot.lock().expect("first artifact") = Some(Arc::clone(&artifact));
                std::future::ready(Err(finish::OutputWorkerError::Business(
                    "injected copy failure".to_string(),
                )))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect_err("first copy fails");
    assert_eq!(first.code, "longshot_controller_copy_failed");

    let retained = first_seen
        .lock()
        .expect("first artifact")
        .as_ref()
        .expect("recorded artifact")
        .clone();
    match &*registry.slot.lock().expect("slot") {
        Slot::OutputPending { artifact, .. } => {
            assert!(Arc::ptr_eq(artifact, &retained));
            assert_eq!(artifact.origin, retained.origin);
        }
        other => panic!("expected OutputPending, got {other:?}"),
    }

    let retry_seen = Arc::new(Mutex::new(None::<Arc<LongshotOutputArtifact>>));
    let retry_slot = Arc::clone(&retry_seen);
    execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Copy,
        &windows,
        FinishOperations::new(
            |_| {
                finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(Vec::new())))
            },
            |_| false,
            move |_, artifact| {
                *retry_slot.lock().expect("retry artifact") = Some(Arc::clone(&artifact));
                std::future::ready(Ok(OutputValue::None))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect("retry copy");

    assert_eq!(finishes.load(Ordering::SeqCst), 1);
    let retried = retry_seen
        .lock()
        .expect("retry artifact")
        .as_ref()
        .expect("retry recorded")
        .clone();
    assert!(Arc::ptr_eq(&retained, &retried));
    assert!(Arc::ptr_eq(&retained.png, &retried.png));
    assert_eq!(retained.origin, retried.origin);
    assert_eq!(windows.destroy_count(), 1);
}

#[tokio::test]
async fn finish_save_success_returns_path_without_copying() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let windows = TestWindows::default();
    windows.add_alive(&label);
    let finishes = AtomicUsize::new(0);
    let saves = AtomicUsize::new(0);

    let result = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Save,
        &windows,
        FinishOperations::new(
            |_| {
                finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(vec![8, 6, 7, 5])))
            },
            |_| false,
            |action, artifact| {
                assert_eq!(action, LongshotOutputAction::Save);
                assert_eq!(artifact.png.as_slice(), &[8, 6, 7, 5]);
                saves.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(OutputValue::SavePath("/tmp/截图-完成.png".to_string())))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect("finish save");

    assert_eq!(result.action, LongshotOutputAction::Save);
    assert_eq!(result.path.as_deref(), Some("/tmp/截图-完成.png"));
    assert_eq!(result.pin_label, None);
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
    assert_eq!(saves.load(Ordering::SeqCst), 1);
    assert_eq!(windows.destroy_count(), 1);
    assert!(registry
        .reserve("capture-overlay-next-save".to_string(), selection())
        .is_ok());
}

#[tokio::test]
async fn finish_pin_success_returns_label_and_commits_once() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let windows = TestWindows::default();
    windows.add_alive(&label);
    let finishes = AtomicUsize::new(0);
    let pins = AtomicUsize::new(0);

    let result = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Pin,
        &windows,
        FinishOperations::new(
            |_| {
                finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(vec![3, 5, 8, 9])))
            },
            |_| false,
            |action, artifact| {
                assert_eq!(action, LongshotOutputAction::Pin);
                assert_eq!(artifact.png.as_slice(), &[3, 5, 8, 9]);
                assert_eq!(artifact.origin, test_origin());
                pins.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(OutputValue::PinLabel("pin-image-longshot".to_string())))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect("finish pin");

    assert_eq!(result.action, LongshotOutputAction::Pin);
    assert_eq!(result.path, None);
    assert_eq!(result.pin_label.as_deref(), Some("pin-image-longshot"));
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
    assert_eq!(pins.load(Ordering::SeqCst), 1);
    assert_eq!(windows.destroy_count(), 1);
    assert!(registry
        .reserve("capture-overlay-next-pin".to_string(), selection())
        .is_ok());
}

#[tokio::test]
async fn finish_pin_not_created_retries_the_same_artifact_without_reencoding() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let finishes = AtomicUsize::new(0);
    let retained = Arc::new(Mutex::new(None::<Arc<LongshotOutputArtifact>>));
    let first_seen = Arc::clone(&retained);

    let error = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Pin,
        &TestWindows::default(),
        FinishOperations::new(
            |_| {
                finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(vec![2, 3, 5, 7])))
            },
            |_| false,
            move |action, artifact| {
                assert_eq!(action, LongshotOutputAction::Pin);
                *first_seen.lock().expect("first") = Some(Arc::clone(&artifact));
                std::future::ready(Err(finish::OutputWorkerError::Business(
                    "Pin manager unavailable".to_string(),
                )))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect_err("confirmed not-created Pin fails");
    assert_eq!(error.code, "longshot_controller_pin_failed");

    let original = retained
        .lock()
        .expect("first")
        .as_ref()
        .expect("recorded")
        .clone();
    assert!(matches!(
        &*registry.slot.lock().expect("slot"),
        Slot::OutputPending {
            artifact,
            retry_policy: RetryPolicy::Any,
            ..
        } if Arc::ptr_eq(artifact, &original)
    ));

    let result = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Pin,
        &TestWindows::default(),
        FinishOperations::new(
            |_| {
                finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(Vec::new())))
            },
            |_| false,
            move |action, artifact| {
                assert_eq!(action, LongshotOutputAction::Pin);
                assert!(Arc::ptr_eq(&artifact, &original));
                assert!(Arc::ptr_eq(&artifact.png, &original.png));
                assert_eq!(artifact.origin, original.origin);
                std::future::ready(Ok(OutputValue::PinLabel("pin-image-retry".to_string())))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect("Pin retry");
    assert_eq!(result.pin_label.as_deref(), Some("pin-image-retry"));
    assert_eq!(result.path, None);
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn finish_save_failure_retries_copy_with_same_arc_without_reencoding() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let finishes = AtomicUsize::new(0);
    let retained = Arc::new(Mutex::new(None::<Arc<LongshotOutputArtifact>>));
    let first_artifact = Arc::clone(&retained);

    let error = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Save,
        &TestWindows::default(),
        FinishOperations::new(
            |_| {
                finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(vec![2, 7, 1, 8])))
            },
            |_| false,
            move |action, artifact| {
                assert_eq!(action, LongshotOutputAction::Save);
                *first_artifact.lock().expect("artifact") = Some(Arc::clone(&artifact));
                std::future::ready(Err(finish::OutputWorkerError::Business(
                    "disk full".to_string(),
                )))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect_err("save failure");
    assert_eq!(error.code, "longshot_controller_save_failed");
    let original = retained
        .lock()
        .expect("artifact")
        .as_ref()
        .expect("recorded")
        .clone();
    assert!(matches!(
        &*registry.slot.lock().expect("slot"),
        Slot::OutputPending { artifact, retry_policy: RetryPolicy::Any, .. }
            if Arc::ptr_eq(artifact, &original)
    ));

    let retry_artifact = Arc::new(Mutex::new(None::<Arc<LongshotOutputArtifact>>));
    let retry_seen = Arc::clone(&retry_artifact);
    let result = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Copy,
        &TestWindows::default(),
        FinishOperations::new(
            |_| {
                finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(Vec::new())))
            },
            |_| false,
            move |action, artifact| {
                assert_eq!(action, LongshotOutputAction::Copy);
                *retry_seen.lock().expect("retry") = Some(Arc::clone(&artifact));
                std::future::ready(Ok(OutputValue::None))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect("retry copy");
    assert_eq!(result.action, LongshotOutputAction::Copy);
    assert_eq!(result.path, None);
    assert_eq!(result.pin_label, None);
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
    assert!(Arc::ptr_eq(
        &original,
        retry_artifact
            .lock()
            .expect("retry")
            .as_ref()
            .expect("recorded retry")
    ));
    let retried = retry_artifact
        .lock()
        .expect("retry")
        .as_ref()
        .expect("recorded retry")
        .clone();
    assert!(Arc::ptr_eq(&original.png, &retried.png));
    assert_eq!(original.origin, retried.origin);
}

#[tokio::test]
async fn finish_copy_failure_can_retry_save_with_same_arc() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let finishes = AtomicUsize::new(0);
    let first = Arc::new(Mutex::new(None::<Arc<LongshotOutputArtifact>>));
    let first_seen = Arc::clone(&first);

    execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Copy,
        &TestWindows::default(),
        FinishOperations::new(
            |_| {
                finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(vec![3, 1, 4, 1])))
            },
            |_| false,
            move |_, artifact| {
                *first_seen.lock().expect("first") = Some(Arc::clone(&artifact));
                std::future::ready(Err(finish::OutputWorkerError::Business(
                    "clipboard busy".to_string(),
                )))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect_err("copy failure");
    let original = first
        .lock()
        .expect("first")
        .as_ref()
        .expect("recorded")
        .clone();

    let result = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Save,
        &TestWindows::default(),
        FinishOperations::new(
            |_| {
                finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(Vec::new())))
            },
            |_| false,
            move |action, artifact| {
                assert_eq!(action, LongshotOutputAction::Save);
                assert!(Arc::ptr_eq(&artifact, &original));
                assert!(Arc::ptr_eq(&artifact.png, &original.png));
                assert_eq!(artifact.origin, original.origin);
                std::future::ready(Ok(OutputValue::SavePath("/tmp/recovered.png".to_string())))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect("retry save");
    assert_eq!(result.action, LongshotOutputAction::Save);
    assert_eq!(result.path.as_deref(), Some("/tmp/recovered.png"));
    assert_eq!(result.pin_label, None);
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn finish_business_error_restores_revealed_snapshot_for_retry() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let old_snapshot = start(9).snapshot;
    let result = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Copy,
        &TestWindows::default(),
        FinishOperations::new(
            |_| {
                std::future::ready(Err(FinishWorkerError::Business(
                    CaptureError::LongshotEstimateLowTexture,
                )))
            },
            |_| true,
            |_, _| std::future::ready(Ok(OutputValue::None)),
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect_err("domain error");
    assert_eq!(result.code, "longshot_estimate_low_texture");
    match &*registry.slot.lock().expect("slot") {
        Slot::Active {
            snapshot,
            revealed: true,
            ..
        } => assert_eq!(*snapshot, old_snapshot),
        other => panic!("expected revealed Active, got {other:?}"),
    }
    assert!(registry.claim_append(&label, &handle).is_ok());
}

#[tokio::test]
async fn finish_destroyed_during_encoding_compensates_exact_active_once() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let cancels = AtomicUsize::new(0);
    let result = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Copy,
        &TestWindows::default(),
        FinishOperations::new(
            |_| {
                std::future::ready(Err(FinishWorkerError::Business(
                    CaptureError::LongshotEstimateLowTexture,
                )))
            },
            |_| true,
            |_, _| std::future::ready(Ok(OutputValue::None)),
            |claimed| {
                assert_eq!(claimed, token);
                cancels.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(()))
            },
            |boundary| {
                if boundary == FinishBoundary::AfterClaim {
                    assert_eq!(registry.claim_destroyed(&label), None);
                }
            },
        ),
    )
    .await
    .expect_err("primary preserved");
    assert_eq!(result.code, "longshot_estimate_low_texture");
    assert_eq!(cancels.load(Ordering::SeqCst), 1);
    assert!(registry
        .reserve("capture-overlay-next-7".to_string(), selection())
        .is_ok());
}

#[tokio::test]
async fn finish_destroyed_copy_failure_drops_artifact_without_cancel() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let cancels = AtomicUsize::new(0);
    let result = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Copy,
        &TestWindows::default(),
        FinishOperations::new(
            |_| std::future::ready(Ok(lifecycle_artifact(vec![4, 3, 2, 1]))),
            |_| false,
            |_, _| {
                std::future::ready(Err(finish::OutputWorkerError::Business(
                    "clipboard unavailable".to_string(),
                )))
            },
            |_| {
                cancels.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(()))
            },
            |boundary| {
                if boundary == FinishBoundary::BeforeOutput {
                    assert_eq!(registry.claim_destroyed(&label), None);
                }
            },
        ),
    )
    .await
    .expect_err("copy fails");
    assert_eq!(result.code, "longshot_controller_copy_failed");
    assert_eq!(cancels.load(Ordering::SeqCst), 0);
    assert!(registry
        .reserve("capture-overlay-next-7".to_string(), selection())
        .is_ok());
}

#[tokio::test]
async fn finish_join_failure_is_conservative_cleanup_failed() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let result = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Copy,
        &TestWindows::default(),
        FinishOperations::new(
            |_| std::future::ready(Err(FinishWorkerError::Join("panic".into()))),
            |_| true,
            |_, _| std::future::ready(Ok(OutputValue::None)),
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect_err("join failure");
    assert_eq!(result.code, "longshot_controller_cleanup_failed");
    assert!(matches!(
        registry.claim_ready(&label),
        Ok(ReadyAction::ShowCleanup)
    ));
}

#[test]
fn finish_claim_rejects_invalid_or_busy_states_before_side_effects() {
    let (registry, label, token) = active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    assert_eq!(
        registry
            .claim_finish(&label, &handle, LongshotOutputAction::Copy)
            .expect_err("unrevealed")
            .code,
        "longshot_controller_missing"
    );
    assert!(matches!(
        registry.claim_ready(&label),
        Ok(ReadyAction::ShowActive(_))
    ));
    let stale = LongshotControllerHandle {
        session_id: "longshot".into(),
        generation: "8".into(),
    };
    assert_eq!(
        registry
            .claim_finish(&label, &stale, LongshotOutputAction::Copy)
            .expect_err("stale")
            .code,
        "longshot_controller_superseded"
    );
    assert!(matches!(
        registry.claim_finish(&label, &handle, LongshotOutputAction::Copy),
        Ok(FinishClaim::Encoding(_))
    ));
    assert_eq!(
        registry
            .claim_finish(&label, &handle, LongshotOutputAction::Copy)
            .expect_err("duplicate")
            .code,
        "longshot_controller_busy"
    );
    assert_eq!(
        registry
            .claim_append(&label, &handle)
            .expect_err("append while finishing")
            .code,
        "longshot_controller_busy"
    );
    assert_eq!(
        registry
            .claim_finish(
                "capture-overlay-not-controller",
                &handle,
                LongshotOutputAction::Copy,
            )
            .expect_err("wrong caller")
            .code,
        "longshot_controller_missing"
    );
}

#[test]
fn finish_output_transitions_require_exact_action_and_arc() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    assert!(matches!(
        registry.claim_finish(&label, &handle, LongshotOutputAction::Save),
        Ok(FinishClaim::Encoding(_))
    ));
    let artifact = output_artifact(vec![5, 8, 9, 7]);
    assert_eq!(
        registry
            .publish_outputting(
                &label,
                &token,
                LongshotOutputAction::Copy,
                Arc::clone(&artifact),
            )
            .expect_err("wrong publish action")
            .code,
        "longshot_controller_cleanup_failed"
    );
    registry
        .publish_outputting(
            &label,
            &token,
            LongshotOutputAction::Save,
            Arc::clone(&artifact),
        )
        .expect("exact publish");

    let fake_same_bytes = output_artifact(artifact.png.as_ref().clone());
    assert!(!registry.complete_output_success(
        &label,
        &token,
        LongshotOutputAction::Save,
        &fake_same_bytes,
    ));
    let fake_outer = Arc::new(LongshotOutputArtifact {
        png: Arc::clone(&artifact.png),
        origin: artifact.origin,
    });
    assert!(!registry.complete_output_success(
        &label,
        &token,
        LongshotOutputAction::Save,
        &fake_outer,
    ));
    assert!(!registry.complete_output_success(
        &label,
        &token,
        LongshotOutputAction::Pin,
        &artifact,
    ));

    assert_eq!(
        registry.complete_output_failure(
            &label,
            &token,
            LongshotOutputAction::Copy,
            &artifact,
            false,
        ),
        OutputFailureAction::OwnershipLost
    );
    assert_eq!(
        registry.complete_output_failure(
            &label,
            &token,
            LongshotOutputAction::Save,
            &fake_same_bytes,
            false,
        ),
        OutputFailureAction::OwnershipLost
    );
    assert_eq!(
        registry.complete_output_failure(
            &label,
            &token,
            LongshotOutputAction::Save,
            &artifact,
            false,
        ),
        OutputFailureAction::Pending
    );
}

#[tokio::test]
async fn finish_copy_worker_join_retains_output_pending_for_retry() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let result =
        execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Copy,
            &TestWindows::default(),
            FinishOperations::new(
                |_| std::future::ready(Ok(lifecycle_artifact(vec![6, 5, 4]))),
                |_| false,
                |action, _| async move {
                    run_output_worker(action, || panic!("injected copy panic")).await
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect_err("copy join failure");
    assert_eq!(result.code, "longshot_controller_copy_failed");
    assert!(matches!(
        &*registry.slot.lock().expect("slot"),
        Slot::OutputPending { artifact, .. } if artifact.png.as_slice() == [6, 5, 4]
    ));
}

#[tokio::test]
async fn pin_is_rejected_before_the_blocking_output_worker_runs() {
    let calls = Arc::new(AtomicUsize::new(0));
    let worker_calls = Arc::clone(&calls);
    let error = run_output_worker(LongshotOutputAction::Pin, move || {
        worker_calls.fetch_add(1, Ordering::SeqCst);
        Ok(OutputValue::PinLabel("must-not-run".to_string()))
    })
    .await
    .expect_err("Pin must stay on the Tauri control path");

    assert!(matches!(
        error,
        finish::OutputWorkerError::Business(message)
            if message.contains("Tauri 控制路径")
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn finish_save_join_allows_copy_and_pin_and_policy_never_upgrades() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let finishes = AtomicUsize::new(0);

    let error =
        execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Save,
            &TestWindows::default(),
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(vec![1, 6, 1, 8])))
                },
                |_| false,
                |action, _| async move {
                    run_output_worker(action, || panic!("injected save panic")).await
                },
                |_| std::future::ready(Ok(())),
                |_| {},
            ),
        )
        .await
        .expect_err("save join");
    assert_eq!(error.code, "longshot_controller_save_uncertain");
    assert!(matches!(
        &*registry.slot.lock().expect("slot"),
        Slot::OutputPending {
            retry_policy: RetryPolicy::CopyPin,
            ..
        }
    ));

    let forbidden_finishes = AtomicUsize::new(0);
    let forbidden_outputs = AtomicUsize::new(0);
    let forbidden = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Save,
        &TestWindows::default(),
        FinishOperations::new(
            |_| {
                forbidden_finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(Vec::new())))
            },
            |_| false,
            |_, _| {
                forbidden_outputs.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(OutputValue::SavePath("must-not-save.png".to_string())))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect_err("retry save forbidden");
    assert_eq!(forbidden.code, "longshot_controller_save_uncertain");
    assert_eq!(forbidden_finishes.load(Ordering::SeqCst), 0);
    assert_eq!(forbidden_outputs.load(Ordering::SeqCst), 0);

    let pin_outputs = AtomicUsize::new(0);
    let pin_error = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Pin,
        &TestWindows::default(),
        FinishOperations::new(
            |_| {
                finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(Vec::new())))
            },
            |_| false,
            |action, _| {
                assert_eq!(action, LongshotOutputAction::Pin);
                pin_outputs.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Err(finish::OutputWorkerError::Business(
                    "pin temporarily unavailable".to_string(),
                )))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect_err("Save 不确定后仍可尝试 Pin");
    assert_eq!(pin_error.code, "longshot_controller_pin_failed");
    assert_eq!(pin_outputs.load(Ordering::SeqCst), 1);
    assert_eq!(finishes.load(Ordering::SeqCst), 1);

    let copy_error = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Copy,
        &TestWindows::default(),
        FinishOperations::new(
            |_| {
                finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(Vec::new())))
            },
            |_| false,
            |action, _| {
                assert_eq!(action, LongshotOutputAction::Copy);
                std::future::ready(Err(finish::OutputWorkerError::Business(
                    "clipboard still busy".to_string(),
                )))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect_err("copy retry failure");
    assert_eq!(copy_error.code, "longshot_controller_copy_failed");
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
    assert!(matches!(
        &*registry.slot.lock().expect("slot"),
        Slot::OutputPending {
            retry_policy: RetryPolicy::CopyPin,
            ..
        }
    ));
}

#[tokio::test]
async fn finish_pin_uncertain_forbids_pin_but_keeps_copy_and_save() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let windows = TestWindows::default();
    windows.add_alive(&label);
    let finishes = AtomicUsize::new(0);
    let retained = Arc::new(Mutex::new(None::<Arc<LongshotOutputArtifact>>));
    let first_seen = Arc::clone(&retained);

    let error = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Pin,
        &windows,
        FinishOperations::new(
            |_| {
                finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(vec![1, 4, 1, 4])))
            },
            |_| false,
            move |action, artifact| {
                assert_eq!(action, LongshotOutputAction::Pin);
                *first_seen.lock().expect("first") = Some(Arc::clone(&artifact));
                std::future::ready(Err(finish::OutputWorkerError::Uncertain(
                    "native Pin completion unknown".to_string(),
                )))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect_err("Pin outcome is uncertain");
    assert_eq!(error.code, "longshot_controller_pin_uncertain");

    let original = retained
        .lock()
        .expect("first")
        .as_ref()
        .expect("recorded")
        .clone();
    assert!(matches!(
        &*registry.slot.lock().expect("slot"),
        Slot::OutputPending {
            artifact,
            retry_policy: RetryPolicy::CopySave,
            ..
        } if Arc::ptr_eq(artifact, &original)
    ));

    let forbidden_finishes = AtomicUsize::new(0);
    let forbidden_outputs = AtomicUsize::new(0);
    let forbidden = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Pin,
        &windows,
        FinishOperations::new(
            |_| {
                forbidden_finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(Vec::new())))
            },
            |_| false,
            |_, _| {
                forbidden_outputs.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(OutputValue::PinLabel("must-not-pin".to_string())))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect_err("uncertain Pin retry must be rejected before side effects");
    assert_eq!(forbidden.code, "longshot_controller_pin_uncertain");
    assert_eq!(forbidden_finishes.load(Ordering::SeqCst), 0);
    assert_eq!(forbidden_outputs.load(Ordering::SeqCst), 0);

    let save_original = Arc::clone(&original);
    let saves = AtomicUsize::new(0);
    let save_error = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Save,
        &windows,
        FinishOperations::new(
            |_| {
                finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(Vec::new())))
            },
            |_| false,
            |action, artifact| {
                assert_eq!(action, LongshotOutputAction::Save);
                assert!(Arc::ptr_eq(&artifact, &save_original));
                saves.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Err(finish::OutputWorkerError::Business(
                    "disk temporarily unavailable".to_string(),
                )))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect_err("Save remains allowed");
    assert_eq!(save_error.code, "longshot_controller_save_failed");
    assert_eq!(saves.load(Ordering::SeqCst), 1);

    let result = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Copy,
        &windows,
        FinishOperations::new(
            |_| {
                finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(Vec::new())))
            },
            |_| false,
            move |action, artifact| {
                assert_eq!(action, LongshotOutputAction::Copy);
                assert!(Arc::ptr_eq(&artifact, &original));
                std::future::ready(Ok(OutputValue::None))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect("Copy remains allowed");
    assert_eq!(result.action, LongshotOutputAction::Copy);
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
    assert_eq!(windows.destroy_count(), 1);
}

#[tokio::test]
async fn save_and_pin_uncertainty_compose_to_copy_only_without_reencoding() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let finishes = AtomicUsize::new(0);
    let retained = Arc::new(Mutex::new(None::<Arc<LongshotOutputArtifact>>));
    let first_seen = Arc::clone(&retained);

    let save_error = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Save,
        &TestWindows::default(),
        FinishOperations::new(
            |_| {
                finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(vec![2, 7, 1, 8])))
            },
            |_| false,
            move |_, artifact| {
                *first_seen.lock().expect("first") = Some(Arc::clone(&artifact));
                std::future::ready(Err(finish::OutputWorkerError::Join(
                    "save completion unknown".to_string(),
                )))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect_err("Save uncertain");
    assert_eq!(save_error.code, "longshot_controller_save_uncertain");
    let original = retained
        .lock()
        .expect("first")
        .as_ref()
        .expect("recorded")
        .clone();

    let pin_original = Arc::clone(&original);
    let pin_error = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Pin,
        &TestWindows::default(),
        FinishOperations::new(
            |_| {
                finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(Vec::new())))
            },
            |_| false,
            move |_, artifact| {
                assert!(Arc::ptr_eq(&artifact, &pin_original));
                std::future::ready(Err(finish::OutputWorkerError::Uncertain(
                    "Pin completion unknown".to_string(),
                )))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect_err("Pin uncertain");
    assert_eq!(pin_error.code, "longshot_controller_pin_uncertain");
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
    assert!(matches!(
        &*registry.slot.lock().expect("slot"),
        Slot::OutputPending {
            artifact,
            retry_policy: RetryPolicy::CopyOnly,
            ..
        } if Arc::ptr_eq(artifact, &original)
    ));

    assert_eq!(
        registry
            .claim_finish(&label, &handle, LongshotOutputAction::Save)
            .expect_err("Save remains forbidden")
            .code,
        "longshot_controller_save_uncertain"
    );
    assert_eq!(
        registry
            .claim_finish(&label, &handle, LongshotOutputAction::Pin)
            .expect_err("Pin remains forbidden")
            .code,
        "longshot_controller_pin_uncertain"
    );

    let result = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Copy,
        &TestWindows::default(),
        FinishOperations::new(
            |_| {
                finishes.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(lifecycle_artifact(Vec::new())))
            },
            |_| false,
            move |action, artifact| {
                assert_eq!(action, LongshotOutputAction::Copy);
                assert!(Arc::ptr_eq(&artifact, &original));
                std::future::ready(Ok(OutputValue::None))
            },
            |_| std::future::ready(Ok(())),
            |_| {},
        ),
    )
    .await
    .expect("Copy remains allowed");
    assert_eq!(result.action, LongshotOutputAction::Copy);
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn finish_destroyed_save_uncertain_drops_artifact() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let error = execute_finish_with_ops(
        &registry,
        &label,
        &handle,
        LongshotOutputAction::Save,
        &TestWindows::default(),
        FinishOperations::new(
            |_| std::future::ready(Ok(lifecycle_artifact(vec![2, 0, 2, 6]))),
            |_| false,
            |_, _| {
                std::future::ready(Err(finish::OutputWorkerError::Join(
                    "save completion unknown".to_string(),
                )))
            },
            |_| std::future::ready(Ok(())),
            |boundary| {
                if boundary == FinishBoundary::BeforeOutput {
                    assert_eq!(registry.claim_destroyed(&label), None);
                }
            },
        ),
    )
    .await
    .expect_err("destroyed save remains uncertain");
    assert_eq!(error.code, "longshot_controller_save_uncertain");
    assert!(registry
        .reserve("capture-overlay-after-save".to_string(), selection())
        .is_ok());
}

#[test]
fn finish_output_pending_discard_drops_artifact_without_lifecycle_cancel() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let artifact = output_artifact(vec![3, 1, 4]);
    assert!(matches!(
        registry.claim_finish(&label, &handle, LongshotOutputAction::Copy),
        Ok(FinishClaim::Encoding(_))
    ));
    registry
        .publish_outputting(
            &label,
            &token,
            LongshotOutputAction::Copy,
            Arc::clone(&artifact),
        )
        .expect("publish outputting");
    assert_eq!(
        registry.complete_output_failure(
            &label,
            &token,
            LongshotOutputAction::Copy,
            &artifact,
            false,
        ),
        OutputFailureAction::Pending
    );
    assert_eq!(
        registry
            .claim_append(&label, &handle)
            .expect_err("pending blocks append")
            .code,
        "longshot_controller_busy"
    );
    assert!(matches!(
        registry.claim_cancel(&label, Some(&handle)),
        Ok(CancelAction::Close)
    ));
    assert!(registry
        .reserve("capture-overlay-next-7".to_string(), selection())
        .is_ok());
}

#[tokio::test]
async fn finish_destroyed_at_every_success_boundary_never_cancels_or_resurrects() {
    for injected in [
        FinishBoundary::AfterClaim,
        FinishBoundary::AfterFinish,
        FinishBoundary::BeforeOutput,
        FinishBoundary::AfterOutput,
        FinishBoundary::BeforeCommit,
    ] {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let finishes = AtomicUsize::new(0);
        let copies = AtomicUsize::new(0);
        let cancels = AtomicUsize::new(0);
        let result = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Copy,
            &TestWindows::default(),
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(vec![1, 9, 9, 8])))
                },
                |_| false,
                |_, _| {
                    copies.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(OutputValue::None))
                },
                |_| {
                    cancels.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(()))
                },
                |boundary| {
                    if boundary == injected {
                        assert_eq!(registry.claim_destroyed(&label), None);
                    }
                },
            ),
        )
        .await
        .expect("finish remains authorized after click");
        assert_eq!(result.action, LongshotOutputAction::Copy);
        assert_eq!(finishes.load(Ordering::SeqCst), 1);
        assert_eq!(copies.load(Ordering::SeqCst), 1);
        assert_eq!(cancels.load(Ordering::SeqCst), 0);
        assert!(registry
            .reserve("capture-overlay-next-7".to_string(), selection())
            .is_ok());
    }
}

#[tokio::test]
async fn finish_pin_success_survives_destroyed_at_every_commit_boundary() {
    for injected in [
        FinishBoundary::AfterClaim,
        FinishBoundary::AfterFinish,
        FinishBoundary::BeforeOutput,
        FinishBoundary::AfterOutput,
        FinishBoundary::BeforeCommit,
    ] {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let finishes = AtomicUsize::new(0);
        let pins = AtomicUsize::new(0);
        let result = execute_finish_with_ops(
            &registry,
            &label,
            &handle,
            LongshotOutputAction::Pin,
            &TestWindows::default(),
            FinishOperations::new(
                |_| {
                    finishes.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(lifecycle_artifact(vec![2, 0, 2, 6])))
                },
                |_| false,
                |action, artifact| {
                    assert_eq!(action, LongshotOutputAction::Pin);
                    assert_eq!(artifact.origin, test_origin());
                    pins.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(OutputValue::PinLabel("pin-image-boundary".to_string())))
                },
                |_| std::future::ready(Ok(())),
                |boundary| {
                    if boundary == injected {
                        assert_eq!(registry.claim_destroyed(&label), None);
                    }
                },
            ),
        )
        .await
        .expect("已领取的 Pin 输出保持授权");

        assert_eq!(result.action, LongshotOutputAction::Pin);
        assert_eq!(result.pin_label.as_deref(), Some("pin-image-boundary"));
        assert_eq!(finishes.load(Ordering::SeqCst), 1);
        assert_eq!(pins.load(Ordering::SeqCst), 1);
        assert!(registry
            .reserve("capture-overlay-after-pin".to_string(), selection())
            .is_ok());
    }
}
