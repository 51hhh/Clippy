use super::*;

#[test]
fn append_claim_accepts_only_exact_revealed_active() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let claim = registry
        .claim_append(&label, &handle)
        .expect("exact append");
    assert_eq!(claim.token, token);
    assert_eq!(claim.old_snapshot, start(9).snapshot);
    let stale = LongshotControllerHandle {
        session_id: token.wire_parts().0.to_string(),
        generation: "8".to_string(),
    };
    assert_eq!(
        registry
            .claim_append(&label, &stale)
            .expect_err("valid stale token is superseded while appending")
            .code,
        "longshot_controller_superseded"
    );
    assert_eq!(
        registry
            .claim_append(&label, &handle)
            .expect_err("second append is busy")
            .code,
        "longshot_controller_busy"
    );

    let (unrevealed, unrevealed_label, unrevealed_token) = active_registry();
    assert_eq!(
        unrevealed
            .claim_append(
                &unrevealed_label,
                &LongshotControllerHandle::from_token(&unrevealed_token),
            )
            .expect_err("unrevealed")
            .code,
        "longshot_controller_missing"
    );
}

#[test]
fn append_claim_rejects_stale_malformed_and_unrevealed_before_actions() {
    let (registry, label, token) = revealed_active_registry();
    let windows = TestWindows::default();
    for generation in ["", "01", "18446744073709551616", "8"] {
        let error = registry
            .claim_append(
                &label,
                &LongshotControllerHandle {
                    session_id: token.wire_parts().0.to_string(),
                    generation: generation.to_string(),
                },
            )
            .expect_err("invalid or stale");
        assert_eq!(error.code, "longshot_controller_superseded");
    }
    assert_eq!(
        registry
            .claim_append(
                "capture-overlay-not-controller",
                &LongshotControllerHandle::from_token(&token),
            )
            .expect_err("bad caller")
            .code,
        "longshot_controller_missing"
    );
    let exact = LongshotControllerHandle::from_token(&token);
    registry.claim_append(&label, &exact).expect("exact claim");
    let stale = LongshotControllerHandle {
        session_id: token.wire_parts().0.to_string(),
        generation: "8".to_string(),
    };
    assert_eq!(
        registry
            .claim_append(&label, &stale)
            .expect_err("stale token while Appending")
            .code,
        "longshot_controller_superseded"
    );
    assert!(registry.owns_append(&label, &token));
    assert_eq!(
        registry
            .claim_append(&label, &exact)
            .expect_err("exact duplicate remains busy")
            .code,
        "longshot_controller_busy"
    );
    assert_eq!(windows.hide_count(), 0);
    assert_eq!(windows.show_count(), 0);
    assert_eq!(windows.attempt_count(), 0);
}

#[test]
fn append_completion_requires_exact_owner_and_visible_commit() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    registry.claim_append(&label, &handle).expect("claim");
    let next = LongshotSnapshot {
        frame_count: 2,
        width: 30,
        frame_height: 40,
        total_height: 70,
    };
    assert_eq!(
        registry
            .complete_append_visible("longshot-controller-old", &token, next)
            .expect_err("old label")
            .code,
        "longshot_controller_superseded"
    );
    let stale = LongshotSessionToken::from_wire_parts("longshot".to_string(), 8);
    assert_eq!(
        registry
            .complete_append_visible(&label, &stale, next)
            .expect_err("stale token")
            .code,
        "longshot_controller_superseded"
    );
    let dto = registry
        .complete_append_visible(&label, &token, next)
        .expect("exact visible commit");
    assert_eq!(dto.frame_count, 2);
    assert_eq!(dto.total_height, 70);
    assert_eq!(
        registry
            .complete_append_visible(&label, &token, start(9).snapshot)
            .expect_err("commit once")
            .code,
        "longshot_controller_superseded"
    );
}

#[tokio::test]
async fn append_executor_orders_hide_settle_worker_show_focus_commit() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let windows = TestWindows::default();
    windows.add_alive(&label);
    let next = LongshotSnapshot {
        frame_count: 2,
        width: 30,
        frame_height: 40,
        total_height: 72,
    };
    let result = execute_append_with_ops(
        &registry,
        &label,
        &handle,
        &windows,
        || async { windows.record(format!("settle:{}", crate::capture::HIDE_SETTLE_MS)) },
        |_| async {
            windows.record("append");
            Ok(next)
        },
        |_| async { panic!("success must not cancel") },
        |boundary| {
            if boundary == AppendBoundary::BeforeCommit {
                windows.record("commit-attempt");
            }
        },
    )
    .await
    .expect("append success");
    assert_eq!(result.frame_count, 2);
    assert_eq!(result.total_height, 72);
    assert_eq!(
        windows.trace(),
        vec![
            "hide",
            "settle:140",
            "append",
            "show",
            "focus",
            "commit-attempt"
        ]
    );
    assert!(matches!(
        &*registry.slot.lock().expect("slot after append"),
        Slot::Active {
            label: current,
            token: current_token,
            snapshot,
            revealed: true,
        } if current == &label && current_token == &token && snapshot == &next
    ));
    registry
        .claim_append(&label, &handle)
        .expect("actual visible commit permits the next exact append");
}

