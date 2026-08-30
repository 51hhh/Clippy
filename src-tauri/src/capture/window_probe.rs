//! 窗口速选候选区的采集。
//!
//! 这里要跨三个坐标系：X11 窗口几何是 X screen 的**原始像素**，冻结帧是显示器的**物理像素**，
//! 覆盖层用的是**逻辑像素**。三者在无缩放的 X11 会话里恰好相等，所以这段代码长期看着是对的；
//! 一旦有缩放就会错得很离谱——实测 GNOME Wayland 下 XWayland 的 X screen 是 3840x2400
//! （逻辑 1920x1200 的两倍），一个普通 QQ 窗口被报成 2598x1472，比整个逻辑桌面还宽。
//! 因此窗口矩形必须先按 `X screen 像素 / 逻辑像素` 折算，再和帧的逻辑边界求交。

use super::types::WindowCandidate;
use crate::screenshot::CapturedMonitorFrame;
use std::collections::HashMap;

/// 窗口矩形，单位随上下文（先是 X 像素，折算后是逻辑像素）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProbeRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// `_GTK_FRAME_EXTENTS` / `_NET_FRAME_EXTENTS` 的四个边距，单位是 X 像素。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct FrameExtents {
    pub left: u32,
    pub right: u32,
    pub top: u32,
    pub bottom: u32,
}

/// 速选候选区小于这个尺寸就没有点击价值，也挡不住误命中。
const MIN_CANDIDATE_SIZE: i32 = 20;

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

    let ratio = x11_pixel_ratio(frames);
    let mut extents_source = FrameExtentsSource::open();

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
        let raw = ProbeRect {
            x,
            y,
            width,
            height,
        };
        let extents = window
            .id()
            .ok()
            .and_then(|id| extents_source.extents(id))
            .unwrap_or_default();
        let rect = to_logical(trim_frame_extents(raw, extents), ratio);
        if (rect.width as i32) < MIN_CANDIDATE_SIZE || (rect.height as i32) < MIN_CANDIDATE_SIZE {
            continue;
        }
        let title = window.title().unwrap_or_default();
        append_window_intersections(&mut result, frames, rect, &title);
    }
    for candidates in result.values_mut() {
        candidates.sort_by(|a, b| (a.width * a.height).total_cmp(&(b.width * b.height)));
    }
    result
}

/// X screen 像素与逻辑像素的比例。
///
/// xcap 的 `Monitor::width()` 已经是 `RandR 像素 / scale_factor`，所以乘回 `scale_factor`
/// 就还原成 RandR 像素——也就是 X11 窗口几何所在的空间。分母用冻结帧的逻辑宽度
/// （已由 `screenshot::backends` 修正过）。取最宽的显示器比较，多显示器混合缩放下只能近似，
/// 但那种组合在 XWayland 里本身就没有可靠的公开接口。
fn x11_pixel_ratio(frames: &[CapturedMonitorFrame]) -> f32 {
    let logical_width = frames
        .iter()
        .map(|frame| frame.logical_width)
        .max()
        .unwrap_or(0);
    let randr_width = xcap::Monitor::all()
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|monitor| {
            let width = monitor.width().ok()? as f32;
            let scale = monitor.scale_factor().ok()?;
            (scale.is_finite() && scale > 0.0).then(|| (width * scale).round() as u32)
        })
        .max()
        .unwrap_or(0);
    ratio_from_widths(randr_width, logical_width)
}

pub(super) fn ratio_from_widths(randr_width: u32, logical_width: u32) -> f32 {
    if randr_width == 0 || logical_width == 0 {
        return 1.0;
    }
    let ratio = randr_width as f32 / logical_width as f32;
    // 比例只可能 ≥ 1（X screen 不会比逻辑桌面小）；超过 4 说明前提已经不成立，
    // 这时候宁可不折算，也不要把窗口缩成一个点。
    if !ratio.is_finite() || !(1.0..=4.0).contains(&ratio) {
        1.0
    } else {
        ratio
    }
}

