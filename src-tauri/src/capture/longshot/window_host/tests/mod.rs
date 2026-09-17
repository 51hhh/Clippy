use super::finish::RetryPolicy;
use super::*;
use crate::capture::longshot::{LongshotArtifact, LongshotSnapshot};
use crate::capture::manager::StageTimings;
use crate::capture::{CaptureManager, CaptureMode, CaptureModeGate};
use crate::pin::PinOrigin;
use crate::screenshot::CapturedMonitorFrame;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct TestWindows {
    existing: Mutex<bool>,
    named_alive: Mutex<std::collections::HashSet<String>>,
    attempts: Mutex<Vec<String>>,
    destroyed: Mutex<Vec<String>>,
    shown: Mutex<Vec<String>>,
    hidden: Mutex<Vec<String>>,
    focused: Mutex<Vec<String>>,
    trace: Mutex<Vec<String>>,
    hide_fails: AtomicBool,
    show_fails: AtomicBool,
}

impl TestWindows {
    fn set_existing(&self, existing: bool) {
        *self.existing.lock().expect("existing") = existing;
    }

    fn destroy_count(&self) -> usize {
        self.destroyed.lock().expect("destroyed").len()
    }

    fn attempt_count(&self) -> usize {
        self.attempts.lock().expect("attempts").len()
    }

    fn destroyed_labels(&self) -> Vec<String> {
        self.destroyed.lock().expect("destroyed").clone()
    }

    fn add_alive(&self, label: &str) {
        self.named_alive
            .lock()
            .expect("named alive")
            .insert(label.to_string());
    }

    fn is_alive(&self, label: &str) -> bool {
        self.named_alive
            .lock()
            .expect("named alive")
            .contains(label)
    }

    fn set_show_fails(&self, fails: bool) {
        self.show_fails.store(fails, Ordering::SeqCst);
    }

    fn set_hide_fails(&self, fails: bool) {
        self.hide_fails.store(fails, Ordering::SeqCst);
    }

    fn hide_count(&self) -> usize {
        self.hidden.lock().expect("hidden").len()
    }

    fn show_count(&self) -> usize {
        self.shown.lock().expect("shown").len()
    }

    fn focus_count(&self) -> usize {
        self.focused.lock().expect("focused").len()
    }

    fn trace(&self) -> Vec<String> {
        self.trace.lock().expect("trace").clone()
    }

    fn record(&self, event: impl Into<String>) {
        self.trace.lock().expect("trace").push(event.into());
    }
}

impl ControlWindowActions for TestWindows {
    fn destroy(&self, label: &str) {
        self.record("destroy");
        self.attempts
            .lock()
            .expect("attempts")
            .push(label.to_string());
        let mut existing = self.existing.lock().expect("existing");
        let named = self.named_alive.lock().expect("named alive").remove(label);
        if *existing || named {
            *existing = false;
            self.destroyed
                .lock()
                .expect("destroyed")
                .push(label.to_string());
        }
    }

    fn hide(&self, label: &str) -> Result<(), LongshotIpcError> {
        self.record("hide");
        self.hidden.lock().expect("hidden").push(label.to_string());
        if self.hide_fails.load(Ordering::SeqCst) {
            Err(LongshotIpcError::new(
                "longshot_controller_hide_failed",
                "injected hide failure",
            ))
        } else {
            Ok(())
        }
    }

    fn show(&self, label: &str) -> Result<(), LongshotIpcError> {
        self.record("show");
        self.shown.lock().expect("shown").push(label.to_string());
        if self.show_fails.load(Ordering::SeqCst) {
            Err(LongshotIpcError::new(
                "longshot_controller_show_failed",
                "injected show failure",
            ))
        } else {
            Ok(())
        }
    }

    fn focus(&self, label: &str) {
        self.record("focus");
        self.focused
            .lock()
            .expect("focused")
            .push(label.to_string());
    }

    fn exists(&self, label: &str) -> bool {
        *self.existing.lock().expect("existing")
            || self
                .named_alive
                .lock()
                .expect("named alive")
                .contains(label)
    }
}

struct DestroyOnShowWindows<'a> {
    registry: &'a LongshotControllerRegistry,
    label: &'a str,
    inner: TestWindows,
}

