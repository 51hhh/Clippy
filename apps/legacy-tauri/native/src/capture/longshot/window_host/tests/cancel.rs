use super::*;

#[test]
fn cancel_during_activation_only_records_request() {
    let registry = LongshotControllerRegistry::new();
    let label = registry
        .reserve("capture-overlay-a-7".to_string(), selection())
        .expect("reserve");
    assert!(registry.publish_started(&label, CONTROLLER_PAGE));
    registry.claim_activation(&label).expect("activation claim");
    assert!(matches!(
        registry.claim_cancel(&label, None).expect("cancel"),
        CancelAction::Requested
    ));
    let error = CaptureError::SessionMissing;
    assert_eq!(
        registry
            .complete_activation(&label, Err(error))
            .expect_err("primary error")
            .code,
        "session_missing"
    );
    assert!(registry
        .reserve("capture-overlay-b-7".to_string(), selection())
        .is_ok());
}

#[test]
fn failed_ready_is_idempotent_and_cancel_never_needs_handle() {
    let registry = LongshotControllerRegistry::new();
    let label = registry
        .reserve("capture-overlay-a-7".to_string(), selection())
        .expect("reserve");
    assert!(registry.publish_started(&label, CONTROLLER_PAGE));
    registry.claim_activation(&label).expect("activation claim");
    let _ = registry.complete_activation(&label, Err(CaptureError::SessionMissing));
    assert!(matches!(
        registry.claim_ready(&label),
        Ok(ReadyAction::ShowFailed)
    ));
    assert!(matches!(
        registry.claim_ready(&label),
        Ok(ReadyAction::None)
    ));
    assert!(matches!(
        registry.claim_cancel(&label, None),
        Ok(CancelAction::Close)
    ));
}

#[test]
fn ready_show_failure_then_destroyed_has_one_termination_winner() {
    let (registry, label, token) = active_registry();
    assert!(matches!(
        registry.claim_ready(&label),
        Ok(ReadyAction::ShowActive(_))
    ));
    assert_eq!(
        registry.claim_forced_termination(&label, &token),
        Some(token.clone())
    );
    assert_eq!(registry.claim_destroyed(&label), None);
    registry.complete_cancel_success(&label, &token);
    assert!(registry
        .reserve("capture-overlay-next-7".to_string(), selection())
        .is_ok());
}

#[test]
fn destroyed_before_ready_show_failure_has_one_termination_winner() {
    let (registry, label, token) = active_registry();
    assert_eq!(registry.claim_destroyed(&label), Some(token.clone()));
    assert_eq!(registry.claim_forced_termination(&label, &token), None);
    registry.complete_cancel_success(&label, &token);
}

#[test]
fn deadline_cleanup_failure_never_reopens_invisible_active_slot() {
    let (registry, label, token) = active_registry();
    assert!(matches!(
        registry.claim_deadline(&label),
        DeadlineAction::Terminate(ref claimed) if claimed == &token
    ));
    registry.complete_cancel_failure(&label, &token, true);
    assert_eq!(
        registry
            .reserve("capture-overlay-next-7".to_string(), selection())
            .expect_err("CleanupFailed 必须阻止新窗口")
            .code,
        "longshot_controller_busy"
    );
}

#[test]
fn cancelled_activation_success_is_compensated_without_publishing_active() {
    let registry = LongshotControllerRegistry::new();
    let label = registry
        .reserve("capture-overlay-a-7".to_string(), selection())
        .expect("reserve");
    assert!(registry.publish_started(&label, CONTROLLER_PAGE));
    registry.claim_activation(&label).expect("claim");
    assert!(matches!(
        registry.claim_cancel(&label, None),
        Ok(CancelAction::Requested)
    ));
    let (activation, compensation) = registry
        .complete_activation(&label, Ok(start(9)))
        .expect("late begin success");
    assert!(activation.is_none(), "Active 不能发布给已销毁窗口");
    let token = compensation.expect("必须补偿 cancel");
    assert_eq!(registry.claim_destroyed(&label), None);
    registry.complete_cancel_success(&label, &token);
    assert!(registry
        .reserve("capture-overlay-next-7".to_string(), selection())
        .is_ok());
}

#[test]
fn explicit_cancel_before_destroyed_keeps_single_winner_and_can_retry_live_failure() {
    let (registry, label, token) = active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    assert!(matches!(
        registry.claim_cancel(&label, Some(&handle)),
        Ok(CancelAction::Terminate(ref claimed)) if claimed == &token
    ));
    assert_eq!(registry.claim_destroyed("longshot-controller-old"), None);
    registry.complete_cancel_failure(&label, &token, true);
    assert!(matches!(
        registry.claim_cancel(&label, Some(&handle)),
        Ok(CancelAction::Terminate(_))
    ));
    assert_eq!(registry.claim_destroyed(&label), None);
    registry.complete_cancel_success(&label, &token);
}

