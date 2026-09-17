use super::*;

#[test]
fn started_barrier_requires_exact_first_path_and_label() {
    let registry = LongshotControllerRegistry::new();
    let label = registry
        .reserve("capture-overlay-a-7".to_string(), selection())
        .expect("reserve");
    assert!(!registry.publish_started(&label, "/wrong.html"));
    assert_eq!(
        registry
            .claim_activation(&label)
            .expect_err("Building 绝不能 activation")
            .code,
        "longshot_controller_missing"
    );
    assert!(!registry.publish_started("longshot-controller-old", CONTROLLER_PAGE));
    assert!(registry.publish_started(&label, CONTROLLER_PAGE));
    assert!(!registry.publish_started(&label, CONTROLLER_PAGE));
    assert!(registry.claim_activation(&label).is_ok());
}

#[test]
fn deadline_before_publish_revokes_exact_launch_only() {
    let registry = LongshotControllerRegistry::new();
    let label = registry
        .reserve("capture-overlay-a-7".to_string(), selection())
        .expect("reserve");
    assert!(matches!(
        registry.claim_deadline("old"),
        DeadlineAction::None
    ));
    assert!(registry.accepts_built_window(&label));
    assert!(matches!(
        registry.claim_deadline(&label),
        DeadlineAction::Close
    ));
    assert!(!registry.accepts_built_window(&label));
    assert!(registry
        .reserve("capture-overlay-b-7".to_string(), selection())
        .is_ok());
}

#[test]
fn activation_worker_failure_is_revealable_but_never_closeable_or_reopenable() {
    let registry = LongshotControllerRegistry::new();
    let label = registry
        .reserve("capture-overlay-a-7".to_string(), selection())
        .expect("reserve");
    assert!(registry.publish_started(&label, CONTROLLER_PAGE));
    registry.claim_activation(&label).expect("Activating");
    assert!(registry.mark_cleanup_failed(&label, None));
    assert!(matches!(
        registry.claim_ready(&label),
        Ok(ReadyAction::ShowCleanup)
    ));
    assert!(matches!(
        registry.claim_ready(&label),
        Ok(ReadyAction::None)
    ));
    registry.rollback_cleanup_reveal(&label);
    assert!(matches!(
        registry.claim_ready(&label),
        Ok(ReadyAction::ShowCleanup)
    ));
    assert_eq!(
        registry
            .claim_cancel(&label, None)
            .expect_err("CleanupFailed 不能被普通 Close 清槽")
            .code,
        "longshot_controller_missing"
    );
    assert_eq!(registry.claim_destroyed(&label), None);
    assert_eq!(
        registry
            .reserve("capture-overlay-next-7".to_string(), selection())
            .expect_err("CleanupFailed 必须持续阻止新会话")
            .code,
        "longshot_controller_busy"
    );
}

#[tokio::test]
async fn pending_and_activating_deadlines_execute_exact_destroy_once() {
    let (pending, pending_label) = pending_registry();
    let pending_windows = TestWindows::default();
    pending_windows.set_existing(true);
    execute_deadline_action(
        pending.claim_deadline(&pending_label),
        &pending_label,
        &pending_windows,
        |_| async { Ok(()) },
    )
    .await;
    assert_eq!(pending_windows.destroy_count(), 1);
    assert_eq!(pending_windows.attempt_count(), 1);
    assert_eq!(
        pending_windows.destroyed_labels(),
        vec![pending_label.clone()]
    );
    assert!(!pending.accepts_built_window(&pending_label));

    let (activating, activating_label) = pending_registry();
    activating
        .claim_activation(&activating_label)
        .expect("Activating");
    let activating_windows = TestWindows::default();
    activating_windows.set_existing(true);
    execute_deadline_action(
        activating.claim_deadline(&activating_label),
        &activating_label,
        &activating_windows,
        |_| async { Ok(()) },
    )
    .await;
    assert_eq!(activating_windows.destroy_count(), 1);
    assert_eq!(activating_windows.attempt_count(), 1);
    let (_, compensation) = activating
        .complete_activation(&activating_label, Ok(start(12)))
        .expect("late success");
    let activating_cancels = AtomicUsize::new(0);
    execute_token_cleanup(compensation, |claimed| {
        activating_cancels.fetch_add(1, Ordering::SeqCst);
        activating.complete_cancel_success(&activating_label, &claimed);
        std::future::ready(Ok(()))
    })
    .await;
    assert_eq!(activating_cancels.load(Ordering::SeqCst), 1);
    assert!(activating
        .reserve("capture-overlay-next-7".to_string(), selection())
        .is_ok());

    let (failing, failing_label) = pending_registry();
    failing
        .claim_activation(&failing_label)
        .expect("Activating failure");
    let failing_windows = TestWindows::default();
    failing_windows.set_existing(true);
    execute_deadline_action(
        failing.claim_deadline(&failing_label),
        &failing_label,
        &failing_windows,
        |_| async { panic!("late begin failure 不能 cancel lifecycle") },
    )
    .await;
    let error = failing
        .complete_activation(&failing_label, Err(CaptureError::SessionMissing))
        .expect_err("late failure");
    assert_eq!(error.code, "session_missing");
    assert_eq!(failing_windows.destroy_count(), 1);
    assert!(failing
        .reserve("capture-overlay-next-7".to_string(), selection())
        .is_ok());
}