impl ControlWindowActions for DestroyOnShowWindows<'_> {
    fn destroy(&self, label: &str) {
        self.inner.destroy(label);
    }

    fn hide(&self, label: &str) -> Result<(), LongshotIpcError> {
        self.inner.hide(label)
    }

    fn show(&self, label: &str) -> Result<(), LongshotIpcError> {
        self.inner.show(label)?;
        let _ = self.registry.claim_destroyed(self.label);
        Ok(())
    }

    fn focus(&self, label: &str) {
        self.inner.focus(label);
    }

    fn exists(&self, label: &str) -> bool {
        self.inner.exists(label)
    }
}

fn pending_registry() -> (LongshotControllerRegistry, String) {
    let registry = LongshotControllerRegistry::new();
    let label = registry
        .reserve("capture-overlay-a-7".to_string(), selection())
        .expect("reserve");
    assert!(registry.publish_started(&label, CONTROLLER_PAGE));
    (registry, label)
}

fn failed_registry() -> (LongshotControllerRegistry, String) {
    let (registry, label) = pending_registry();
    registry.claim_activation(&label).expect("claim");
    let _ = registry.complete_activation(&label, Err(CaptureError::SessionMissing));
    (registry, label)
}

fn ordinary_fixture() -> (
    CaptureManager,
    Arc<CaptureModeGate>,
    String,
    String,
    CaptureSelection,
) {
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
    let selection = CaptureSelection {
        session_id: start.session_id.clone(),
        monitor_id: 7,
        x: 1.0,
        y: 2.0,
        width: 30.0,
        height: 40.0,
    };
    (manager, gate, start.session_id, caller, selection)
}

fn assert_and_finish_ordinary(
    manager: &CaptureManager,
    gate: &CaptureModeGate,
    session_id: &str,
    caller: &str,
) {
    assert!(manager.payload(caller).is_ok());
    assert_eq!(
        gate.active_mode().expect("gate"),
        Some(CaptureMode::Ordinary)
    );
    let session = manager.finish(session_id).expect("ordinary intact");
    assert_eq!(session.restore_labels, vec!["main"]);
    assert_eq!(session.lowered_pins, vec!["pin-a"]);
    session.finalize_mode().expect("release");
}

fn start(generation: u64) -> super::super::LongshotStart {
    super::super::LongshotStart {
        token: LongshotSessionToken::from_wire_parts("longshot".to_string(), generation),
        snapshot: LongshotSnapshot {
            frame_count: 1,
            width: 30,
            frame_height: 40,
            total_height: 40,
        },
    }
}

fn active_registry() -> (LongshotControllerRegistry, String, LongshotSessionToken) {
    let registry = LongshotControllerRegistry::new();
    let label = registry
        .reserve("capture-overlay-a-7".to_string(), selection())
        .expect("reserve");
    assert!(registry.publish_started(&label, CONTROLLER_PAGE));
    registry.claim_activation(&label).expect("claim");
    let (activation, compensation) = registry
        .complete_activation(&label, Ok(start(9)))
        .expect("complete");
    assert!(activation.is_some());
    assert!(compensation.is_none());
    let token = LongshotSessionToken::from_wire_parts("longshot".to_string(), 9);
    (registry, label, token)
}

fn revealed_active_registry() -> (LongshotControllerRegistry, String, LongshotSessionToken) {
    let (registry, label, token) = active_registry();
    assert!(matches!(
        registry.claim_ready(&label),
        Ok(ReadyAction::ShowActive(_))
    ));
    (registry, label, token)
}

fn selection() -> CaptureSelection {
    CaptureSelection {
        session_id: "ordinary".to_string(),
        monitor_id: 7,
        x: 1.0,
        y: 2.0,
        width: 30.0,
        height: 40.0,
    }
}

fn lifecycle_artifact(png: Vec<u8>) -> LongshotArtifact {
    LongshotArtifact {
        png,
        origin: test_origin(),
    }
}

fn test_origin() -> PinOrigin {
    PinOrigin {
        x: -12.5,
        y: 8.25,
        width: 320.5,
        height: 640.75,
    }
}

fn output_artifact(png: Vec<u8>) -> Arc<LongshotOutputArtifact> {
    Arc::new(LongshotOutputArtifact::from(lifecycle_artifact(png)))
}

mod activation;
mod append;
mod cancel;
mod finish;
mod preview;
mod registry;