/// 去掉 GTK 客户端装饰的不可见阴影边距。GNOME 下 CSD 窗口的客户端矩形比肉眼看到的
/// 窗口大一圈（左右各约 26px、下方更多），不减掉的话速选框会明显包住一块空白。
pub(super) fn trim_frame_extents(rect: ProbeRect, extents: FrameExtents) -> ProbeRect {
    let horizontal = extents.left.saturating_add(extents.right);
    let vertical = extents.top.saturating_add(extents.bottom);
    if horizontal >= rect.width || vertical >= rect.height {
        return rect;
    }
    ProbeRect {
        x: rect.x.saturating_add_unsigned(extents.left),
        y: rect.y.saturating_add_unsigned(extents.top),
        width: rect.width - horizontal,
        height: rect.height - vertical,
    }
}

pub(super) fn to_logical(rect: ProbeRect, ratio: f32) -> ProbeRect {
    if !ratio.is_finite() || ratio <= 0.0 || (ratio - 1.0).abs() < f32::EPSILON {
        return rect;
    }
    ProbeRect {
        x: (rect.x as f32 / ratio).round() as i32,
        y: (rect.y as f32 / ratio).round() as i32,
        width: (rect.width as f32 / ratio).round() as u32,
        height: (rect.height as f32 / ratio).round() as u32,
    }
}

