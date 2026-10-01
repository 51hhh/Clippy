//! 直接运行生产 reveal 的 Windows 光标坐标合同；不枚举窗口或调用光标/输入 API。

use super::*;
use crate::capture::{CaptureMode, CaptureModeGate};

pub(super) fn frame(
    id: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    sx: f32,
    sy: f32,
) -> CapturedMonitorFrame {
    CapturedMonitorFrame {
        monitor_id: id,
        x,
        y,
        logical_width: width,
        logical_height: height,
        pixel_width: (width as f32 * sx).round() as u32,
        pixel_height: (height as f32 * sy).round() as u32,
        scale_x: sx,
        scale_y: sy,
        physical_bounds: Some(crate::screenshot::PhysicalMonitorBounds {
            x: (f64::from(x) * f64::from(sx)).round() as i32,
            y: (f64::from(y) * f64::from(sy)).round() as i32,
            width: (width as f32 * sx).round() as u32,
            height: (height as f32 * sy).round() as u32,
        }),
        rgba: Arc::from(Vec::new()),
    }
}

pub(super) fn manager_with_frames(
    frames: Vec<CapturedMonitorFrame>,
    intent: CaptureIntent,
) -> CaptureManager {
    let manager = CaptureManager::new();
    let prefix = match intent {
        CaptureIntent::Screenshot => "capture-overlay",
        CaptureIntent::Recording => "recording-overlay",
    };
    let overlays = frames
        .iter()
        .map(|frame| OverlaySpec {
            label: format!("{prefix}-fixture-{}", frame.monitor_id),
            x: frame.x,
            y: frame.y,
            width: frame.logical_width,
            #[cfg(target_os = "windows")]
            physical_bounds: frame.physical_bounds,
            height: frame.logical_height,
        })
        .collect();
    *manager.session.lock().unwrap() = Some(CaptureSession {
        id: "fixture".to_string(),
        intent,
        identity: Arc::new(()),
        overlays,
        frames,
        restore_labels: Vec::new(),
        lowered_pins: Vec::new(),
        windows: HashMap::new(),
        probe_hint: false,
        focus_assigned: false,
        timings: StageTimings::default(),
        output: None,
        mode_ownership: Arc::new(CaptureModeGate::new())
            .try_claim_owned(CaptureMode::Ordinary)
            .unwrap(),
    });
    manager
}

pub(super) fn reveal_pair(
    manager: &CaptureManager,
    cursor: Option<(f64, f64)>,
    first: usize,
    owner: usize,
) {
    let labels: Vec<_> = manager
        .session
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .overlays
        .iter()
        .map(|spec| spec.label.clone())
        .collect();
    assert_eq!(labels.len(), 2);
    for index in [first, 1 - first] {
        assert_eq!(
            manager
                .reveal(&labels[index], cursor, None)
                .unwrap()
                .take_focus,
            index == owner,
            "physical cursor {cursor:?}: overlay {index} vs owner {owner}"
        );
    }
}

fn uniform() -> Vec<CapturedMonitorFrame> {
    vec![
        frame(1, 0, 0, 1280, 720, 1.5, 1.5),
        frame(2, 1280, 0, 1280, 720, 1.5, 1.5),
    ]
}

#[test]
fn uniform_150_cursor_in_primary_does_not_focus_secondary_in_either_ready_order() {
    for first in 0..2 {
        let manager = manager_with_frames(uniform(), CaptureIntent::Screenshot);
        reveal_pair(&manager, Some((1800.0, 300.0)), first, 0);
    }
}

#[test]
fn mixed_100_150_and_150_100_layouts_use_each_frame_scale() {
    let manager = manager_with_frames(
        vec![
            frame(1, 0, 0, 1920, 1080, 1.0, 1.0),
            frame(2, 1280, 0, 1280, 720, 1.5, 1.5),
        ],
        CaptureIntent::Screenshot,
    );
    reveal_pair(&manager, Some((3000.0, 150.0)), 0, 1);
    let manager = manager_with_frames(
        vec![
            frame(1, 0, 0, 1280, 720, 1.5, 1.5),
            frame(2, 1920, 0, 1920, 1080, 1.0, 1.0),
        ],
        CaptureIntent::Screenshot,
    );
    reveal_pair(&manager, Some((1800.0, 150.0)), 1, 0);
}