#[tokio::test]
async fn active_revealed_unrevealed_and_old_label_deadlines_execute_contract() {
    let (registry, label, token) = active_registry();
    let windows = TestWindows::default();
    windows.set_existing(true);
    let cancels = AtomicUsize::new(0);
    execute_deadline_action(
        registry.claim_deadline(&label),
        &label,
        &windows,
        |claimed| {
            assert_eq!(claimed, token);
            cancels.fetch_add(1, Ordering::SeqCst);
            registry.complete_cancel_success(&label, &claimed);
            std::future::ready(Ok(()))
        },
    )
    .await;
    assert_eq!(cancels.load(Ordering::SeqCst), 1);
    assert_eq!(windows.attempt_count(), 1);
    assert_eq!(windows.destroy_count(), 1);
    assert_eq!(windows.destroyed_labels(), vec![label]);

    let (revealed, revealed_label, _) = active_registry();
    assert!(matches!(
        revealed.claim_ready(&revealed_label),
        Ok(ReadyAction::ShowActive(_))
    ));
    let revealed_windows = TestWindows::default();
    revealed_windows.set_existing(true);
    let revealed_cancels = AtomicUsize::new(0);
    execute_deadline_action(
        revealed.claim_deadline(&revealed_label),
        &revealed_label,
        &revealed_windows,
        |_| {
            revealed_cancels.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(()))
        },
    )
    .await;
    assert_eq!(revealed_cancels.load(Ordering::SeqCst), 0);
    assert_eq!(revealed_windows.attempt_count(), 0);
    assert_eq!(revealed_windows.destroy_count(), 0);

    let (current, current_label) = pending_registry();
    let old_windows = TestWindows::default();
    old_windows.set_existing(true);
    execute_deadline_action(
        current.claim_deadline("longshot-controller-old"),
        "longshot-controller-old",
        &old_windows,
        |_| async { Ok(()) },
    )
    .await;
    assert_eq!(old_windows.attempt_count(), 0);
    assert!(current.accepts_built_window(&current_label));
}

#[tokio::test]
async fn late_and_partial_build_outcomes_destroy_the_actual_exact_attempt_once() {
    let registry = LongshotControllerRegistry::new();
    let label = registry
        .reserve("capture-overlay-a-7".to_string(), selection())
        .expect("reserve");
    let windows = TestWindows::default();
    windows.add_alive("longshot-controller-decoy");
    execute_deadline_action(
        registry.claim_deadline(&label),
        &label,
        &windows,
        |_| async { Ok(()) },
    )
    .await;
    assert_eq!(windows.destroy_count(), 0, "deadline 时窗口尚未出现");
    assert_eq!(windows.attempt_count(), 1);
    windows.set_existing(true); // builder 迟到后才真正创建 attempted window
    windows.add_alive(&label);
    assert!(complete_build_attempt(&registry, &label, Ok(()), &windows).is_err());
    assert_eq!(windows.destroy_count(), 1);
    assert_eq!(windows.attempt_count(), 2);
    assert_eq!(windows.destroyed_labels(), vec![label.clone()]);
    assert!(windows.is_alive("longshot-controller-decoy"));

    let partial = LongshotControllerRegistry::new();
    let partial_label = partial
        .reserve("capture-overlay-b-7".to_string(), selection())
        .expect("reserve partial");
    let partial_windows = TestWindows::default();
    partial_windows.set_existing(true);
    assert!(complete_build_attempt(
        &partial,
        &partial_label,
        Err("partial build".to_string()),
        &partial_windows,
    )
    .is_err());
    assert_eq!(partial_windows.destroy_count(), 1);
    assert_eq!(partial_windows.attempt_count(), 1);
    assert_eq!(
        partial_windows.destroyed_labels(),
        vec![partial_label.clone()]
    );
    assert!(!partial.accepts_built_window(&partial_label));
    assert!(partial
        .reserve("capture-overlay-next-7".to_string(), selection())
        .is_ok());

    let absent = LongshotControllerRegistry::new();
    let absent_windows = TestWindows::default();
    let armed = AtomicBool::new(false);
    let error = open_with_ops(
        &absent,
        "capture-overlay-c-7",
        selection(),
        &absent_windows,
        |_, _| Ok(()),
        |_| armed.store(true, Ordering::SeqCst),
        |_| {
            assert!(armed.load(Ordering::SeqCst));
            Err("builder failed before window".to_string())
        },
    )
    .expect_err("builder error");
    assert_eq!(error.code, "longshot_controller_create_failed");
    assert_eq!(absent_windows.attempt_count(), 1);
    assert_eq!(absent_windows.destroy_count(), 0);
    assert!(absent
        .reserve("capture-overlay-next-7".to_string(), selection())
        .is_ok());
}

