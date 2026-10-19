use super::tests::{encoded_frame, selection};
use super::*;
use crate::screenshot::PhysicalMonitorBounds;

fn frame(x: i32, y: i32, logical_x: i32, logical_y: i32) -> CapturedMonitorFrame {
    let mut frame = encoded_frame(logical_x, logical_y, 128, 100, 192, 150, 1.5, 1.5, 7);
    frame.physical_bounds = Some(PhysicalMonitorBounds {
        x,
        y,
        width: 192,
        height: 150,
    });
    frame
}

#[test]
fn scroll_target_adds_crop_center_to_original_physical_origin() {
    for (x, y, logical_x, logical_y, expected) in [
        (2560, 1250, 1707, 833, (2575, 1268)),
        (-2560, -1250, -1707, -833, (-2545, -1232)),
    ] {
        let (adapter, _) = LongshotFrameAdapter::from_first(
            &frame(x, y, logical_x, logical_y),
            &selection(0.0, 0.0, 20.0, 24.0),
        )
        .unwrap();
        assert_eq!(adapter.scroll_target().unwrap(), expected);
    }
}

#[test]
fn physical_origin_drift_rejects_recapture_even_if_logical_signature_is_unchanged() {
    let first = frame(2560, 1250, 1707, 833);
    let (adapter, image) =
        LongshotFrameAdapter::from_first(&first, &selection(0.0, 0.0, 20.0, 24.0)).unwrap();
    for (x, y) in [(2561, 1250), (2560, 1251)] {
        let next = frame(x, y, 1707, 833);
        assert!(matches!(
            adapter.crop_next(&next),
            Err(CaptureError::LongshotFrameGeometryChanged)
        ));
    }
    assert_eq!(adapter.crop_next(&first).unwrap(), image);
}

#[test]
fn missing_and_invalid_authority_is_rejected_before_first_crop() {
    for bounds in [
        None,
        Some(PhysicalMonitorBounds {
            x: 2560,
            y: 1250,
            width: 191,
            height: 150,
        }),
    ] {
        let mut first = frame(2560, 1250, 1707, 833);
        first.physical_bounds = bounds;
        assert!(matches!(
            LongshotFrameAdapter::from_first(&first, &selection(0.0, 0.0, 20.0, 24.0)),
            Err(CaptureError::LongshotFrameInvalid)
        ));
    }
}