fn append_window_intersections(
    result: &mut HashMap<u32, Vec<WindowCandidate>>,
    frames: &[CapturedMonitorFrame],
    rect: ProbeRect,
    title: &str,
) {
    for frame in frames {
        let left = rect.x.max(frame.x);
        let top = rect.y.max(frame.y);
        let right = rect
            .x
            .saturating_add_unsigned(rect.width)
            .min(frame.x.saturating_add_unsigned(frame.logical_width));
        let bottom = rect
            .y
            .saturating_add_unsigned(rect.height)
            .min(frame.y.saturating_add_unsigned(frame.logical_height));
        if right - left < MIN_CANDIDATE_SIZE || bottom - top < MIN_CANDIDATE_SIZE {
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

#[cfg(target_os = "linux")]
mod frame_extents {
    use super::FrameExtents;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
    use x11rb::rust_connection::RustConnection;

    /// 一次会话只开一条 X 连接，按需查询边距。没有 X（纯 Wayland 且无 XWayland）时
    /// 整体退化为"没有边距"，速选仍可用，只是框会包含阴影。
    pub(super) struct FrameExtentsSource {
        inner: Option<Inner>,
    }

    struct Inner {
        connection: RustConnection,
        gtk_extents: u32,
        net_extents: u32,
    }

    impl FrameExtentsSource {
        pub(super) fn open() -> Self {
            Self {
                inner: Self::try_open(),
            }
        }

        fn try_open() -> Option<Inner> {
            let (connection, _) = RustConnection::connect(None).ok()?;
            let gtk_extents = atom(&connection, b"_GTK_FRAME_EXTENTS")?;
            let net_extents = atom(&connection, b"_NET_FRAME_EXTENTS")?;
            Some(Inner {
                connection,
                gtk_extents,
                net_extents,
            })
        }

        pub(super) fn extents(&mut self, window: u32) -> Option<FrameExtents> {
            let inner = self.inner.as_ref()?;
            // CSD 窗口只有 _GTK_FRAME_EXTENTS（阴影，要减掉）；SSD 窗口只有
            // _NET_FRAME_EXTENTS（标题栏与边框，属于窗口本体，不减）。所以只认前者。
            read_extents(&inner.connection, window, inner.gtk_extents).or_else(|| {
                // 有 _NET_FRAME_EXTENTS 说明是服务端装饰，明确返回零而不是 None，
                // 免得调用方以为查询失败。
                read_extents(&inner.connection, window, inner.net_extents)
                    .map(|_| FrameExtents::default())
            })
        }
    }

    fn atom(connection: &RustConnection, name: &[u8]) -> Option<u32> {
        Some(connection.intern_atom(true, name).ok()?.reply().ok()?.atom)
    }

    fn read_extents(
        connection: &RustConnection,
        window: u32,
        property: u32,
    ) -> Option<FrameExtents> {
        let reply = connection
            .get_property(false, window, property, AtomEnum::CARDINAL, 0, 4)
            .ok()?
            .reply()
            .ok()?;
        let values: Vec<u32> = reply.value32()?.collect();
        // 协议规定四个 CARDINAL：left, right, top, bottom。
        let [left, right, top, bottom] = values.as_slice() else {
            return None;
        };
        Some(FrameExtents {
            left: *left,
            right: *right,
            top: *top,
            bottom: *bottom,
        })
    }
}

#[cfg(not(target_os = "linux"))]
mod frame_extents {
    use super::FrameExtents;

    pub(super) struct FrameExtentsSource;

    impl FrameExtentsSource {
        pub(super) fn open() -> Self {
            Self
        }

        pub(super) fn extents(&mut self, _window: u32) -> Option<FrameExtents> {
            None
        }
    }
}

use frame_extents::FrameExtentsSource;

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: u32, height: u32) -> ProbeRect {
        ProbeRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn ratio_recovers_xwayland_scaling() {
        // 实测值：XWayland X screen 3840 宽，逻辑桌面 1920 宽。
        assert_eq!(ratio_from_widths(3840, 1920), 2.0);
        assert_eq!(ratio_from_widths(1920, 1920), 1.0);
        assert_eq!(ratio_from_widths(3840, 2560), 1.5);
    }

    #[test]
    fn ratio_falls_back_to_one_on_nonsense_inputs() {
        // X screen 不可能比逻辑桌面小，比例也不该大到 4 倍以上。
        assert_eq!(ratio_from_widths(0, 1920), 1.0);
        assert_eq!(ratio_from_widths(3840, 0), 1.0);
        assert_eq!(ratio_from_widths(1280, 1920), 1.0);
        assert_eq!(ratio_from_widths(19200, 1920), 1.0);
    }

    #[test]
    fn logical_conversion_shrinks_the_measured_qq_window_into_the_desktop() {
        // 报障现场：QQ 客户端矩形 (268,150) 2598x1472，比 1920 逻辑桌面还宽。
        let logical = to_logical(rect(268, 150, 2598, 1472), 2.0);
        assert_eq!(logical, rect(134, 75, 1299, 736));
        assert!(logical.width < 1920 && logical.height < 1200);
    }

    #[test]
    fn logical_conversion_is_identity_at_ratio_one() {
        assert_eq!(to_logical(rect(10, 20, 30, 40), 1.0), rect(10, 20, 30, 40));
        assert_eq!(to_logical(rect(10, 20, 30, 40), 0.0), rect(10, 20, 30, 40));
        assert_eq!(
            to_logical(rect(10, 20, 30, 40), f32::NAN),
            rect(10, 20, 30, 40)
        );
    }

    #[test]
    fn frame_extents_trim_the_invisible_gtk_shadow() {
        let trimmed = trim_frame_extents(
            rect(100, 100, 800, 600),
            FrameExtents {
                left: 26,
                right: 26,
                top: 24,
                bottom: 68,
            },
        );
        assert_eq!(trimmed, rect(126, 124, 748, 508));
    }

    #[test]
    fn frame_extents_never_collapse_the_window() {
        let absurd = FrameExtents {
            left: 900,
            right: 900,
            top: 0,
            bottom: 0,
        };
        assert_eq!(
            trim_frame_extents(rect(0, 0, 800, 600), absurd),
            rect(0, 0, 800, 600)
        );
    }

    #[test]
    fn intersections_are_clipped_to_the_frame_and_made_frame_relative() {
        let mut result = HashMap::new();
        let frames = vec![CapturedMonitorFrame {
            monitor_id: 3,
            x: 1920,
            y: 0,
            logical_width: 1920,
            logical_height: 1080,
            pixel_width: 1920,
            pixel_height: 1080,
            scale_x: 1.0,
            scale_y: 1.0,
            rgba: std::sync::Arc::from(Vec::new()),
        }];
        // 窗口跨过两个显示器，只有右半边落在这个帧上。
        append_window_intersections(&mut result, &frames, rect(1820, 100, 400, 300), "spanning");
        let candidates = result.get(&3).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            (
                candidates[0].x,
                candidates[0].y,
                candidates[0].width,
                candidates[0].height
            ),
            (0.0, 100.0, 300.0, 300.0)
        );
    }

    #[test]
    fn intersections_drop_slivers_below_the_click_threshold() {
        let mut result = HashMap::new();
        let frames = vec![CapturedMonitorFrame {
            monitor_id: 1,
            x: 0,
            y: 0,
            logical_width: 1920,
            logical_height: 1080,
            pixel_width: 1920,
            pixel_height: 1080,
            scale_x: 1.0,
            scale_y: 1.0,
            rgba: std::sync::Arc::from(Vec::new()),
        }];
        append_window_intersections(&mut result, &frames, rect(1915, 100, 400, 300), "sliver");
        assert!(result.is_empty());
    }
}
