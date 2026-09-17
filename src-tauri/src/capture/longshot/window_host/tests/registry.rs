use super::*;

#[test]
fn generation_parser_is_canonical_and_lossless() {
    let handle = LongshotControllerHandle {
        session_id: "session".to_string(),
        generation: u64::MAX.to_string(),
    };
    assert_eq!(handle.to_token().expect("u64 max").wire_parts().1, u64::MAX);
    for invalid in ["", " 1", "+1", "-1", "01", "18446744073709551616"] {
        let handle = LongshotControllerHandle {
            session_id: "session".to_string(),
            generation: invalid.to_string(),
        };
        assert_eq!(
            handle.to_token().expect_err("必须拒绝").code,
            "longshot_controller_superseded"
        );
    }
}

#[test]
fn pin_output_wire_contract_keeps_label_separate_from_save_path() {
    let action: LongshotOutputAction = serde_json::from_str("\"pin\"").expect("Pin action");
    assert_eq!(action, LongshotOutputAction::Pin);
    assert_eq!(serde_json::to_string(&action).unwrap(), "\"pin\"");

    let value = serde_json::to_value(LongshotOutputResult {
        action,
        path: None,
        pin_label: Some("pin-image-longshot".to_string()),
    })
    .expect("Pin result");
    assert_eq!(value["action"], "pin");
    assert_eq!(value["path"], serde_json::Value::Null);
    assert_eq!(value["pinLabel"], "pin-image-longshot");
}

#[test]
fn serde_rejects_numeric_generation() {
    let error =
        serde_json::from_str::<LongshotControllerHandle>(r#"{"sessionId":"s","generation":1}"#)
            .expect_err("JSON number 不能进入字符串 generation");
    assert!(error.to_string().contains("string"));
}

#[test]
fn stale_handle_cannot_claim_active_generation() {
    let (registry, label, token) = active_registry();
    let stale = LongshotControllerHandle {
        session_id: "longshot".to_string(),
        generation: "8".to_string(),
    };
    assert_eq!(
        registry
            .claim_cancel(&label, Some(&stale))
            .expect_err("旧代次")
            .code,
        "longshot_controller_superseded"
    );
    assert_eq!(registry.claim_destroyed(&label), Some(token));
}

#[test]
fn old_label_events_never_touch_new_reservation() {
    let registry = LongshotControllerRegistry::new();
    let old = registry
        .reserve("capture-overlay-a-7".to_string(), selection())
        .expect("old reserve");
    assert!(matches!(
        registry.claim_deadline(&old),
        DeadlineAction::Close
    ));
    let new = registry
        .reserve("capture-overlay-b-7".to_string(), selection())
        .expect("new reserve");
    assert_eq!(registry.claim_destroyed(&old), None);
    assert!(registry.accepts_built_window(&new));
}

#[test]
fn every_nonempty_representative_blocks_duplicate_open() {
    let registry = LongshotControllerRegistry::new();
    let label = registry
        .reserve("capture-overlay-a-7".to_string(), selection())
        .expect("Building");
    assert_eq!(
        registry
            .reserve("capture-overlay-b-7".to_string(), selection())
            .expect_err("Building busy")
            .code,
        "longshot_controller_busy"
    );
    assert!(registry.publish_started(&label, CONTROLLER_PAGE));
    assert_eq!(
        registry
            .reserve("capture-overlay-b-7".to_string(), selection())
            .expect_err("Pending busy")
            .code,
        "longshot_controller_busy"
    );
    registry.claim_activation(&label).expect("Activating");
    assert_eq!(
        registry
            .reserve("capture-overlay-b-7".to_string(), selection())
            .expect_err("Activating busy")
            .code,
        "longshot_controller_busy"
    );
    let _ = registry.complete_activation(&label, Ok(start(9)));
    assert_eq!(
        registry
            .reserve("capture-overlay-b-7".to_string(), selection())
            .expect_err("Active busy")
            .code,
        "longshot_controller_busy"
    );
}

#[test]
fn max_generation_survives_json_round_trip_as_string() {
    let original = LongshotControllerHandle {
        session_id: "session".to_string(),
        generation: u64::MAX.to_string(),
    };
    let json = serde_json::to_string(&original).expect("serialize");
    assert!(json.contains(r#""generation":"18446744073709551615""#));
    let decoded: LongshotControllerHandle = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded, original);
    assert_eq!(decoded.to_token().expect("parse").wire_parts().1, u64::MAX);
}

#[test]
fn stale_handoff_cannot_unlock_a_replaced_ordinary_capture() {
    let registry = LongshotControllerRegistry::new();
    let label = registry
        .reserve("capture-overlay-old".into(), selection())
        .unwrap();
    assert!(registry.take_handoff(&label, false, |_| false).is_none());
    assert!(registry
        .take_handoff("another-controller", false, |_| true)
        .is_none());
}
