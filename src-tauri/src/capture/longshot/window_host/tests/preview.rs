use super::*;

#[test]
fn preview_authorization_is_read_only_and_requires_exact_revealed_active() {
    let (registry, label, token) = active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    assert_eq!(
        registry
            .authorize_preview(&label, &handle)
            .expect_err("未 reveal 不可预览")
            .code,
        "longshot_controller_missing"
    );
    assert!(matches!(
        &*registry.slot.lock().expect("slot"),
        Slot::Active {
            revealed: false,
            ..
        }
    ));
    assert!(matches!(
        registry.claim_ready(&label),
        Ok(ReadyAction::ShowActive(_))
    ));

    assert_eq!(
        registry
            .authorize_preview("main", &handle)
            .expect_err("伪 label 不可预览")
            .code,
        "longshot_controller_missing"
    );
    assert_eq!(
        registry
            .authorize_preview("longshot-controller-other", &handle)
            .expect_err("同前缀其他 label 不可预览")
            .code,
        "longshot_controller_missing"
    );
    let malformed = LongshotControllerHandle {
        session_id: handle.session_id.clone(),
        generation: "01".to_string(),
    };
    assert_eq!(
        registry
            .authorize_preview(&label, &malformed)
            .expect_err("畸形 generation 不可预览")
            .code,
        "longshot_controller_superseded"
    );
    let stale = LongshotControllerHandle {
        session_id: handle.session_id.clone(),
        generation: "8".to_string(),
    };
    assert_eq!(
        registry
            .authorize_preview(&label, &stale)
            .expect_err("旧 generation 不可预览")
            .code,
        "longshot_controller_superseded"
    );
    assert_eq!(registry.authorize_preview(&label, &handle).unwrap(), token);
    assert!(registry.confirms_preview(&label, &token).unwrap());
    assert!(matches!(
        &*registry.slot.lock().expect("slot"),
        Slot::Active {
            label: current,
            token: current_token,
            revealed: true,
            ..
        } if current == &label && current_token == &token
    ));
}

#[test]
fn preview_authorization_reports_busy_for_every_exact_worker_phase() {
    let (appending, append_label, append_token) = revealed_active_registry();
    let append_handle = LongshotControllerHandle::from_token(&append_token);
    appending
        .claim_append(&append_label, &append_handle)
        .expect("进入 Appending");
    assert_eq!(
        appending
            .authorize_preview(&append_label, &append_handle)
            .expect_err("Appending 应 busy")
            .code,
        "longshot_controller_busy"
    );
    let stale_append_handle = LongshotControllerHandle {
        session_id: append_handle.session_id.clone(),
        generation: "8".to_string(),
    };
    assert_eq!(
        appending
            .authorize_preview(&append_label, &stale_append_handle)
            .expect_err("Appending 的旧 handle 应 superseded")
            .code,
        "longshot_controller_superseded"
    );

    let (finishing, finish_label, finish_token) = revealed_active_registry();
    let finish_handle = LongshotControllerHandle::from_token(&finish_token);
    finishing
        .claim_finish(&finish_label, &finish_handle, LongshotOutputAction::Copy)
        .expect("进入 Finishing");
    assert_eq!(
        finishing
            .authorize_preview(&finish_label, &finish_handle)
            .expect_err("Finishing 应 busy")
            .code,
        "longshot_controller_busy"
    );

    let (pending, pending_label, pending_token) = revealed_active_registry();
    let pending_handle = LongshotControllerHandle::from_token(&pending_token);
    let snapshot = start(9).snapshot;
    *pending.slot.lock().expect("slot") = Slot::OutputPending {
        label: pending_label.clone(),
        token: pending_token.clone(),
        snapshot,
        artifact: Arc::new(LongshotOutputArtifact {
            png: Arc::new(vec![137, 80, 78, 71]),
            origin: test_origin(),
        }),
        retry_policy: RetryPolicy::Any,
    };
    assert_eq!(
        pending
            .authorize_preview(&pending_label, &pending_handle)
            .expect_err("OutputPending 应 busy")
            .code,
        "longshot_controller_busy"
    );
}

#[tokio::test]
async fn preview_post_check_makes_phase_change_win_over_worker_error() {
    let (registry, label, token) = revealed_active_registry();
    let handle = LongshotControllerHandle::from_token(&token);
    let error = execute_preview_with_ops(
        &registry,
        &label,
        &handle,
        |_| async { Err(LongshotIpcError::new("codec", "旧 worker 错误")) },
        || {
            registry
                .claim_append(&label, &handle)
                .expect("worker 后模拟 Append 获胜");
        },
    )
    .await
    .expect_err("阶段变化必须覆盖 worker 错误");
    assert_eq!(error.code, "longshot_controller_superseded");
}