#[tokio::test]
async fn append_hide_and_business_failures_restore_visible_old_snapshot() {
    let (hidden, hidden_label, hidden_token) = revealed_active_registry();
    let hidden_handle = LongshotControllerHandle::from_token(&hidden_token);
    let hidden_windows = TestWindows::default();
    hidden_windows.add_alive(&hidden_label);
    hidden_windows.set_hide_fails(true);
    let worker_calls = AtomicUsize::new(0);
    let error = execute_append_with_ops(
        &hidden,
        &hidden_label,
        &hidden_handle,
        &hidden_windows,
        || async {},
        |_| {
            worker_calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(start(10).snapshot))
        },
        |_| async { Ok(()) },
        |_| {},
    )
    .await
    .expect_err("hide failure");
    assert_eq!(error.code, "longshot_controller_hide_failed");
    assert_eq!(worker_calls.load(Ordering::SeqCst), 0);
    assert_eq!(hidden_windows.show_count(), 1);
    assert_eq!(hidden_windows.focus_count(), 1);
    hidden
        .claim_append(&hidden_label, &hidden_handle)
        .expect("hide failure is retryable");

    let (business, business_label, business_token) = revealed_active_registry();
    let business_handle = LongshotControllerHandle::from_token(&business_token);
    let business_windows = TestWindows::default();
    business_windows.add_alive(&business_label);
    let error = execute_append_with_ops(
        &business,
        &business_label,
        &business_handle,
        &business_windows,
        || async {},
        |_| async {
            Err(LongshotIpcError::from(
                CaptureError::LongshotEstimateLowTexture,
            ))
        },
        |_| async { Ok(()) },
        |_| {},
    )
    .await
    .expect_err("business failure");
    assert_eq!(error.code, "longshot_estimate_low_texture");
    assert_eq!(business_windows.show_count(), 1);
    business
        .claim_append(&business_label, &business_handle)
        .expect("business failure is retryable");

    let (raced, raced_label, raced_token) = revealed_active_registry();
    let raced_handle = LongshotControllerHandle::from_token(&raced_token);
    let race_windows = DestroyOnShowWindows {
        registry: &raced,
        label: &raced_label,
        inner: TestWindows::default(),
    };
    race_windows.inner.add_alive(&raced_label);
    race_windows.inner.set_hide_fails(true);
    let error = execute_append_with_ops(
        &raced,
        &raced_label,
        &raced_handle,
        &race_windows,
        || async {},
        |_| async { panic!("hide failure must not append") },
        |_| async { Ok(()) },
        |_| {},
    )
    .await
    .expect_err("Destroyed during hide recovery show wins");
    assert_eq!(error.code, "longshot_controller_superseded");
    assert_eq!(race_windows.inner.destroy_count(), 1);
}

#[tokio::test]
async fn append_spawn_blocking_panic_is_internal_and_retryable() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let windows = TestWindows::default();
    windows.add_alive(&label);
    let error = execute_append_with_ops(
        &registry,
        &label,
        &handle,
        &windows,
        || async {},
        |_| {
            run_append_worker(|| -> Result<LongshotSnapshot, CaptureError> {
                panic!("injected append panic")
            })
        },
        |_| async { Ok(()) },
        |_| {},
    )
    .await
    .expect_err("panic is structured");
    assert_eq!(error.code, "longshot_controller_internal");
    assert_eq!(windows.show_count(), 1);
    registry
        .claim_append(&label, &handle)
        .expect("join failure is retryable");
}

