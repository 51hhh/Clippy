//! 只把原生快照数据送入生产规划；不创建窗口或枚举桌面。
use super::*;
use crate::pin::{PinFingerprint, PinOriginRegistry, PinPhysicalOrigin};

fn monitor(x: i32, y: i32, w: u32, h: u32, scale: f64, primary: bool) -> NativeMonitor {
    NativeMonitor {
        bounds: PhysicalMonitorBounds {
            x,
            y,
            width: w,
            height: h,
        },
        work: PhysicalRect {
            position: PhysicalPosition::new(x, y),
            size: PhysicalSize::new(w, h),
        },
        scale,
        primary,
    }
}

fn origin(m: NativeMonitor, x: f64, y: f64) -> PinOrigin {
    PinOrigin {
        x: x / m.scale,
        y: y / m.scale,
        width: 80.0,
        height: 60.0,
        physical: Some(PinPhysicalOrigin {
            monitor: m.bounds,
            x,
            y,
        }),
    }
}

fn physical(
    layout: ImageLayout,
    stage: PlacementStage,
) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    let (position, size) = layout
        .native
        .unwrap()
        .requests(layout.width, layout.height, 1.0, stage);
    match (position, size) {
        (Position::Physical(p), Size::Physical(s)) => (p, s),
        other => panic!("Windows 请求必须保留物理单位: {other:?}"),
    }
}

#[test]
fn windows_pin_origin_mixed_100_150_uses_source_in_either_enumeration_order() {
    let a = monitor(0, 0, 2560, 1440, 1.0, true);
    let b = monitor(2560, 0, 1920, 1080, 1.5, false);
    for monitors in [[a, b], [b, a]] {
        let layout = plan_image(
            &monitors,
            Some(PhysicalPosition::new(100.0, 100.0)),
            (300.0, 150.0),
            Some(origin(b, 2860.0, 150.0)),
        );
        assert_eq!(layout.scale, 1.5);
        assert_eq!((layout.width, layout.height), (200.0, 100.0));
        assert_eq!(layout.native.unwrap().monitor.bounds, b.bounds);
        assert_eq!(
            physical(layout, PlacementStage::Create).0,
            PhysicalPosition::new(2842, 132)
        );
    }
}

#[test]
fn windows_pin_origin_mixed_150_100_does_not_use_primary_dpi() {
    let a = monitor(0, 0, 2880, 1620, 1.5, true);
    let b = monitor(2880, 0, 1920, 1080, 1.0, false);
    let layout = plan_image(
        &[a, b],
        None,
        (300.0, 150.0),
        Some(origin(b, 3100.0, 180.0)),
    );
    assert_eq!(layout.scale, 1.0);
    assert_eq!((layout.width, layout.height), (300.0, 150.0));
    assert_eq!(
        physical(layout, PlacementStage::Reveal).0,
        PhysicalPosition::new(3088, 168)
    );
}

#[test]
fn windows_pin_origin_nondivisible_negative_vertical_origins_keep_raw_pixels() {
    for (x, y, scale) in [
        (-2560, -1250, 1.5),
        (2560, 1250, 4.0 / 3.0),
        (0, -1440, 1.25),
    ] {
        let m = monitor(x, y, 1920, 1080, scale, true);
        let layout = plan_image(
            &[m],
            None,
            (150.0, 90.0),
            Some(origin(m, f64::from(x) + 120.0, f64::from(y) + 100.0)),
        );
        assert_eq!(layout.width, 150.0 / scale);
        assert_eq!(
            physical(layout, PlacementStage::Create).0,
            PhysicalPosition::new(
                x + (120.0 - 12.0 * scale).round() as i32,
                y + (100.0 - 12.0 * scale).round() as i32
            )
        );
    }
}

#[test]
fn windows_pin_origin_actual_png_dimensions_keep_rotation_aspect_and_tiny_pixels() {
    let m = monitor(0, 0, 1920, 1080, 1.5, true);
    for pixels in [(90.0, 300.0), (1.0, 1.0)] {
        let layout = plan_image(&[m], None, pixels, Some(origin(m, 100.0, 100.0)));
        assert_eq!(
            (layout.width, layout.height),
            (pixels.0 / 1.5, pixels.1 / 1.5)
        );
        assert!(physical(layout, PlacementStage::Reveal).1.height >= 552);
    }
}