#[test]
fn revealed_active_is_immune_to_deadline_but_destroyed_still_cleans_it() {
    let (registry, label, token) = active_registry();
    assert!(matches!(
        registry.claim_ready(&label),
        Ok(ReadyAction::ShowActive(_))
    ));
    assert!(matches!(
        registry.claim_deadline(&label),
        DeadlineAction::None
    ));
    assert_eq!(registry.claim_destroyed(&label), Some(token));
}

#[test]
fn failed_terminating_and_cleanup_failed_all_block_duplicate_open() {
    let registry = LongshotControllerRegistry::new();
    let token = LongshotSessionToken::from_wire_parts("longshot".to_string(), 3);
    for slot in [
        Slot::Failed {
            label: "longshot-controller-failed".to_string(),
            revealed: false,
        },
        Slot::Terminating {
            label: "longshot-controller-terminating".to_string(),
            token: token.clone(),
            snapshot: Some(start(3).snapshot),
            window_destroyed: false,
            origin: TerminationOrigin::RevealedActive,
        },
        Slot::CleanupFailed {
            label: "longshot-controller-cleanup".to_string(),
            _token: Some(token.clone()),
            revealed: false,
        },
    ] {
        *registry.slot.lock().expect("test slot") = slot;
        assert_eq!(
            registry
                .reserve("capture-overlay-b-7".to_string(), selection())
                .expect_err("非空状态必须 Busy")
                .code,
            "longshot_controller_busy"
        );
    }
}

#[test]
fn emergency_cleanup_decision_matches_resource_certainty() {
    assert_eq!(emergency_decision(true, true), EmergencyDecision::Destroy);
    assert_eq!(
        emergency_decision(false, true),
        EmergencyDecision::AwaitReady
    );
    assert_eq!(emergency_decision(true, false), EmergencyDecision::Reveal);
    assert_eq!(emergency_decision(false, false), EmergencyDecision::Reveal);
}

#[test]
fn emergency_cleanup_success_clears_exact_slot_but_failure_stays_revealable() {
    let registry = LongshotControllerRegistry::new();
    let label = registry
        .reserve("capture-overlay-a-7".to_string(), selection())
        .expect("reserve");
    assert!(registry.publish_started(&label, CONTROLLER_PAGE));
    registry.claim_activation(&label).expect("Activating");
    let token = LongshotSessionToken::from_wire_parts("longshot".to_string(), 9);
    assert!(registry.mark_cleanup_failed(&label, Some(token.clone())));
    assert!(registry.settle_emergency_cleanup(&label, &token, true));
    assert!(registry
        .reserve("capture-overlay-next-7".to_string(), selection())
        .is_ok());

    let failed = LongshotControllerRegistry::new();
    let failed_label = failed
        .reserve("capture-overlay-b-7".to_string(), selection())
        .expect("reserve failed path");
    assert!(failed.publish_started(&failed_label, CONTROLLER_PAGE));
    failed.claim_activation(&failed_label).expect("Activating");
    assert!(failed.mark_cleanup_failed(&failed_label, Some(token.clone())));
    assert!(failed.settle_emergency_cleanup(&failed_label, &token, false));
    assert!(matches!(
        failed.claim_ready(&failed_label),
        Ok(ReadyAction::ShowCleanup)
    ));
    assert_eq!(
        failed
            .reserve("capture-overlay-next-7".to_string(), selection())
            .expect_err("清理失败不能开放新会话")
            .code,
        "longshot_controller_busy"
    );
}