#[test]
fn negative_and_vertical_layouts_keep_the_actual_cursor_owner() {
    let manager = manager_with_frames(
        vec![
            frame(1, -1280, 0, 1280, 720, 1.5, 1.5),
            frame(2, 0, 0, 1920, 1080, 1.0, 1.0),
        ],
        CaptureIntent::Screenshot,
    );
    reveal_pair(&manager, Some((-1800.0, 150.0)), 1, 0);
    let manager = manager_with_frames(
        vec![
            frame(1, 0, -864, 1536, 864, 1.25, 1.25),
            frame(2, 0, 0, 1280, 720, 1.5, 1.5),
        ],
        CaptureIntent::Screenshot,
    );
    reveal_pair(&manager, Some((150.0, -900.0)), 1, 0);
}

#[test]
fn anisotropic_factors_project_x_and_y_independently() {
    let manager = manager_with_frames(
        vec![
            frame(1, 0, 0, 1000, 1000, 2.0, 1.25),
            frame(2, 1000, 0, 1000, 1000, 2.0, 1.25),
        ],
        CaptureIntent::Screenshot,
    );
    reveal_pair(&manager, Some((1700.0, 1100.0)), 1, 0);
}

#[test]
fn physical_edges_include_left_top_and_exclude_right_bottom() {
    for (cursor, owner) in [
        ((0.0, 0.0), Some(0)),
        ((1919.99, 1079.99), Some(0)),
        ((1920.0, 0.0), Some(1)),
        ((3839.99, 1079.99), Some(1)),
        ((3840.0, 0.0), None),
        ((100.0, 1080.0), None),
    ] {
        let manager = manager_with_frames(uniform(), CaptureIntent::Screenshot);
        let first = owner.map_or(1, |owner| 1 - owner);
        reveal_pair(&manager, Some(cursor), first, owner.unwrap_or(first));
    }
}

#[test]
fn invalid_scale_or_empty_frame_cannot_guess_a_cursor_owner() {
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        for axis in 0..2 {
            let manager = manager_with_frames(uniform(), CaptureIntent::Screenshot);
            {
                let mut session = manager.session.lock().unwrap();
                let frame = &mut session.as_mut().unwrap().frames[1];
                if axis == 0 {
                    frame.scale_x = scale;
                } else {
                    frame.scale_y = scale;
                }
            }
            reveal_pair(&manager, Some((2400.0, 100.0)), 0, 0);
        }
    }
    for axis in 0..4 {
        let manager = manager_with_frames(uniform(), CaptureIntent::Screenshot);
        {
            let mut session = manager.session.lock().unwrap();
            let frame = &mut session.as_mut().unwrap().frames[1];
            match axis {
                0 => frame.logical_width = 0,
                1 => frame.logical_height = 0,
                2 => frame.pixel_width = 0,
                _ => frame.pixel_height = 0,
            }
        }
        reveal_pair(&manager, Some((2400.0, 100.0)), 0, 0);
    }
}

#[test]
fn missing_nonfinite_and_offscreen_cursor_focus_only_first_ready_overlay() {
    for cursor in [
        None,
        Some((f64::NAN, 100.0)),
        Some((100.0, f64::INFINITY)),
        Some((-10000.0, -10000.0)),
        Some((10000.0, 10000.0)),
    ] {
        let manager = manager_with_frames(uniform(), CaptureIntent::Screenshot);
        reveal_pair(&manager, cursor, 1, 1);
    }
}

#[test]
fn unscaled_single_monitor_retains_both_intents_and_label_validation() {
    for intent in [CaptureIntent::Screenshot, CaptureIntent::Recording] {
        let manager = manager_with_frames(vec![frame(1, 0, 0, 1920, 1080, 1.0, 1.0)], intent);
        let label = manager.session.lock().unwrap().as_ref().unwrap().overlays[0]
            .label
            .clone();
        assert!(
            manager
                .reveal(&label, Some((1800.0, 1000.0)), None)
                .unwrap()
                .take_focus
        );
        assert_eq!(
            manager.reveal("absent", None, None).unwrap_err().code(),
            "overlay_not_in_session"
        );
        let empty = CaptureManager::new();
        assert_eq!(
            empty.reveal(&label, None, None).unwrap_err().code(),
            "session_missing"
        );
    }
}
