use super::types::WindowCandidate;
use crate::screenshot::CapturedMonitorFrame;
use std::collections::HashMap;

pub(super) fn probe_windows(frames: &[CapturedMonitorFrame]) -> HashMap<u32, Vec<WindowCandidate>> {
    let mut result: HashMap<u32, Vec<WindowCandidate>> = HashMap::new();
    // 部分 Wayland 合成器不给窗口几何，窗口速选会整体退化；日志里必须留下原因，
    // 否则只能看到覆盖层上那句"不可用"，排障没有线索。
    let windows = match xcap::Window::all() {
        Ok(windows) => windows,
        Err(error) => {
            log::info!("窗口枚举失败，截图窗口速选不可用: {error}");
            return result;
        }
    };
    if windows.is_empty() {
        log::info!("窗口枚举返回空列表，截图窗口速选不可用");
        return result;
    }
    for window in windows {
        if window.is_minimized().unwrap_or(false) || window.pid().unwrap_or(0) == std::process::id()
        {
            continue;
        }
        let Ok(x) = window.x() else { continue };
        let Ok(y) = window.y() else { continue };
        let Ok(width) = window.width() else {
            continue;
        };
        let Ok(height) = window.height() else {
            continue;
        };
        if width < 20 || height < 20 {
            continue;
        }
        let title = window.title().unwrap_or_default();
        append_window_intersections(&mut result, frames, x, y, width, height, &title);
    }
    for candidates in result.values_mut() {
        candidates.sort_by(|a, b| (a.width * a.height).total_cmp(&(b.width * b.height)));
    }
    result
}

fn append_window_intersections(
    result: &mut HashMap<u32, Vec<WindowCandidate>>,
    frames: &[CapturedMonitorFrame],
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    title: &str,
) {
    for frame in frames {
        let left = x.max(frame.x);
        let top = y.max(frame.y);
        let right = (x + width as i32).min(frame.x + frame.logical_width as i32);
        let bottom = (y + height as i32).min(frame.y + frame.logical_height as i32);
        if right - left < 20 || bottom - top < 20 {
            continue;
        }
        result
            .entry(frame.monitor_id)
            .or_default()
            .push(WindowCandidate {
                x: (left - frame.x) as f64,
                y: (top - frame.y) as f64,
                width: (right - left) as f64,
                height: (bottom - top) as f64,
                title: title.to_string(),
            });
    }
}
