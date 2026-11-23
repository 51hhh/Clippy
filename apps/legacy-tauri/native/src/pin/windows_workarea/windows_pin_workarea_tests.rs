//! 原生领域快照驱动真实保存/恢复/客户区边界入口；不枚举桌面或创建窗口。
use super::super::windows_geometry::PlacementStage;
use super::*;
use crate::screenshot::PhysicalMonitorBounds;
use tauri::{PhysicalRect, Position, Size};

fn monitor(
    name: &str,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f64,
    primary: bool,
) -> NamedMonitor {
    NamedMonitor {
        name: Some(name.into()),
        native: NativeMonitor {
            bounds: PhysicalMonitorBounds {
                x,
                y,
                width,
                height,
            },
            work: PhysicalRect {
                position: PhysicalPosition::new(x, y),
                size: PhysicalSize::new(width, height),
            },
            scale,
            primary,
        },
    }
}

fn mixed() -> [NamedMonitor; 2] {
    [
        monitor("Primary", 0, 0, 2560, 1440, 1.0, true),
        monitor("Secondary", 2560, 0, 1920, 1080, 1.5, false),
    ]
}
fn snapshot(owner: &NamedMonitor, x: i32, y: i32) -> WindowSnapshot {
    WindowSnapshot {
        outer_position: Some(PhysicalPosition::new(x, y)),
        outer_size: Some(PhysicalSize::new(400, 600)),
        client_position: Some(PhysicalPosition::new(x, y)),
        client_size: Some(PhysicalSize::new(400, 600)),
        scale: Some(owner.native.scale),
        owner: Some(owner.clone()),
    }
}
fn saved() -> StoredPinPlacement {
    StoredPinPlacement {
        x: 2860.0 / 1.5,
        y: 150.0 / 1.5,
        display_name: Some("Secondary".into()),
        display_x: 2560.0 / 1.5,
        display_y: 0.0,
        display_width: 1920.0 / 1.5,
        display_height: 1080.0 / 1.5,
        display_scale: 1.5,
    }
}
fn request(
    layout: WorkspaceLayout,
    stage: PlacementStage,
) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    let native = layout.native.expect("恢复选择的显示器必须贯穿到原生请求");
    let (position, size) = native.requests(200.0, 100.0, 1.0, stage);
    match (position, size) {
        (Position::Physical(p), Size::Physical(s)) => (p, s),
        other => panic!("工作区请求必须是物理类型: {other:?}"),
    }
}
fn assert_bounds(actual: ToolbarBounds, expected: (f64, f64, f64, f64)) {
    for (actual, expected) in [
        (actual.x, expected.0),
        (actual.y, expected.1),
        (actual.width, expected.2),
        (actual.height, expected.3),
    ] {
        assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
    }
}

#[test]
fn capture_mixed_owner_does_not_follow_logical_overlap_or_enumeration_order() {
    let [a, b] = mixed();
    let window = snapshot(&b, 2860, 150);
    for monitors in [[a.clone(), b.clone()], [b.clone(), a.clone()]] {
        assert_eq!(capture_placement(&window, &monitors), Some(saved()));
    }
}

#[test]
fn capture_negative_vertical_taskbar_and_lagging_window_dpi_uses_one_owner_scale() {
    let mut m = monitor("Left", -2560, -1440, 1920, 1080, 1.5, true);
    m.native.work.position.y += 40;
    m.native.work.size.height -= 80;
    let mut window = snapshot(&m, -2440, -1300);
    window.scale = Some(1.0);
    let record = capture_placement(&window, &[m.clone()]).unwrap();
    assert_eq!(record.x, -2440.0 / 1.5);
    assert_eq!(record.y, -1300.0 / 1.5);
    assert_eq!(record.display_y, -1400.0 / 1.5);
    assert_eq!(record.display_height, 1000.0 / 1.5);
    assert_eq!(record.display_scale, 1.5);
}

#[test]
fn capture_missing_owner_uses_largest_physical_window_intersection() {
    let [a, b] = mixed();
    let mut window = snapshot(&b, 2500, 100);
    window.owner = None;
    let record = capture_placement(&window, &[a, b]).unwrap();
    assert_eq!(record.display_name.as_deref(), Some("Secondary"));
    assert_eq!(record.x, 2500.0 / 1.5);
}

#[test]
fn capture_missing_position_or_invalid_display_does_not_invent_metadata() {
    let [_, mut b] = mixed();
    let mut window = snapshot(&b, 2860, 150);
    window.outer_position = None;
    assert!(capture_placement(&window, &[b.clone()]).is_none());
    window.outer_position = Some(PhysicalPosition::new(2860, 150));
    b.native.work.size.width = 0;
    window.owner = Some(b.clone());
    assert!(capture_placement(&window, &[b]).is_none());
}

