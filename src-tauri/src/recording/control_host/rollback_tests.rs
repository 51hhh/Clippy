use super::{finish_control_close, rollback_prepared_control_with, RecordingControlRegistry};
use crate::recording::control_registry::RecordingControlRegistryError;
use crate::recording::manager::RecordingToken;
use std::cell::Cell;

#[test]
fn recording_control_rollback_failed_destroy_keeps_registry_quarantined() {
    let registry = RecordingControlRegistry::new();
    let label = registry.reserve("prepare-failed").unwrap();
    let calls = Cell::new(0);
    let result = rollback_prepared_control_with(&registry, "prepare-failed", |requested| {
        assert_eq!(requested, label);
        calls.set(calls.get() + 1);
        Err("native destroy failed".to_string())
    });

    assert_eq!(
        registry.reserve("replacement"),
        Err(RecordingControlRegistryError::Busy)
    );
    assert_eq!(result, Err("native destroy failed".to_string()));
    assert_eq!(calls.get(), 1);
}

#[test]
fn recording_control_rollback_repeat_does_not_clear_failed_cleanup() {
    let registry = RecordingControlRegistry::new();
    registry.reserve("failed").unwrap();
    let _ = rollback_prepared_control_with(&registry, "failed", |_| Err("destroy".to_string()));
    let calls = Cell::new(0);
    assert!(rollback_prepared_control_with(&registry, "failed", |_| {
        calls.set(calls.get() + 1);
        Ok(())
    })
    .is_ok());

    assert_eq!(calls.get(), 0);
    assert_eq!(
        registry.reserve("next"),
        Err(RecordingControlRegistryError::Busy)
    );
}

#[test]
fn recording_control_rollback_success_retires_old_caller() {
    let registry = RecordingControlRegistry::new();
    let old = registry.reserve("old").unwrap();
    let calls = Cell::new(0);
    rollback_prepared_control_with(&registry, "old", |label| {
        assert_eq!(label, old);
        calls.set(calls.get() + 1);
        Ok(())
    })
    .unwrap();
    let next = registry.reserve("new").unwrap();
    let token = RecordingToken {
        session_id: "new".to_string(),
        generation: 2,
    };
    registry.bind(&token).unwrap();

    assert_eq!(calls.get(), 1);
    assert_ne!(old, next);
    assert_eq!(
        registry.mark_ready(&old),
        Err(RecordingControlRegistryError::Superseded)
    );
    assert_eq!(registry.token_for_caller(&next).unwrap(), token);
}

#[test]
fn recording_control_rollback_wrong_session_does_not_destroy_owner() {
    let registry = RecordingControlRegistry::new();
    let label = registry.reserve("owner").unwrap();
    let calls = Cell::new(0);
    rollback_prepared_control_with(&registry, "foreign", |_| {
        calls.set(calls.get() + 1);
        Err("must not execute".to_string())
    })
    .unwrap();
    let token = RecordingToken {
        session_id: "owner".to_string(),
        generation: 1,
    };
    registry.bind(&token).unwrap();

    assert_eq!(calls.get(), 0);
    assert_eq!(registry.token_for_caller(&label).unwrap(), token);
}

#[test]
fn recording_control_rollback_missing_reservation_does_not_call_destroy() {
    let registry = RecordingControlRegistry::new();
    let calls = Cell::new(0);
    rollback_prepared_control_with(&registry, "missing", |_| {
        calls.set(calls.get() + 1);
        Err("must not execute".to_string())
    })
    .unwrap();

    assert_eq!(calls.get(), 0);
    assert!(registry.reserve("next").is_ok());
}

#[test]
fn recording_control_rollback_normal_close_keeps_original_destroy_error() {
    let registry = RecordingControlRegistry::new();
    registry.reserve("normal").unwrap();
    let close = registry.begin_close("normal").unwrap();
    let result = finish_control_close(&registry, close, Err("original close error".to_string()));

    assert_eq!(result, Err("original close error".to_string()));
    assert_eq!(
        registry.reserve("next"),
        Err(RecordingControlRegistryError::Busy)
    );
}

#[test]
fn recording_control_rollback_normal_success_allows_replacement() {
    let registry = RecordingControlRegistry::new();
    registry.reserve("normal").unwrap();
    let close = registry.begin_close("normal").unwrap();
    finish_control_close(&registry, close, Ok(())).unwrap();

    assert!(registry.reserve("replacement").is_ok());
}