#[tokio::test]
async fn pending_cancel_and_destroyed_both_orders_destroy_at_most_once() {
    let (registry, label) = pending_registry();
    let windows = TestWindows::default();
    windows.set_existing(true);
    let cancel_calls = AtomicUsize::new(0);
    let action = registry.claim_cancel(&label, None).expect("cancel first");
    execute_cancel_action(action, &label, &windows, |_| async {
        cancel_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
    .await
    .expect("execute");
    assert_eq!(registry.claim_destroyed(&label), None);
    assert!(registry.claim_cancel(&label, None).is_err());
    assert_eq!(windows.destroy_count(), 1);
    assert_eq!(windows.attempt_count(), 1);
    assert_eq!(windows.destroyed_labels(), vec![label.clone()]);
    assert_eq!(cancel_calls.load(Ordering::SeqCst), 0);

    let (registry, label) = pending_registry();
    let windows = TestWindows::default();
    windows.set_existing(false); // external Destroyed 已经移除窗口
    assert_eq!(registry.claim_destroyed(&label), None);
    assert!(registry.claim_cancel(&label, None).is_err());
    assert_eq!(windows.destroy_count(), 0);
    assert_eq!(windows.attempt_count(), 0);
}

#[tokio::test]
async fn failed_cancel_and_destroyed_both_orders_destroy_at_most_once() {
    let (registry, label) = failed_registry();
    let windows = TestWindows::default();
    windows.set_existing(true);
    let action = registry.claim_cancel(&label, None).expect("cancel first");
    execute_cancel_action(action, &label, &windows, |_| async { Ok(()) })
        .await
        .expect("execute");
    assert_eq!(registry.claim_destroyed(&label), None);
    assert!(registry.claim_cancel(&label, None).is_err());
    assert_eq!(windows.destroy_count(), 1);
    assert_eq!(windows.attempt_count(), 1);
    assert_eq!(windows.destroyed_labels(), vec![label.clone()]);

    let (registry, label) = failed_registry();
    let windows = TestWindows::default();
    assert_eq!(registry.claim_destroyed(&label), None);
    assert!(registry.claim_cancel(&label, None).is_err());
    assert_eq!(windows.destroy_count(), 0);
    assert_eq!(windows.attempt_count(), 0);
}

#[tokio::test]
async fn activating_cancel_and_destroyed_orders_compensate_success_once() {
    for explicit_first in [true, false] {
        let (registry, label) = pending_registry();
        registry.claim_activation(&label).expect("Activating");
        let windows = TestWindows::default();
        windows.set_existing(explicit_first);
        if explicit_first {
            let action = registry.claim_cancel(&label, None).expect("cancel");
            execute_cancel_action(action, &label, &windows, |_| async { Ok(()) })
                .await
                .expect("destroy");
            assert_eq!(registry.claim_destroyed(&label), None);
        } else {
            assert_eq!(registry.claim_destroyed(&label), None);
            let late = registry
                .claim_cancel(&label, None)
                .expect("late cancel 幂等");
            execute_cancel_action(late, &label, &windows, |_| async { Ok(()) })
                .await
                .expect("late cancel executor");
        }
        let (activation, compensation) = registry
            .complete_activation(&label, Ok(start(11)))
            .expect("late success");
        assert!(activation.is_none());
        let token = compensation.expect("compensation token");
        let cancel_calls = AtomicUsize::new(0);
        execute_token_cleanup(Some(token), |claimed| {
            cancel_calls.fetch_add(1, Ordering::SeqCst);
            registry.complete_cancel_success(&label, &claimed);
            std::future::ready(Ok(()))
        })
        .await;
        assert_eq!(cancel_calls.load(Ordering::SeqCst), 1);
        assert_eq!(windows.destroy_count(), usize::from(explicit_first));
        assert_eq!(windows.attempt_count(), 1);
    }
}

#[tokio::test]
async fn activating_cancel_and_destroyed_orders_late_failure_never_cancel_lifecycle() {
    for explicit_first in [true, false] {
        let (registry, label) = pending_registry();
        registry.claim_activation(&label).expect("Activating");
        let windows = TestWindows::default();
        windows.set_existing(explicit_first);
        if explicit_first {
            let action = registry.claim_cancel(&label, None).expect("cancel");
            execute_cancel_action(action, &label, &windows, |_| async { Ok(()) })
                .await
                .expect("destroy");
            assert_eq!(registry.claim_destroyed(&label), None);
        } else {
            assert_eq!(registry.claim_destroyed(&label), None);
            let late = registry
                .claim_cancel(&label, None)
                .expect("late cancel 幂等");
            execute_cancel_action(late, &label, &windows, |_| async { Ok(()) })
                .await
                .expect("late cancel executor");
        }
        let error = registry
            .complete_activation(&label, Err(CaptureError::SessionMissing))
            .expect_err("late begin failure");
        assert_eq!(error.code, "session_missing");
        assert_eq!(windows.destroy_count(), usize::from(explicit_first));
        assert_eq!(windows.attempt_count(), 1);
        assert!(registry
            .reserve("capture-overlay-next-7".to_string(), selection())
            .is_ok());
    }
}

#[tokio::test]
async fn active_cancel_and_destroyed_both_orders_cancel_and_destroy_at_most_once() {
    let (registry, label, token) = active_registry();
    let windows = TestWindows::default();
    windows.set_existing(true);
    let calls = AtomicUsize::new(0);
    let action = registry
        .claim_cancel(&label, Some(&LongshotControllerHandle::from_token(&token)))
        .expect("cancel first");
    execute_cancel_action(action, &label, &windows, |claimed| {
        assert_eq!(claimed, token);
        calls.fetch_add(1, Ordering::SeqCst);
        registry.complete_cancel_success(&label, &claimed);
        std::future::ready(Ok(()))
    })
    .await
    .expect("execute");
    assert_eq!(registry.claim_destroyed(&label), None);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(windows.destroy_count(), 1);
    assert_eq!(windows.attempt_count(), 1);
    assert_eq!(windows.destroyed_labels(), vec![label.clone()]);

    let (registry, label, token) = active_registry();
    let windows = TestWindows::default();
    let claimed = registry.claim_destroyed(&label).expect("Destroyed wins");
    assert_eq!(claimed, token);
    let calls = AtomicUsize::new(0);
    execute_token_cleanup(Some(claimed), |claimed| {
        calls.fetch_add(1, Ordering::SeqCst);
        registry.complete_cancel_success(&label, &claimed);
        std::future::ready(Ok(()))
    })
    .await;
    assert!(registry
        .claim_cancel(&label, Some(&LongshotControllerHandle::from_token(&token)))
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(windows.destroy_count(), 0);
    assert_eq!(windows.attempt_count(), 0);
}

#[tokio::test]
async fn ready_show_failures_apply_active_failed_and_cleanup_contracts() {
    let (active, active_label, token) = active_registry();
    let active_windows = TestWindows::default();
    active_windows.set_existing(true);
    active_windows.set_show_fails(true);
    let active_cancels = AtomicUsize::new(0);
    let active_result = execute_ready_action(
        &active,
        active.claim_ready(&active_label).expect("active ready"),
        &active_label,
        &active_windows,
        |claimed| {
            assert_eq!(claimed, token);
            active_cancels.fetch_add(1, Ordering::SeqCst);
            active.complete_cancel_success(&active_label, &claimed);
            std::future::ready(Ok(()))
        },
    )
    .await;
    assert_eq!(
        active_result.expect_err("show failure").code,
        "longshot_controller_show_failed"
    );
    assert_eq!(active_cancels.load(Ordering::SeqCst), 1);
    assert_eq!(active_windows.destroy_count(), 1);

    let (manager, gate, session_id, caller, _) = ordinary_fixture();
    let (failed, failed_label) = failed_registry();
    let failed_windows = TestWindows::default();
    failed_windows.set_existing(true);
    failed_windows.set_show_fails(true);
    let failed_cancels = AtomicUsize::new(0);
    let _ = execute_ready_action(
        &failed,
        failed.claim_ready(&failed_label).expect("failed ready"),
        &failed_label,
        &failed_windows,
        |_| {
            failed_cancels.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(()))
        },
    )
    .await;
    assert_eq!(failed_cancels.load(Ordering::SeqCst), 0);
    assert_eq!(failed_windows.destroy_count(), 1);
    assert_and_finish_ordinary(&manager, &gate, &session_id, &caller);

    let (cleanup, cleanup_label) = pending_registry();
    cleanup
        .claim_activation(&cleanup_label)
        .expect("Activating");
    assert!(cleanup.mark_cleanup_failed(&cleanup_label, None));
    let cleanup_windows = TestWindows::default();
    cleanup_windows.set_existing(true);
    cleanup_windows.set_show_fails(true);
    let cleanup_result = execute_ready_action(
        &cleanup,
        cleanup.claim_ready(&cleanup_label).expect("cleanup ready"),
        &cleanup_label,
        &cleanup_windows,
        |_| async { panic!("CleanupFailed show failure must not cancel") },
    )
    .await;
    assert_eq!(
        cleanup_result.expect_err("cleanup show failure").code,
        "longshot_controller_cleanup_failed"
    );
    assert_eq!(cleanup_windows.destroy_count(), 0);
    assert!(matches!(
        cleanup.claim_ready(&cleanup_label),
        Ok(ReadyAction::ShowCleanup)
    ));
}

#[tokio::test]
async fn activation_spawn_blocking_panic_becomes_revealable_cleanup_failed() {
    let (registry, label) = pending_registry();
    registry.claim_activation(&label).expect("Activating");
    let failure = run_activation_worker(&registry, &label, || {
        panic!("injected activation worker panic")
    })
    .await
    .expect_err("JoinError must be structured");
    assert!(failure.cleanup_recorded);
    assert_eq!(failure.error.code, "longshot_controller_cleanup_failed");
    assert!(matches!(
        registry.claim_ready(&label),
        Ok(ReadyAction::ShowCleanup)
    ));
    assert_eq!(
        registry
            .reserve("capture-overlay-next-7".to_string(), selection())
            .expect_err("cleanup failure blocks reopen")
            .code,
        "longshot_controller_busy"
    );
}