#[test]
fn sqlite_roundtrip_then_real_layout_application_preserves_saved_presentation() {
    use super::super::model::{PinSource, SharpenSlot};
    use crate::models::{ClipItem, ContentType};
    use crate::storage::{PinWorkspaceItemWrite, StorageEngine};
    use std::sync::Arc;
    let monitors = mixed();
    let placement = capture_placement(&snapshot(&monitors[1], 2860, 150), &monitors).unwrap();
    let storage = StorageEngine::new_in_memory().unwrap();
    let id = storage
        .upsert_pin_workspace_item(PinWorkspaceItemWrite {
            id: None,
            group_id: None,
            revision_id: None,
            content_type: ContentType::Text,
            text_content: Some("saved pin"),
            html_content: None,
            content_hash: "pin-workarea",
            content_width: 200.0,
            content_height: 100.0,
            scale: 0.75,
            opacity: 0.8,
            locked: true,
            above: true,
            placement: Some(&placement),
        })
        .unwrap();
    let item = storage.load_pin_workspace_item(id).unwrap().unwrap();
    assert_eq!(item.placement, Some(saved()));
    let source = Arc::new(PinSource::Clip {
        item: ClipItem {
            id: -id,
            content_type: ContentType::Text,
            text_content: Some("saved pin".into()),
            html_content: None,
            image_data: None,
            content_hash: "pin-workarea".into(),
            is_favorite: false,
            is_sensitive: false,
            created_at: 0,
            byte_size: 0,
        },
        image: None,
    });
    let mut entry = PinEntry {
        label: format!("pin-workspace-{id}"),
        source: source.clone(),
        content_width: item.content_width,
        content_height: item.content_height,
        scale: item.scale,
        opacity: item.opacity,
        locked: item.locked,
        above: item.above,
        workspace_id: Some(id),
        workspace_group_id: None,
        position: None,
        restore_position: None,
        origin: None,
        native_layout: None,
        device_scale: 1.0,
        buffer_scale: 1.0,
        sharpen: Arc::new(SharpenSlot::default()),
    };
    let outer =
        super::super::window::outer_size(entry.content_width, entry.content_height, entry.scale);
    restore_layout(&monitors, None, item.placement.as_ref(), outer).apply(&mut entry);
    assert_eq!((entry.device_scale, entry.buffer_scale), (1.5, 1.5));
    assert!(entry.restore_position.is_none());
    assert!(entry.native_layout.is_some());
    assert!(Arc::ptr_eq(&source, &entry.source));
    assert_eq!(
        (entry.scale, entry.opacity, entry.locked, entry.above),
        (0.75, 0.8, true, true)
    );
    assert_eq!((entry.content_width, entry.content_height), (200.0, 100.0));
    assert_eq!(entry.workspace_id, Some(id));
}

#[test]
fn legacy_normal_record_keeps_chosen_display_in_either_enumeration_order() {
    let [a, b] = mixed();
    for monitors in [[a.clone(), b.clone()], [b.clone(), a.clone()]] {
        let layout = restore_layout(&monitors, None, Some(&saved()), (268.0, 368.0));
        assert_eq!(layout.scale, 1.5);
        assert!(layout.restore_position.is_none());
        assert_eq!(
            request(layout, PlacementStage::Create).0,
            PhysicalPosition::new(2860, 150)
        );
    }
}

#[test]
fn renamed_display_matches_saved_physical_reference_even_after_dpi_change() {
    let [a, mut b] = mixed();
    b.name = Some("Renamed".into());
    b.native.scale = 2.0;
    let layout = restore_layout(&[a, b.clone()], None, Some(&saved()), (268.0, 368.0));
    assert_eq!(layout.scale, 2.0);
    assert_eq!(layout.native.unwrap().monitor.bounds, b.native.bounds);
    assert_eq!(
        request(layout, PlacementStage::Reveal).0,
        PhysicalPosition::new(2860, 150)
    );
}

#[test]
fn moved_negative_display_preserves_saved_fraction_and_current_dpi() {
    let [a, _] = mixed();
    let b = monitor("Secondary", -2560, -1440, 2560, 1440, 2.0, false);
    let layout = restore_layout(&[a, b], None, Some(&saved()), (268.0, 368.0));
    assert_eq!(layout.scale, 2.0);
    assert_eq!(
        request(layout, PlacementStage::Create).0,
        PhysicalPosition::new(-2160, -1240)
    );
}

#[test]
fn removed_display_falls_back_to_primary_and_clamps_oversized_window() {
    let a = monitor("Primary", 0, 40, 1366, 736, 1.25, true);
    let layout = restore_layout(&[a], None, Some(&saved()), (2000.0, 1600.0));
    assert_eq!(layout.scale, 1.25);
    assert!(layout.restore_position.is_none());
    let native = layout.native.unwrap();
    let (position, size) = native.requests(2000.0, 1600.0, 1.0, PlacementStage::Reveal);
    assert_eq!(
        position.to_physical::<i32>(2.0),
        PhysicalPosition::new(0, 40)
    );
    assert_eq!(size.to_physical::<u32>(2.0), PhysicalSize::new(1366, 736));
}

