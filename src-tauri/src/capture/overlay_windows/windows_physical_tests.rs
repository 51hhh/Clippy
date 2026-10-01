//! 锁定 Tauri/dpi 的真实坐标类型回归；不创建原生窗口。

use super::*;
use crate::screenshot::PhysicalMonitorBounds;

fn spec() -> OverlaySpec {
    OverlaySpec {
        label: "capture-overlay-native-bounds".to_string(),
        x: 1707,
        y: -833,
        width: 1280,
        height: 720,
        physical_bounds: Some(PhysicalMonitorBounds {
            x: 2560,
            y: -1250,
            width: 1920,
            height: 1080,
        }),
    }
}

#[test]
fn overlay_requests_exact_physical_bounds_at_every_window_dpi() {
    let (position, size) = overlay_geometry(&spec()).unwrap();
    assert!(matches!(position, tauri::Position::Physical(_)));
    assert!(matches!(size, tauri::Size::Physical(_)));
    for dpi in [1.0, 1.25, 1.5, 2.0] {
        assert_eq!(
            position.to_physical::<i32>(dpi),
            tauri::PhysicalPosition::new(2560, -1250)
        );
        assert_eq!(
            size.to_physical::<u32>(dpi),
            tauri::PhysicalSize::new(1920, 1080)
        );
    }
}

#[test]
fn guide_and_overlay_share_the_authoritative_physical_request() {
    let guide = super::super::manager::LongshotGuideSpec {
        monitor_x: 1707,
        monitor_y: -833,
        monitor_width: 1280,
        monitor_height: 720,
        physical_bounds: spec().physical_bounds,
        selection_x: 10.0,
        selection_y: 20.0,
        selection_width: 30.0,
        selection_height: 40.0,
    };
    let overlay = super::super::longshot::window_host::guide_overlay_spec(
        "longshot-guide-test".to_string(),
        guide,
    );
    assert_eq!(overlay.physical_bounds, spec().physical_bounds);
    let (position, size) = overlay_geometry(&overlay).unwrap();
    assert_eq!(
        position.to_physical::<i32>(1.0),
        tauri::PhysicalPosition::new(2560, -1250)
    );
    assert_eq!(
        size.to_physical::<u32>(2.0),
        tauri::PhysicalSize::new(1920, 1080)
    );
}

#[test]
fn missing_empty_or_overflowing_bounds_cannot_form_a_window_request() {
    for bounds in [
        None,
        Some(PhysicalMonitorBounds {
            width: 0,
            ..spec().physical_bounds.unwrap()
        }),
        Some(PhysicalMonitorBounds {
            height: 0,
            ..spec().physical_bounds.unwrap()
        }),
        Some(PhysicalMonitorBounds {
            x: i32::MAX,
            ..spec().physical_bounds.unwrap()
        }),
        Some(PhysicalMonitorBounds {
            y: i32::MAX,
            ..spec().physical_bounds.unwrap()
        }),
    ] {
        let mut invalid = spec();
        invalid.physical_bounds = bounds;
        assert!(overlay_geometry(&invalid).is_err());
    }
}