#[tokio::test]
async fn append_cancel_wins_at_every_boundary_without_resurrection() {
    for target in [
        AppendBoundary::AfterClaim,
        AppendBoundary::AfterHide,
        AppendBoundary::AfterSettle,
        AppendBoundary::AfterWorker,
        AppendBoundary::AfterShow,
        AppendBoundary::BeforeCommit,
    ] {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let windows = TestWindows::default();
        windows.add_alive(&label);
        let cancels = AtomicUsize::new(0);
        let result = execute_append_with_ops(
            &registry,
            &label,
            &handle,
            &windows,
            || async {},
            |_| async { Ok(start(10).snapshot) },
            |_| async { panic!("boundary winner owns cleanup") },
            |boundary| {
                if boundary == target {
                    let action = registry
                        .claim_cancel(&label, Some(&handle))
                        .expect("cancel winner");
                    let CancelAction::Terminate(claimed) = action else {
                        panic!("active append cancellation must terminate")
                    };
                    cancels.fetch_add(1, Ordering::SeqCst);
                    registry.complete_cancel_success(&label, &claimed);
                }
            },
        )
        .await;
        assert_eq!(
            result.expect_err("cancel supersedes append").code,
            "longshot_controller_superseded"
        );
        assert_eq!(cancels.load(Ordering::SeqCst), 1);
        assert!(registry
            .reserve("capture-overlay-next-7".to_string(), selection())
            .is_ok());
    }
}

#[tokio::test]
async fn append_destroyed_wins_at_every_boundary_without_second_cleanup() {
    for target in [
        AppendBoundary::AfterClaim,
        AppendBoundary::AfterHide,
        AppendBoundary::AfterSettle,
        AppendBoundary::AfterWorker,
        AppendBoundary::AfterShow,
        AppendBoundary::BeforeCommit,
    ] {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let windows = TestWindows::default();
        windows.add_alive(&label);
        let cancels = AtomicUsize::new(0);
        let result = execute_append_with_ops(
            &registry,
            &label,
            &handle,
            &windows,
            || async {},
            |_| async { Ok(start(10).snapshot) },
            |_| async { panic!("Destroyed winner owns cleanup") },
            |boundary| {
                if boundary == target {
                    let claimed = registry.claim_destroyed(&label).expect("Destroyed winner");
                    cancels.fetch_add(1, Ordering::SeqCst);
                    registry.complete_cancel_success(&label, &claimed);
                }
            },
        )
        .await;
        assert_eq!(
            result.expect_err("Destroyed supersedes append").code,
            "longshot_controller_superseded"
        );
        assert_eq!(cancels.load(Ordering::SeqCst), 1);
        assert!(registry
            .reserve("capture-overlay-next-7".to_string(), selection())
            .is_ok());
    }
}

#[test]
fn appending_cancel_failure_reveals_before_retry_or_enters_cleanup_failed() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    registry.claim_append(&label, &handle).expect("append");
    assert!(matches!(
        registry.claim_cancel(&label, Some(&handle)),
        Ok(CancelAction::Terminate(_))
    ));
    let windows = TestWindows::default();
    windows.add_alive(&label);
    let result = recover_cancel_failure_with_ops(
        &registry,
        &label,
        &token,
        &windows,
        CaptureError::LongshotSessionMissing,
        || {
            windows.record("probe");
            true
        },
    );
    assert_eq!(
        result.expect_err("primary remains").code,
        "longshot_session_missing"
    );
    assert_eq!(windows.trace(), vec!["probe", "show", "focus"]);
    registry
        .claim_append(&label, &handle)
        .expect("visible Active retry");

    for mode in [0, 1, 2] {
        let (failed, failed_label, failed_token) = revealed_active_registry();
        let failed_handle = LongshotControllerHandle::from_token(&failed_token);
        failed
            .claim_append(&failed_label, &failed_handle)
            .expect("append");
        failed
            .claim_cancel(&failed_label, Some(&failed_handle))
            .expect("cancel");
        let failed_windows = TestWindows::default();
        if mode != 0 {
            failed_windows.add_alive(&failed_label);
        }
        if mode == 2 {
            failed_windows.set_show_fails(true);
        }
        let error = recover_cancel_failure_with_ops(
            &failed,
            &failed_label,
            &failed_token,
            &failed_windows,
            CaptureError::LongshotSessionMissing,
            || mode != 1,
        )
        .expect_err("unsafe retry must fail cleanup");
        assert_eq!(error.code, "longshot_controller_cleanup_failed");
        assert_eq!(
            failed
                .reserve("capture-overlay-next-7".to_string(), selection())
                .expect_err("CleanupFailed blocks open")
                .code,
            "longshot_controller_busy"
        );
    }

    let (before_show, before_label, before_token) = revealed_active_registry();
    let before_handle = LongshotControllerHandle::from_token(&before_token);
    before_show
        .claim_append(&before_label, &before_handle)
        .expect("append");
    before_show
        .claim_cancel(&before_label, Some(&before_handle))
        .expect("cancel");
    assert_eq!(before_show.claim_destroyed(&before_label), None);
    let before_windows = TestWindows::default();
    let error = recover_cancel_failure_with_ops(
        &before_show,
        &before_label,
        &before_token,
        &before_windows,
        CaptureError::LongshotSessionMissing,
        || panic!("Forbidden retry visibility must not probe lifecycle"),
    )
    .expect_err("Destroyed before recovery becomes CleanupFailed");
    assert_eq!(error.code, "longshot_controller_cleanup_failed");
    assert!(matches!(
        before_show.claim_ready(&before_label),
        Ok(ReadyAction::ShowCleanup)
    ));
    assert_eq!(
        before_show
            .reserve("capture-overlay-next-7".to_string(), selection())
            .expect_err("Destroyed during retry reveal is CleanupFailed")
            .code,
        "longshot_controller_busy"
    );

    let (after_show, after_label, after_token) = revealed_active_registry();
    let after_handle = LongshotControllerHandle::from_token(&after_token);
    after_show
        .claim_append(&after_label, &after_handle)
        .expect("append");
    after_show
        .claim_cancel(&after_label, Some(&after_handle))
        .expect("cancel");
    let race_windows = DestroyOnShowWindows {
        registry: &after_show,
        label: &after_label,
        inner: TestWindows::default(),
    };
    race_windows.inner.add_alive(&after_label);
    let error = recover_cancel_failure_with_ops(
        &after_show,
        &after_label,
        &after_token,
        &race_windows,
        CaptureError::LongshotSessionMissing,
        || true,
    )
    .expect_err("Destroyed after show prevents Active commit");
    assert_eq!(error.code, "longshot_controller_cleanup_failed");
    assert_eq!(race_windows.inner.destroy_count(), 1);
    assert_eq!(
        after_show
            .reserve("capture-overlay-next-7".to_string(), selection())
            .expect_err("race remains CleanupFailed")
            .code,
        "longshot_controller_busy"
    );
}