#[test]
fn no_record_or_invalid_record_uses_cursor_and_no_monitor_avoids_logical_guess() {
    let monitors = mixed();
    let mut bad = saved();
    bad.display_width = 0.0;
    for record in [None, Some(&bad)] {
        let layout = restore_layout(
            &monitors,
            Some(PhysicalPosition::new(3000.0, 100.0)),
            record,
            (268.0, 368.0),
        );
        assert_eq!(layout.scale, 1.5);
        assert!(layout.restore_position.is_none());
        assert_eq!(
            request(layout, PlacementStage::Create).0,
            PhysicalPosition::new(3012, 112)
        );
    }
    let missing = restore_layout(&[], None, Some(&saved()), (268.0, 368.0));
    assert!(missing.native.is_none());
    assert!(missing.restore_position.is_none());
    assert_eq!(missing.scale, 1.0);
}

#[test]
fn create_and_reveal_requests_do_not_reinterpret_saved_position_at_current_dpi() {
    let layout = restore_layout(&mixed(), None, Some(&saved()), (268.0, 368.0));
    assert_eq!(
        request(layout, PlacementStage::Create),
        request(layout, PlacementStage::Reveal)
    );
    for stage in [PlacementStage::Create, PlacementStage::Reveal] {
        let (p, s) = layout.native.unwrap().requests(200.0, 100.0, 1.0, stage);
        for dpi in [1.0, 1.25, 2.0] {
            assert_eq!(p.to_physical::<i32>(dpi), PhysicalPosition::new(2860, 150));
            assert_eq!(s.to_physical::<u32>(dpi), PhysicalSize::new(402, 552));
        }
    }
}

#[test]
fn toolbar_mixed_overlap_uses_client_rectangle_and_native_owner() {
    let monitors = mixed();
    let mut window = snapshot(&monitors[1], 2500, 0);
    window.outer_size = Some(PhysicalSize::new(600, 600));
    window.client_position = Some(PhysicalPosition::new(2508, 8));
    window.client_size = Some(PhysicalSize::new(584, 584));
    assert_bounds(
        toolbar_bounds(&window, &monitors),
        (52.0 / 1.5, 0.0, 532.0 / 1.5, 584.0 / 1.5),
    );
    let json = serde_json::to_value(toolbar_bounds(&window, &monitors)).unwrap();
    assert_eq!(json.as_object().unwrap().len(), 4);
}

#[test]
fn toolbar_intersection_uses_window_dpi_only_after_physical_taskbar_clip() {
    let mut m = monitor("Left", -1920, -1080, 1920, 1080, 2.0, true);
    m.native.work.position.y += 40;
    m.native.work.size.height -= 80;
    let mut window = snapshot(&m, -1800, -1060);
    window.scale = Some(1.25);
    window.client_size = Some(PhysicalSize::new(600, 600));
    assert_bounds(toolbar_bounds(&window, &[m]), (0.0, 16.0, 480.0, 464.0));
}

#[test]
fn toolbar_uniform_right_bottom_clip_keeps_existing_local_semantics() {
    let mut m = monitor("Primary", 0, 0, 1920, 1080, 1.25, true);
    m.native.work.position.y = 40;
    m.native.work.size.height = 1000;
    let mut window = snapshot(&m, 1750, 850);
    window.outer_size = Some(PhysicalSize::new(400, 300));
    window.client_size = window.outer_size;
    assert_bounds(toolbar_bounds(&window, &[m]), (0.0, 0.0, 136.0, 152.0));
}

#[test]
fn toolbar_missing_owner_uses_largest_physical_overlap_without_logical_first_match() {
    let monitors = mixed();
    let mut window = snapshot(&monitors[1], 2500, 100);
    window.owner = None;
    assert_bounds(
        toolbar_bounds(&window, &monitors),
        (60.0 / 1.5, 0.0, 340.0 / 1.5, 400.0),
    );
}

#[test]
fn toolbar_unknown_queries_fall_back_to_client_or_existing_unknown() {
    let monitors = mixed();
    let mut window = snapshot(&monitors[1], 2860, 150);
    window.client_size = Some(PhysicalSize::new(300, 450));
    window.client_position = None;
    assert_bounds(toolbar_bounds(&window, &monitors), (0.0, 0.0, 200.0, 300.0));
    window.client_position = Some(PhysicalPosition::new(100000, 100000));
    window.owner = None;
    assert_bounds(toolbar_bounds(&window, &[]), (0.0, 0.0, 200.0, 300.0));
    window.client_size = None;
    assert_eq!(toolbar_bounds(&window, &monitors), ToolbarBounds::UNKNOWN);
    window.client_size = Some(PhysicalSize::new(300, 450));
    window.scale = Some(f64::NAN);
    assert_eq!(toolbar_bounds(&window, &monitors), ToolbarBounds::UNKNOWN);
}
