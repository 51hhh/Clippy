use super::DesktopRect;
use anyhow::Result;

#[cfg(target_os = "linux")]
mod gnome;
#[cfg(target_os = "linux")]
mod x11;

const MIN_WINDOW_SIDE: i64 = 20;

struct DesktopWindow {
    geometry: DesktopRect,
    pid: u32,
    title: String,
}

#[derive(Debug)]
pub struct WindowTarget {
    pub pixels: [u32; 4],
    pub title: String,
}

/// 每块输出沿用窗口服务给出的从前到后顺序，只投影真正落在对应冻结帧里的区域。
pub fn for_outputs(outputs: &[(DesktopRect, (u32, u32))]) -> Result<Vec<Vec<WindowTarget>>> {
    let windows = desktop_windows()?;
    let pid = std::process::id();
    Ok(outputs
        .iter()
        .map(|&(geometry, size)| targets_for_output(&windows, pid, geometry, size))
        .collect())
}

fn targets_for_output(
    windows: &[DesktopWindow],
    own_pid: u32,
    geometry: DesktopRect,
    size: (u32, u32),
) -> Vec<WindowTarget> {
    windows
        .iter()
        .filter(|window| window.pid != own_pid || window.title == "Clippy Pin")
        .filter_map(|window| {
            Some(WindowTarget {
                pixels: intersect_pixels(window.geometry, geometry, size)?,
                title: window
                    .title
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(160)
                    .collect(),
            })
        })
        .collect()
}

fn intersect_pixels(
    window: DesktopRect,
    output: DesktopRect,
    size: (u32, u32),
) -> Option<[u32; 4]> {
    let x = i64::from(output.x);
    let y = i64::from(output.y);
    let left = i64::from(window.x).max(x) - x;
    let top = i64::from(window.y).max(y) - y;
    let right =
        (i64::from(window.x) + i64::from(window.width)).min(x + i64::from(output.width)) - x;
    let bottom =
        (i64::from(window.y) + i64::from(window.height)).min(y + i64::from(output.height)) - y;
    if right - left < MIN_WINDOW_SIDE || bottom - top < MIN_WINDOW_SIDE {
        return None;
    }
    let scale = |coord: i64, logical: u32, pixels: u32| {
        coord as u64 * u64::from(pixels) / u64::from(logical)
    };
    let scaled_ceil = |coord: i64, logical: u32, pixels: u32| {
        (coord as u64 * u64::from(pixels)).div_ceil(u64::from(logical))
    };
    Some([
        scale(left, output.width, size.0) as u32,
        scale(top, output.height, size.1) as u32,
        scaled_ceil(right, output.width, size.0) as u32,
        scaled_ceil(bottom, output.height, size.1) as u32,
    ])
}

/// 让明确选中的源像素边界走与自由框选相同的输出链，修正浮点比值往错误一侧舍入的情况。
pub fn normalized_pixel_bounds(
    [left, top, right, bottom]: [u32; 4],
    (width, height): (u32, u32),
) -> [f32; 4] {
    fn edge(pixel: u32, dimension: u32, trailing: bool) -> f32 {
        let wanted = pixel as f32;
        let dimension = dimension as f32;
        let ratio = wanted / dimension;
        if trailing && ratio * dimension > wanted {
            ratio.next_down()
        } else if !trailing && ratio * dimension < wanted {
            ratio.next_up()
        } else {
            ratio
        }
    }
    [
        edge(left, width, false),
        edge(top, height, false),
        edge(right, width, true),
        edge(bottom, height, true),
    ]
}

#[cfg(target_os = "linux")]
fn desktop_windows() -> Result<Vec<DesktopWindow>> {
    if !crate::session::is_wayland() {
        x11::windows()
    } else if crate::session::is_gnome() {
        gnome::windows()
    } else {
        // 通用 Wayland 会话没有全局窗口位置，也无法用 XWayland 的列表判断遮挡。
        Ok(Vec::new())
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn desktop_windows() -> Result<Vec<DesktopWindow>> {
    Ok(xcap::Window::snapshots()?
        .into_iter()
        .map(|window| DesktopWindow {
            geometry: DesktopRect {
                x: window.x,
                y: window.y,
                width: window.width,
                height: window.height,
            },
            title: window.title,
            pid: window.pid,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: u32, height: u32) -> DesktopRect {
        DesktopRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn each_part_of_a_window_uses_its_frozen_output_scale_and_origin() {
        let window = rect(-60, -45, 174, 132);
        assert_eq!(
            intersect_pixels(window, rect(-1920, -120, 1920, 1080), (3840, 2160)),
            Some([3720, 150, 3840, 414]),
        );
        assert_eq!(
            intersect_pixels(window, rect(0, -240, 1536, 960), (1920, 1200)),
            Some([0, 243, 143, 409]),
        );
        assert_eq!(
            intersect_pixels(window, rect(800, 0, 80, 80), (80, 80)),
            None
        );
    }

    #[test]
    fn keeps_pins_and_native_stacking_while_skipping_the_capture_overlays() {
        let window = |pid, title: &str| DesktopWindow {
            geometry: rect(20, 20, 160, 100),
            pid,
            title: title.into(),
        };
        let windows = [
            window(7, "Clippy Screenshot"),
            window(2, "Top window"),
            window(7, "Clippy Pin"),
            window(3, "Covered window"),
        ];
        let candidates = targets_for_output(&windows, 7, rect(0, 0, 640, 480), (640, 480));
        assert_eq!(
            candidates
                .iter()
                .map(|target| target.title.as_str())
                .collect::<Vec<_>>(),
            ["Top window", "Clippy Pin", "Covered window"],
        );
    }

    #[test]
    fn a_window_selection_round_trips_through_the_capture_crop() {
        for width in [853, 1280, 1920, 2560, 3840, 4099, 7680, 16384] {
            for left in 0..width {
                let expected = [left, left, left + 1, left + 1];
                let bounds = normalized_pixel_bounds(expected, (width, width));
                assert_eq!(
                    super::super::pixel_rect(width, width, bounds).unwrap(),
                    expected
                );
            }
        }
    }
}
