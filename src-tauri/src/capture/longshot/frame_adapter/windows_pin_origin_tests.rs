use super::tests::{encoded_frame, selection};
use super::*;
use crate::screenshot::PhysicalMonitorBounds;

#[test]
fn union_origin_adds_signed_offsets_to_raw_crop_on_both_axes() {
    for (x, y) in [(2560, 1250), (-2560, -1250)] {
        let mut frame = encoded_frame(x / 2, y / 2, 128, 100, 192, 200, 1.5, 2.0, 7);
        frame.physical_bounds = Some(PhysicalMonitorBounds {
            x,
            y,
            width: 192,
            height: 200,
        });
        let (adapter, _) =
            LongshotFrameAdapter::from_first(&frame, &selection(2.2, 3.2, 20.0, 24.0)).unwrap();
        let source = adapter
            .output_origin_with_offset(100, 150, -40, -70)
            .unwrap();
        let physical = source.physical.unwrap();
        assert_eq!(physical.monitor, frame.physical_bounds.unwrap());
        assert_eq!(
            (physical.x, physical.y),
            (f64::from(x) + 3.0 - 40.0, f64::from(y) + 6.0 - 70.0)
        );
    }
}

#[test]
fn impossible_native_union_anchor_is_rejected_without_saturating_i32() {
    let mut frame = encoded_frame(0, 0, 128, 100, 192, 150, 1.5, 1.5, 7);
    frame.physical_bounds.as_mut().unwrap().x = i32::MIN;
    let (adapter, _) =
        LongshotFrameAdapter::from_first(&frame, &selection(0.0, 0.0, 20.0, 24.0)).unwrap();
    assert!(matches!(
        adapter.output_origin_with_offset(100, 100, -50, -50),
        Err(CaptureError::LongshotFrameInvalid)
    ));
}