#[test]
fn open_builder_failure_uses_real_validation_and_preserves_ordinary_resources() {
    let manager = CaptureManager::new();
    let gate = Arc::new(CaptureModeGate::new());
    let ownership = gate
        .try_claim_owned(CaptureMode::Ordinary)
        .expect("Ordinary ownership");
    let frame = CapturedMonitorFrame {
        monitor_id: 7,
        x: 0,
        y: 0,
        logical_width: 100,
        logical_height: 80,
        pixel_width: 100,
        pixel_height: 80,
        scale_x: 1.0,
        scale_y: 1.0,
        rgba: Arc::from(vec![255; 100 * 80 * 4]),
    };
    let start = manager
        .begin(
            vec![frame],
            vec!["main".to_string()],
            vec!["pin-a".to_string()],
            false,
            StageTimings::default(),
            ownership,
        )
        .expect("ordinary begin");
    let caller = start.overlays[0].label.clone();
    let chosen = CaptureSelection {
        session_id: start.session_id.clone(),
        monitor_id: 7,
        x: 1.0,
        y: 2.0,
        width: 30.0,
        height: 40.0,
    };
    let registry = LongshotControllerRegistry::new();
    let windows = TestWindows::default();
    windows.set_existing(true); // 模拟 builder 已部分创建 exact attempted window
    let armed = AtomicBool::new(false);
    let result = open_with_ops(
        &registry,
        &caller,
        chosen,
        &windows,
        |label, selection| manager.validate_longshot_open(label, selection),
        |_| armed.store(true, Ordering::SeqCst),
        |_| {
            assert!(armed.load(Ordering::SeqCst), "deadline 必须先于 builder");
            Err("partial build".to_string())
        },
    );
    assert_eq!(
        result.expect_err("build 必须失败").code,
        "longshot_controller_create_failed"
    );
    assert_eq!(windows.destroy_count(), 1);
    assert_eq!(windows.attempt_count(), 1);
    assert!(manager.payload(&caller).is_ok());
    assert_eq!(
        gate.active_mode().expect("gate"),
        Some(CaptureMode::Ordinary)
    );
    let session = manager.finish(&start.session_id).expect("ordinary intact");
    assert_eq!(session.restore_labels, vec!["main"]);
    assert_eq!(session.lowered_pins, vec!["pin-a"]);
    session.finalize_mode().expect("release");
    assert!(registry
        .reserve("capture-overlay-next-7".to_string(), selection())
        .is_ok());
}

