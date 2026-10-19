use super::windows_tests::{assert_candidate, candidates, frame};
use super::*;
use crate::screenshot::PhysicalMonitorBounds;

#[test]
fn non_divisible_origins_do_not_shift_window_local_edges() {
    for (x, logical_x) in [(2560, 1707), (-2560, -1707)] {
        let mut monitor = frame(7, logical_x, 0, 1280, 720, 1.5);
        monitor.physical_bounds = Some(PhysicalMonitorBounds {
            x,
            y: 0,
            width: 1920,
            height: 1080,
        });
        let result = candidates(
            &[monitor],
            ProbeRect {
                x,
                y: 150,
                width: 150,
                height: 150,
            },
        );
        assert_candidate(&result, 7, [0.0, 100.0, 100.0, 100.0]);
    }
}

#[test]
fn each_axis_subtracts_raw_origin_before_scaling_and_clipping() {
    let mut monitor = frame(7, 1707, -1001, 1280, 800, 1.5);
    monitor.scale_y = 1.25;
    monitor.pixel_height = 1000;
    monitor.physical_bounds = Some(PhysicalMonitorBounds {
        x: 2560,
        y: -1251,
        width: 1920,
        height: 1000,
    });
    let result = candidates(
        &[monitor],
        ProbeRect {
            x: 2545,
            y: -1261,
            width: 165,
            height: 135,
        },
    );
    assert_candidate(&result, 7, [0.0, 0.0, 100.0, 100.0]);
}

#[test]
fn missing_or_mismatched_authoritative_bounds_do_not_create_candidates() {
    for bounds in [
        None,
        Some(PhysicalMonitorBounds {
            x: 0,
            y: 0,
            width: 1919,
            height: 1080,
        }),
    ] {
        let mut monitor = frame(7, 0, 0, 1920, 1080, 1.0);
        monitor.physical_bounds = bounds;
        assert!(candidates(
            &[monitor],
            ProbeRect {
                x: 0,
                y: 0,
                width: 150,
                height: 150
            }
        )
        .is_empty());
    }
}
