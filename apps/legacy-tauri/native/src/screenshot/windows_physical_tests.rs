//! 只调用取像素之后的生产数据入口，不枚举显示器或捕获屏幕。

use super::*;
use image::RgbaImage;

fn info(x: i32, y: i32) -> MonitorInfo {
    MonitorInfo {
        id: 7,
        rect: Rect {
            x,
            y,
            width: 192,
            height: 150,
        },
        scale_factor: 1.5,
    }
}

#[test]
fn original_positive_and_negative_bounds_survive_freeze_and_logical_normalization() {
    for (x, y, logical_x, logical_y) in [(2560, 1250, 1707, 833), (-2560, -1250, -1707, -833)] {
        let (monitor, frozen) =
            backends::freeze_xcap_monitor(info(x, y), RgbaImage::new(192, 150)).unwrap();
        assert_eq!((monitor.rect.x, monitor.rect.y), (logical_x, logical_y));
        let bounds = Some(PhysicalMonitorBounds {
            x,
            y,
            width: 192,
            height: 150,
        });
        assert_eq!(frozen.physical_bounds, bounds);
        let frames = captured_frames_from(vec![monitor], vec![frozen]).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].physical_bounds, bounds);
        assert_eq!((frames[0].scale_x, frames[0].scale_y), (1.5, 1.5));
        assert_eq!(frames[0].rgba.len(), 192 * 150 * 4);
    }
}

#[test]
fn mismatched_empty_and_overflowing_raw_monitor_bounds_are_rejected() {
    for (monitor, width, height) in [
        (info(2560, 0), 191, 150),
        (info(2560, 0), 192, 149),
        (info(2560, 0), 0, 150),
        (info(i32::MAX - 10, 0), 192, 150),
        (info(0, i32::MAX - 10), 192, 150),
    ] {
        assert!(backends::freeze_xcap_monitor(monitor, RgbaImage::new(width, height)).is_err());
    }
}

#[test]
fn captured_frame_conversion_does_not_accept_missing_authoritative_bounds() {
    let (monitor, mut frozen) =
        backends::freeze_xcap_monitor(info(0, 0), RgbaImage::new(192, 150)).unwrap();
    frozen.physical_bounds = None;
    assert!(captured_frames_from(vec![monitor], vec![frozen]).is_err());
}