#[test]
fn windows_pin_origin_unknown_or_removed_source_uses_cursor_without_logical_guess() {
    let a = monitor(0, 0, 2560, 1440, 1.0, true);
    let b = monitor(2560, 0, 1920, 1080, 1.5, false);
    let removed = monitor(-1920, 0, 1920, 1080, 1.0, false);
    let mut unknown = origin(a, 100.0, 100.0);
    unknown.physical = None;
    for source in [None, Some(unknown), Some(origin(removed, -1800.0, 100.0))] {
        let layout = plan_image(
            &[a, b],
            Some(PhysicalPosition::new(3000.0, 100.0)),
            (300.0, 150.0),
            source,
        );
        assert_eq!(layout.scale, 1.5);
        assert_eq!(layout.native.unwrap().monitor.bounds, b.bounds);
        assert_eq!(
            physical(layout, PlacementStage::Create).0,
            PhysicalPosition::new(3012, 112)
        );
    }
}

#[test]
fn windows_pin_origin_invalid_snapshot_or_provenance_has_safe_fallback() {
    let good = monitor(0, 0, 1920, 1080, 1.25, true);
    let mut bad = good;
    bad.scale = f64::NAN;
    let mut source = origin(good, 120.0, 100.0);
    source.physical.as_mut().unwrap().x = f64::NAN;
    let layout = plan_image(&[bad, good], None, (300.0, 150.0), Some(source));
    assert_eq!(layout.scale, 1.25);
    assert_eq!(layout.native.unwrap().monitor.bounds, good.bounds);
    physical(layout, PlacementStage::Reveal);
    let missing = plan_image(&[], None, (300.0, 150.0), None);
    assert!(missing.native.is_none());
    assert_eq!(missing.scale, 1.0);
}

#[test]
fn windows_pin_origin_create_reveal_ignore_current_window_dpi_and_clamp_work_area() {
    let mut m = monitor(-2560, 1250, 1920, 1080, 1.5, true);
    m.work.position.y += 40;
    m.work.size.height -= 80;
    let layout = plan_image(
        &[m],
        None,
        (3000.0, 2000.0),
        Some(origin(m, -700.0, 2250.0)),
    );
    let created = physical(layout, PlacementStage::Create);
    let revealed = physical(layout, PlacementStage::Reveal);
    assert_eq!(created, revealed);
    assert!(created.1.width <= m.work.size.width && created.1.height <= m.work.size.height);
    for current_dpi in [1.0, 1.25, 2.0] {
        let request = layout.native.unwrap().requests(
            layout.width,
            layout.height,
            1.0,
            PlacementStage::Reveal,
        );
        assert_eq!(request.0.to_physical::<i32>(current_dpi), created.0);
        assert_eq!(request.1.to_physical::<u32>(current_dpi), created.1);
    }
}

#[test]
fn windows_pin_origin_union_anchor_may_leave_source_but_never_changes_display() {
    let a = monitor(0, 0, 2560, 1440, 1.0, true);
    let b = monitor(2560, 0, 1920, 1080, 1.5, false);
    let layout = plan_image(
        &[a, b],
        None,
        (900.0, 600.0),
        Some(origin(b, 2300.0, -100.0)),
    );
    assert_eq!(layout.native.unwrap().monitor.bounds, b.bounds);
    assert_eq!(
        physical(layout, PlacementStage::Create).0,
        PhysicalPosition::new(2560, 0)
    );
}

#[test]
fn windows_pin_origin_ipc_cannot_forge_physical_provenance() {
    let m = monitor(2560, 0, 1920, 1080, 1.5, false);
    let source = origin(m, 2860.0, 150.0);
    let value = serde_json::to_value(source).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 4);
    let mut forged = value;
    forged["physical"] =
        serde_json::json!({"x":2860,"y":150,"monitor":{"x":2560,"y":0,"width":1920,"height":1080}});
    forged["physicalBounds"] = forged["physical"].clone();
    let received: PinOrigin = serde_json::from_value(forged).unwrap();
    assert!(received.physical.is_none());
}

#[test]
fn windows_pin_origin_registry_preserves_backend_source_after_real_png_reencoding() {
    use image::ImageEncoder;
    let m = monitor(-2560, 1250, 1920, 1080, 1.5, true);
    let source = origin(m, -2440.0, 1350.0);
    let rgba = [1, 2, 3, 255, 4, 5, 6, 128, 0, 0, 0, 0, 8, 9, 10, 255];
    let png = crate::screenshot::encode_png(&rgba, 2, 2).unwrap();
    let decoded = image::load_from_memory(&png).unwrap().into_rgba8();
    let mut reencoded = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut reencoded,
        image::codecs::png::CompressionType::Best,
        image::codecs::png::FilterType::NoFilter,
    )
    .write_image(decoded.as_raw(), 2, 2, image::ExtendedColorType::Rgba8)
    .unwrap();
    assert_ne!(png, reencoded);
    let registry = PinOriginRegistry::default();
    registry.remember(PinFingerprint::of(2, 2, &rgba), source);
    assert_eq!(registry.lookup(&reencoded), Some(source));
    assert!(registry.lookup(b"not png").is_none());
}