#[test]
fn open_before_window_error_and_late_success_preserve_real_ordinary_resources() {
    let (manager, gate, session_id, caller, chosen) = ordinary_fixture();
    let registry = LongshotControllerRegistry::new();
    let windows = TestWindows::default();
    let result = open_with_ops(
        &registry,
        &caller,
        chosen,
        &windows,
        |label, selection| manager.validate_longshot_open(label, selection),
        |_| {},
        |_| Err("builder failed before window".to_string()),
    );
    assert_eq!(
        result.expect_err("build error").code,
        "longshot_controller_create_failed"
    );
    assert_eq!(windows.attempt_count(), 1);
    assert_eq!(windows.destroy_count(), 0);
    assert_and_finish_ordinary(&manager, &gate, &session_id, &caller);

    let (manager, gate, session_id, caller, chosen) = ordinary_fixture();
    let registry = LongshotControllerRegistry::new();
    let windows = TestWindows::default();
    let result = open_with_ops(
        &registry,
        &caller,
        chosen,
        &windows,
        |label, selection| manager.validate_longshot_open(label, selection),
        |label| {
            let action = registry.claim_deadline(label);
            assert!(matches!(action, DeadlineAction::Close));
            assert_eq!(
                execute_deadline_window_action(action, label, &windows),
                None
            );
        },
        |_| {
            windows.set_existing(true); // deadline 后 builder 才迟到成功
            Ok(())
        },
    );
    let error = result.expect_err("late build must be rejected");
    assert_eq!(error.code, "longshot_controller_missing");
    assert_eq!(windows.attempt_count(), 2);
    assert_eq!(windows.destroy_count(), 1);
    let destroyed = windows.destroyed_labels();
    assert_eq!(destroyed.len(), 1);
    assert!(destroyed[0].starts_with(CONTROLLER_PREFIX));
    assert_and_finish_ordinary(&manager, &gate, &session_id, &caller);
}

#[test]
fn normal_open_with_ops_success_returns_launch_and_preserves_ordinary() {
    let (manager, gate, session_id, caller, chosen) = ordinary_fixture();
    let registry = LongshotControllerRegistry::new();
    let windows = TestWindows::default();
    let armed = AtomicBool::new(false);
    let built = AtomicBool::new(false);
    let launch = open_with_ops(
        &registry,
        &caller,
        chosen,
        &windows,
        |label, selection| manager.validate_longshot_open(label, selection),
        |_| armed.store(true, Ordering::SeqCst),
        |_| {
            assert!(armed.load(Ordering::SeqCst));
            built.store(true, Ordering::SeqCst);
            Ok(())
        },
    )
    .expect("normal build");
    assert!(launch.label.starts_with(CONTROLLER_PREFIX));
    assert!(built.load(Ordering::SeqCst));
    assert!(registry.accepts_built_window(&launch.label));
    assert_eq!(windows.attempt_count(), 0);
    assert_and_finish_ordinary(&manager, &gate, &session_id, &caller);
}

#[tokio::test]
async fn ready_success_shows_and_focuses_exact_window_once() {
    let (registry, label, _) = active_registry();
    let windows = TestWindows::default();
    let first = registry.claim_ready(&label).expect("first ready");
    execute_ready_action(&registry, first, &label, &windows, |_| async {
        panic!("ready success must not cancel")
    })
    .await
    .expect("show success");
    let repeated = registry.claim_ready(&label).expect("repeated ready");
    execute_ready_action(&registry, repeated, &label, &windows, |_| async {
        panic!("repeated ready must not cancel")
    })
    .await
    .expect("idempotent");
    assert_eq!(windows.show_count(), 1);
    assert_eq!(windows.focus_count(), 1);
    assert_eq!(windows.shown.lock().expect("shown").as_slice(), &[label]);
}

#[test]
fn handoff_failure_keeps_the_exact_ordinary_origin_after_activation_consumes_launch() {
    let registry = LongshotControllerRegistry::new();
    let source = selection();
    let label = registry
        .reserve("capture-overlay-original".into(), source.clone())
        .unwrap();
    registry.publish_started(&label, CONTROLLER_PAGE);
    registry.claim_activation(&label).unwrap();
    let (caller, result) = registry
        .take_handoff(&label, false, |id| id == source.session_id)
        .unwrap();
    assert_eq!(caller, "capture-overlay-original");
    let json = serde_json::to_value(result).unwrap();
    assert_eq!(json["controllerLabel"], label);
    assert_eq!(json["sessionId"], source.session_id);
    assert_eq!(json["accepted"], false);
    assert!(registry.take_handoff(&label, false, |_| true).is_none());
}

#[test]
fn controller_deadline_or_destroy_preserves_one_failure_notification() {
    for destroyed in [true, false] {
        let registry = LongshotControllerRegistry::new();
        let label = registry
            .reserve("capture-overlay-original".into(), selection())
            .unwrap();
        if destroyed {
            registry.claim_destroyed(&label);
        } else {
            registry.claim_deadline(&label);
        }
        assert!(registry.take_handoff(&label, false, |_| true).is_some());
        assert!(registry.take_handoff(&label, false, |_| true).is_none());
    }
}
