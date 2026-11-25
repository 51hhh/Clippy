use super::windows_focus_tests::{frame, manager_with_frames};
use super::*;
use crate::screenshot::PhysicalMonitorBounds;

fn frozen() -> CapturedMonitorFrame {
    let mut frame = frame(7, -1707, 625, 600, 400, 1.5, 2.0);
    frame.physical_bounds = Some(PhysicalMonitorBounds {
        x: -2560,
        y: 1250,
        width: 900,
        height: 800,
    });
    frame
}
fn selection(x: f64, y: f64, width: f64, height: f64) -> CaptureSelection {
    CaptureSelection {
        session_id: "fixture".into(),
        monitor_id: 7,
        x,
        y,
        width,
        height,
    }
}

#[test]
fn ordinary_omitted_or_forged_origin_comes_from_bound_frame_and_rounded_crop() {
    for submitted in [
        None,
        Some(
            serde_json::from_value(
                serde_json::json!({"x":99999,"y":88888,"width":99,"height":88,"physical":{"x":1}}),
            )
            .unwrap(),
        ),
    ] {
        let manager = manager_with_frames(vec![frozen()], CaptureIntent::Screenshot);
        let claim = manager
            .claim_output(
                "capture-overlay-fixture-7",
                &selection(0.2, 0.2, 2.0, 2.0),
                submitted,
            )
            .unwrap();
        let source = claim.origin.expect("前端省略来源仍须保留冻结事实");
        let physical = source.physical.unwrap();
        assert_eq!(physical.monitor, frozen().physical_bounds.unwrap());
        assert_eq!((physical.x, physical.y), (-2560.0, 1250.0));
        assert_eq!((source.width, source.height), (4.0 / 1.5, 5.0 / 2.0));
    }
}

#[test]
fn clamped_left_and_bottom_edges_use_actual_crop_not_requested_rect() {
    for (select, position, dims) in [
        (
            selection(-2.0, -3.0, 5.0, 6.0),
            (-2560.0, 1250.0),
            (5.0 / 1.5, 3.0),
        ),
        (
            selection(598.8, 398.8, 3.0, 3.0),
            (-1662.0, 2047.0),
            (2.0 / 1.5, 1.5),
        ),
    ] {
        let manager = manager_with_frames(vec![frozen()], CaptureIntent::Screenshot);
        let source = manager
            .claim_output("capture-overlay-fixture-7", &select, None)
            .unwrap()
            .origin
            .unwrap();
        let physical = source.physical.unwrap();
        assert_eq!((physical.x, physical.y), position);
        assert_eq!((source.width, source.height), dims);
        assert!(source.sanitized().is_some(), "原生 crop 很小也不能丢来源");
    }
}

#[test]
fn output_retry_retains_same_native_source_and_rendered_artifact() {
    let manager = manager_with_frames(vec![frozen()], CaptureIntent::Screenshot);
    let claim = manager
        .claim_output(
            "capture-overlay-fixture-7",
            &selection(20.0, 30.0, 30.0, 40.0),
            None,
        )
        .unwrap();
    assert!(claim.origin.unwrap().physical.is_some());
    let png = crate::screenshot::encode_png(&[20, 30, 40, 255].repeat(4), 2, 2).unwrap();
    let artifact = Arc::new(crate::capture::commit_image_from_png(png).unwrap());
    manager.publish_output(&claim, artifact.clone()).unwrap();
    manager.settle_output(&claim, false, false).unwrap();
    let retry = manager
        .retry_output("capture-overlay-fixture-7", CaptureAction::Pin)
        .unwrap();
    assert_eq!(retry.origin, claim.origin);
    assert!(Arc::ptr_eq(retry.artifact.as_ref().unwrap(), &artifact));
}

#[test]
fn missing_or_inconsistent_frozen_authority_is_rejected_before_claim() {
    for bounds in [
        None,
        Some(PhysicalMonitorBounds {
            x: -2560,
            y: 1250,
            width: 899,
            height: 800,
        }),
    ] {
        let mut frame = frozen();
        frame.physical_bounds = bounds;
        let manager = manager_with_frames(vec![frame], CaptureIntent::Screenshot);
        assert!(manager
            .claim_output(
                "capture-overlay-fixture-7",
                &selection(20.0, 30.0, 30.0, 40.0),
                None
            )
            .is_err());
        assert!(manager
            .session
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .output
            .is_none());
    }
}

#[test]
fn source_is_never_derived_from_another_callers_frame() {
    let other = frame(8, 0, 0, 100, 100, 1.0, 1.0);
    let manager = manager_with_frames(vec![frozen(), other], CaptureIntent::Screenshot);
    assert!(matches!(
        manager.claim_output(
            "capture-overlay-fixture-8",
            &selection(20.0, 30.0, 30.0, 40.0),
            None
        ),
        Err(CaptureError::SelectionMonitorMismatch)
    ));
    assert!(manager
        .session
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .output
        .is_none());
}