#[test]
fn appending_deadline_and_old_aba_events_are_noops() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    registry.claim_append(&label, &handle).expect("append");
    assert!(matches!(
        registry.claim_deadline(&label),
        DeadlineAction::None
    ));
    assert_eq!(registry.claim_destroyed("longshot-controller-old"), None);
    let stale = LongshotSessionToken::from_wire_parts("longshot".to_string(), 8);
    assert_eq!(
        registry
            .complete_append_visible(&label, &stale, start(11).snapshot)
            .expect_err("stale worker")
            .code,
        "longshot_controller_superseded"
    );
    let dto = registry
        .complete_append_visible(&label, &token, start(10).snapshot)
        .expect("current commit");
    assert_eq!(dto.frame_count, 1);
}

#[tokio::test]
async fn append_show_failure_prioritizes_visibility_cleanup_for_all_worker_results() {
    for worker_kind in 0..3 {
        let (registry, label, token) = revealed_active_registry();
        let handle = LongshotControllerHandle::from_token(&token);
        let windows = TestWindows::default();
        windows.add_alive(&label);
        windows.set_show_fails(true);
        let cancels = AtomicUsize::new(0);
        let result = execute_append_with_ops(
            &registry,
            &label,
            &handle,
            &windows,
            || async {},
            |_| async move {
                match worker_kind {
                    0 => Ok(start(10).snapshot),
                    1 => Err(LongshotIpcError::from(
                        CaptureError::LongshotEstimateLowTexture,
                    )),
                    _ => Err(LongshotIpcError::internal("join failure")),
                }
            },
            |claimed| {
                cancels.fetch_add(1, Ordering::SeqCst);
                registry.complete_cancel_success(&label, &claimed);
                std::future::ready(Ok(()))
            },
            |_| {},
        )
        .await;
        assert_eq!(
            result.expect_err("show outranks worker").code,
            "longshot_controller_show_failed"
        );
        assert_eq!(cancels.load(Ordering::SeqCst), 1);
        assert_eq!(windows.destroy_count(), 1);
        assert!(registry
            .reserve("capture-overlay-next-7".to_string(), selection())
            .is_ok());
    }

    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let windows = TestWindows::default();
    windows.add_alive(&label);
    windows.set_show_fails(true);
    let result = execute_append_with_ops(
        &registry,
        &label,
        &handle,
        &windows,
        || async {},
        |_| async { Ok(start(10).snapshot) },
        |_| {
            std::future::ready(Err(LongshotIpcError::cleanup_failed(
                "injected direct cleanup failure",
            )))
        },
        |_| {},
    )
    .await;
    assert_eq!(
        result.expect_err("cleanup outranks show").code,
        "longshot_controller_cleanup_failed"
    );
    assert!(matches!(
        registry.claim_ready(&label),
        Ok(ReadyAction::ShowCleanup)
    ));
}
