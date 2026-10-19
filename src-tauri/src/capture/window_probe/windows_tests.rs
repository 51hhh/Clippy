//! 仅使用冻结几何的 Windows 窗口候选回归，不枚举窗口或读取屏幕。

use super::*;

pub(super) fn frame(
    id: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f32,
) -> CapturedMonitorFrame {
    CapturedMonitorFrame {
        monitor_id: id,
        x,
        y,
        logical_width: width,
        logical_height: height,
        pixel_width: (width as f32 * scale).round() as u32,
        pixel_height: (height as f32 * scale).round() as u32,
        scale_x: scale,
        scale_y: scale,
        physical_bounds: Some(crate::screenshot::PhysicalMonitorBounds {
            x: (f64::from(x) * f64::from(scale)).round() as i32,
            y: (f64::from(y) * f64::from(scale)).round() as i32,
            width: (width as f32 * scale).round() as u32,
            height: (height as f32 * scale).round() as u32,
        }),
        rgba: std::sync::Arc::from(Vec::new()),
    }
}

pub(super) fn candidates(
    frames: &[CapturedMonitorFrame],
    raw: ProbeRect,
) -> HashMap<u32, Vec<WindowCandidate>> {
    let mut result = HashMap::new();
    append_windows_window_intersections(&mut result, frames, raw, "window");
    result
}

pub(super) fn assert_candidate(
    result: &HashMap<u32, Vec<WindowCandidate>>,
    id: u32,
    expected: [f64; 4],
) {
    let candidates = result.get(&id).expect("该显示器上必须保留窗口的可见部分");
    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    for (actual, expected) in [candidate.x, candidate.y, candidate.width, candidate.height]
        .into_iter()
        .zip(expected)
    {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }
    assert_eq!(candidate.title, "window");
}

#[test]
fn mixed_primary_100_secondary_150_projects_both_visible_parts() {
    let frames = [
        frame(1, 0, 0, 1920, 1080, 1.0),
        frame(2, 1280, 0, 1280, 720, 1.5),
    ];
    let result = candidates(
        &frames,
        ProbeRect {
            x: 1800,
            y: 150,
            width: 400,
            height: 600,
        },
    );
    assert_candidate(&result, 1, [1800.0, 150.0, 120.0, 600.0]);
    assert_candidate(&result, 2, [0.0, 100.0, 280.0 / 1.5, 400.0]);
}

#[test]
fn mixed_primary_150_secondary_100_does_not_lose_primary_part() {
    let frames = [
        frame(1, 0, 0, 1280, 720, 1.5),
        frame(2, 1920, 0, 1920, 1080, 1.0),
    ];
    let result = candidates(
        &frames,
        ProbeRect {
            x: 1800,
            y: 150,
            width: 400,
            height: 600,
        },
    );
    assert_candidate(&result, 1, [1200.0, 100.0, 80.0, 400.0]);
    assert_candidate(&result, 2, [0.0, 150.0, 280.0, 600.0]);
}

#[test]
fn mixed_negative_origin_uses_each_frames_local_coordinates() {
    let frames = [
        frame(1, -1280, 0, 1280, 720, 1.5),
        frame(2, 0, 0, 1920, 1080, 1.0),
    ];
    let result = candidates(
        &frames,
        ProbeRect {
            x: -300,
            y: 150,
            width: 500,
            height: 300,
        },
    );
    assert_candidate(&result, 1, [1080.0, 100.0, 200.0, 200.0]);
    assert_candidate(&result, 2, [0.0, 150.0, 200.0, 300.0]);
}

#[test]
fn vertical_125_and_150_layout_clips_each_part_before_returning() {
    let frames = [
        frame(1, 0, -864, 1536, 864, 1.25),
        frame(2, 0, 0, 1280, 720, 1.5),
    ];
    let result = candidates(
        &frames,
        ProbeRect {
            x: 150,
            y: -200,
            width: 600,
            height: 500,
        },
    );
    assert_candidate(&result, 1, [120.0, 704.0, 480.0, 160.0]);
    assert_candidate(&result, 2, [100.0, 0.0, 400.0, 200.0]);
}

#[test]
fn uniform_scale_and_single_monitor_keep_existing_geometry() {
    let frames = [
        frame(1, 0, 0, 1280, 720, 1.5),
        frame(2, 1280, 0, 1280, 720, 1.5),
    ];
    let result = candidates(
        &frames,
        ProbeRect {
            x: 1800,
            y: 150,
            width: 600,
            height: 300,
        },
    );
    assert_candidate(&result, 1, [1200.0, 100.0, 80.0, 200.0]);
    assert_candidate(&result, 2, [0.0, 100.0, 320.0, 200.0]);
    let result = candidates(
        &[frame(1, 0, 0, 1920, 1080, 1.0)],
        ProbeRect {
            x: 100,
            y: 200,
            width: 300,
            height: 200,
        },
    );
    assert_candidate(&result, 1, [100.0, 200.0, 300.0, 200.0]);
}

#[test]
fn fractional_edges_and_minimum_size_are_measured_after_clipping() {
    let frames = [frame(1, 0, 0, 1536, 864, 1.25)];
    let result = candidates(
        &frames,
        ProbeRect {
            x: 101,
            y: 151,
            width: 100,
            height: 201,
        },
    );
    assert_candidate(&result, 1, [80.8, 120.8, 80.0, 160.8]);
    let result = candidates(
        &frames,
        ProbeRect {
            x: 1895,
            y: 150,
            width: 100,
            height: 100,
        },
    );
    assert_candidate(&result, 1, [1516.0, 120.0, 20.0, 80.0]);
    assert!(candidates(
        &frames,
        ProbeRect {
            x: 1896,
            y: 150,
            width: 100,
            height: 100
        }
    )
    .is_empty());
    assert!(candidates(
        &frames,
        ProbeRect {
            x: 2000,
            y: 150,
            width: 100,
            height: 100
        }
    )
    .is_empty());
}

#[test]
fn invalid_frame_scale_or_extent_yields_no_guessed_candidate() {
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        for axis in 0..2 {
            let mut metadata = frame(1, 0, 0, 1920, 1080, 1.0);
            if axis == 0 {
                metadata.scale_x = scale;
            } else {
                metadata.scale_y = scale;
            }
            assert!(candidates(
                &[metadata],
                ProbeRect {
                    x: 100,
                    y: 100,
                    width: 200,
                    height: 200
                }
            )
            .is_empty());
        }
    }
    for axis in 0..4 {
        let mut metadata = frame(1, 0, 0, 1920, 1080, 1.0);
        match axis {
            0 => metadata.logical_width = 0,
            1 => metadata.logical_height = 0,
            2 => metadata.pixel_width = 0,
            _ => metadata.pixel_height = 0,
        }
        assert!(candidates(
            &[metadata],
            ProbeRect {
                x: 100,
                y: 100,
                width: 200,
                height: 200
            }
        )
        .is_empty());
    }
}

#[test]
fn physical_candidates_preserve_front_to_back_order() {
    let frames = [
        frame(1, 0, 0, 1280, 720, 1.5),
        frame(2, 1280, 0, 1280, 720, 1.5),
    ];
    let raw = ProbeRect {
        x: 1800,
        y: 150,
        width: 600,
        height: 300,
    };
    let mut result = HashMap::new();
    for title in ["top", "bottom"] {
        append_windows_window_intersections(&mut result, &frames, raw, title);
    }
    for metadata in &frames {
        assert_eq!(
            result[&metadata.monitor_id]
                .iter()
                .map(|candidate| candidate.title.as_str())
                .collect::<Vec<_>>(),
            ["top", "bottom"]
        );
    }
}
