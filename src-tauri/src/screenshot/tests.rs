
use super::*;

#[test]
fn portal_file_uri_decodes_to_local_path() {
    let path = portal_screenshot_uri_to_path("file:///tmp/Clippy%20Shot.png").unwrap();
    assert_eq!(path, std::path::PathBuf::from("/tmp/Clippy Shot.png"));
}

#[test]
fn encode_png_compresses_simple_screenshot_data() {
    let width = 64;
    let height = 64;
    let mut rgba = Vec::with_capacity(width * height * 4);
    for _ in 0..(width * height) {
        rgba.extend_from_slice(&[240, 240, 240, 255]);
    }

    let png = encode_png(&rgba, width as u32, height as u32).unwrap();

    assert!(png.len() < rgba.len() / 4);
    assert_eq!(png_dimensions(&png).unwrap(), (width as u32, height as u32));
}

#[test]
fn portal_mapping_splits_horizontal_monitors() {
    let monitors = vec![
        MonitorInfo {
            id: 1,
            rect: Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 50,
            },
            scale_factor: 1.0,
        },
        MonitorInfo {
            id: 2,
            rect: Rect {
                x: 100,
                y: 0,
                width: 100,
                height: 50,
            },
            scale_factor: 1.0,
        },
    ];
    let desktop = monitor_union(&monitors).unwrap();

    assert_eq!(
        scaled_monitor_rect(&monitors[0].rect, &desktop, 2.0, 2.0, 400, 100).unwrap(),
        ImageRect {
            x: 0,
            y: 0,
            width: 200,
            height: 100,
        }
    );
    assert_eq!(
        scaled_monitor_rect(&monitors[1].rect, &desktop, 2.0, 2.0, 400, 100).unwrap(),
        ImageRect {
            x: 200,
            y: 0,
            width: 200,
            height: 100,
        }
    );
}

#[test]
fn compose_places_left_1x_right_2x_monitors_without_gap() {
    let monitors = horizontal_monitors(1.0, 2.0);
    let frames = vec![
        solid_frame(1, 100, 50, 1.0, [255, 0, 0, 255]),
        solid_frame(2, 200, 100, 2.0, [0, 0, 255, 255]),
    ];

    let (rgba, width, height) = compose_desktop_image(&monitors, &frames).unwrap();

    assert_eq!((width, height), (300, 100));
    assert_eq!(pixel_at(&rgba, width, 99, 25), [255, 0, 0, 255]);
    assert_eq!(pixel_at(&rgba, width, 100, 25), [0, 0, 255, 255]);
    assert_eq!(pixel_at(&rgba, width, 299, 25), [0, 0, 255, 255]);
}

#[test]
fn compose_places_left_2x_right_1x_monitors_without_overlap() {
    let monitors = horizontal_monitors(2.0, 1.0);
    let frames = vec![
        solid_frame(1, 200, 100, 2.0, [255, 0, 0, 255]),
        solid_frame(2, 100, 50, 1.0, [0, 0, 255, 255]),
    ];

    let (rgba, width, height) = compose_desktop_image(&monitors, &frames).unwrap();

    assert_eq!((width, height), (300, 100));
    assert_eq!(pixel_at(&rgba, width, 199, 25), [255, 0, 0, 255]);
    assert_eq!(pixel_at(&rgba, width, 200, 25), [0, 0, 255, 255]);
    assert_eq!(pixel_at(&rgba, width, 299, 25), [0, 0, 255, 255]);
}

fn horizontal_monitors(left_scale: f32, right_scale: f32) -> Vec<MonitorInfo> {
    vec![
        MonitorInfo {
            id: 1,
            rect: Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 50,
            },
            scale_factor: left_scale,
        },
        MonitorInfo {
            id: 2,
            rect: Rect {
                x: 100,
                y: 0,
                width: 100,
                height: 50,
            },
            scale_factor: right_scale,
        },
    ]
}

fn solid_frame(
    monitor_id: u32,
    width: u32,
    height: u32,
    scale_factor: f32,
    color: [u8; 4],
) -> FrozenFrame {
    FrozenFrame {
        monitor_id,
        rgba: Arc::from(solid_rgba(width, height, color)),
        width,
        height,
        scale_factor,
    }
}

fn solid_rgba(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for _ in 0..width * height {
        rgba.extend_from_slice(&color);
    }
    rgba
}

fn pixel_at(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let start = ((y * width + x) * 4) as usize;
    [
        rgba[start],
        rgba[start + 1],
        rgba[start + 2],
        rgba[start + 3],
    ]
}
