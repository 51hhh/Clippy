//! 实际 reveal/guide 的原始物理原点回归，不执行窗口枚举或输入。

use super::windows_focus_tests::{frame, manager_with_frames, reveal_pair};
use super::*;
use crate::screenshot::PhysicalMonitorBounds;

fn positive() -> Vec<CapturedMonitorFrame> {
    let primary = frame(1, 0, 0, 2560, 1080, 1.0, 1.0);
    let mut secondary = frame(2, 1707, 0, 1280, 720, 1.5, 1.5);
    secondary.physical_bounds = Some(PhysicalMonitorBounds {
        x: 2560,
        y: 0,
        width: 1920,
        height: 1080,
    });
    vec![primary, secondary]
}

#[test]
fn non_divisible_positive_origin_uses_exact_physical_half_open_edges() {
    for (x, owner) in [
        (2559.99, 0),
        (2560.0, 1),
        (2560.25, 1),
        (4479.99, 1),
        (4480.0, 0),
    ] {
        // 最后一项不命中任何屏，沿用首先就绪的主屏。
        let manager = manager_with_frames(positive(), CaptureIntent::Screenshot);
        reveal_pair(&manager, Some((x, 300.0)), 0, owner);
    }
}

#[test]
fn non_divisible_negative_and_vertical_origins_keep_the_native_edge() {
    let mut negative = frame(2, -1707, 0, 1280, 720, 1.5, 1.5);
    negative.physical_bounds = Some(PhysicalMonitorBounds {
        x: -2560,
        y: 0,
        width: 1920,
        height: 1080,
    });
    let manager = manager_with_frames(
        vec![frame(1, 0, 0, 1920, 1080, 1.0, 1.0), negative],
        CaptureIntent::Screenshot,
    );
    reveal_pair(&manager, Some((-640.25, 100.0)), 0, 1);

    let mut above = frame(2, 0, -1001, 1280, 800, 1.5, 1.25);
    above.physical_bounds = Some(PhysicalMonitorBounds {
        x: 0,
        y: -1251,
        width: 1920,
        height: 1000,
    });
    let manager = manager_with_frames(
        vec![frame(1, 0, 0, 1920, 1080, 1.0, 1.0), above],
        CaptureIntent::Recording,
    );
    reveal_pair(&manager, Some((100.0, -251.1)), 0, 1);
}

#[test]
fn missing_mismatched_and_overflowing_frame_bounds_use_ready_fallback() {
    for bounds in [
        None,
        Some(PhysicalMonitorBounds {
            width: 1919,
            ..positive()[1].physical_bounds.unwrap()
        }),
        Some(PhysicalMonitorBounds {
            height: 0,
            ..positive()[1].physical_bounds.unwrap()
        }),
        Some(PhysicalMonitorBounds {
            x: i32::MAX,
            ..positive()[1].physical_bounds.unwrap()
        }),
    ] {
        let mut frames = positive();
        frames[1].physical_bounds = bounds;
        let manager = manager_with_frames(frames, CaptureIntent::Screenshot);
        reveal_pair(&manager, Some((3000.0, 300.0)), 0, 0);
    }
}

#[test]
fn guide_preserves_the_caller_frames_physical_bounds() {
    let manager = manager_with_frames(positive(), CaptureIntent::Screenshot);
    let guide = manager
        .longshot_guide_spec(
            "capture-overlay-fixture-2",
            &CaptureSelection {
                session_id: "fixture".to_string(),
                monitor_id: 2,
                x: 1.1,
                y: 2.2,
                width: 30.0,
                height: 30.0,
            },
        )
        .unwrap();
    assert_eq!(
        guide.physical_bounds,
        Some(PhysicalMonitorBounds {
            x: 2560,
            y: 0,
            width: 1920,
            height: 1080
        })
    );
    assert_eq!((guide.monitor_x, guide.monitor_width), (1707, 1280));
    assert_eq!(guide.selection_x, 1.0 / 1.5);
    assert_eq!(guide.selection_y, 2.0);
}
