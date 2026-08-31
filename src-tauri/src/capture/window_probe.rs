//! 窗口速选候选区的采集。
//!
//! 有两条来源，按可靠性排序：
//!
//! 1. **GNOME Shell 扩展**（`shell_extension`）。给的是逻辑像素、已排除 CSD 阴影的
//!    `frame_rect`，还带真正的堆叠顺序，不需要任何折算。GNOME Wayland 下这是唯一能
//!    看到原生 Wayland 窗口的途径——实测同一时刻扩展报 3 个窗口，X11 枚举报 0 个。
//! 2. **X11 枚举**（xcap + x11rb）。只看得到 XWayland 窗口，且要跨三个坐标系：X11
//!    窗口几何是 X screen 的**原始像素**，冻结帧是显示器的**物理像素**，覆盖层用的是
//!    **逻辑像素**。三者在无缩放的 X11 会话里恰好相等，所以这段代码长期看着是对的；
//!    一旦有缩放就会错得很离谱——实测 GNOME Wayland 下 XWayland 的 X screen 是
//!    3840x2400（逻辑 1920x1200 的两倍），一个普通 QQ 窗口被报成 2598x1472，比整个
//!    逻辑桌面还宽。因此窗口矩形必须先按 `X screen 像素 / 逻辑像素` 折算，再求交。
//!
//! **不变量：下发给覆盖层的候选数组即堆叠顺序，索引 0 是最上层。** 覆盖层的 `windowAt`
//! 取第一个命中的候选，所以这个顺序就是遮挡关系的答案：点落在两个窗口重叠处时选上面
//! 那个，落在下层窗口露出来的部分时选下层，被完全盖住的窗口自然选不到。拿不到堆叠顺序
//! 时才退化成"面积小的优先"这种猜测。

use super::shell_extension::ShellWindow;
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
    if let Some(windows) = super::shell_extension::probe() {
        let result = candidates_from_shell(frames, &windows);
        if !result.is_empty() {
            return result;
        }
        log::info!("GNOME Shell 窗口扩展没给出可用候选，退回 X11 枚举");
    }
    candidates_from_x11(frames)
}

/// Shell 扩展来源：坐标已是逻辑像素且不含阴影，直接求交即可，数组顺序就是堆叠顺序。
pub(super) fn candidates_from_shell(
    frames: &[CapturedMonitorFrame],
    windows: &[ShellWindow],
) -> HashMap<u32, Vec<WindowCandidate>> {
    let mut result: HashMap<u32, Vec<WindowCandidate>> = HashMap::new();
    let own_pid = std::process::id();
    for window in windows {
        // 自己的悬浮面板/覆盖层不该成为速选目标。
        if window.pid != 0 && window.pid == own_pid {
            continue;
        }
        if window.width < MIN_CANDIDATE_SIZE || window.height < MIN_CANDIDATE_SIZE {
            continue;
        }
        let rect = ProbeRect {
            x: window.x,
            y: window.y,
            width: window.width as u32,
            height: window.height as u32,
        };
        append_window_intersections(&mut result, frames, rect, &window.title);
    }
    result
}

fn candidates_from_x11(frames: &[CapturedMonitorFrame]) -> HashMap<u32, Vec<WindowCandidate>> {
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
    let mut x11 = X11Probe::open();
    let own_pid = std::process::id();
    let mut collected: Vec<(Option<u32>, ProbeRect, String)> = Vec::new();

    for window in windows {
        if window.pid().unwrap_or(0) == own_pid {
            continue;
        }
        let id = window.id().ok();
        // xcap 的 is_minimized 在 XWayland 下不可信（实测把肉眼可见的 QQ 报成已最小化，
        // 于是唯一的候选被滤掉了）。改用 EWMH 的权威信号 WM_STATE == IconicState。
        if id.map(|id| x11.is_iconified(id)).unwrap_or(false) {
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
        let extents = id.and_then(|id| x11.extents(id)).unwrap_or_default();
        let rect = to_logical(trim_frame_extents(raw, extents), ratio);
        if (rect.width as i32) < MIN_CANDIDATE_SIZE || (rect.height as i32) < MIN_CANDIDATE_SIZE {
            continue;
        }
        collected.push((id, rect, window.title().unwrap_or_default()));
    }

    order_x11_candidates(&mut collected, x11.stacking_order().as_deref());
    for (_, rect, title) in &collected {
        append_window_intersections(&mut result, frames, *rect, title);
    }
    result
}

/// 把 X11 候选排成"索引 0 最上层"。
///
/// 有 `_NET_CLIENT_LIST_STACKING`（由下到上）时按它排，这是真正的遮挡关系；拿不到时
/// 退化成面积小的优先——纯猜，但比任意顺序好：小窗口通常压在大窗口上面。
pub(super) fn order_x11_candidates(
    candidates: &mut [(Option<u32>, ProbeRect, String)],
    stacking: Option<&[u32]>,
) {
    match stacking {
        Some(stacking) if !stacking.is_empty() => {
            // 不在列表里的窗口（没被 WM 管理）当作最底层。
            let rank = |id: &Option<u32>| {
                id.and_then(|id| stacking.iter().position(|entry| *entry == id))
                    .map(|index| index as i64)
                    .unwrap_or(-1)
            };
            candidates.sort_by_key(|candidate| std::cmp::Reverse(rank(&candidate.0)));
        }
        _ => candidates.sort_by_key(|(_, rect, _)| (rect.width as u64) * (rect.height as u64)),
    }
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
mod x11_probe {
    use super::FrameExtents;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
    use x11rb::rust_connection::RustConnection;

    /// X11 侧的窗口元数据查询：阴影边距、最小化状态、堆叠顺序。
    ///
    /// 一次会话只开一条 X 连接。没有 X（纯 Wayland 且无 XWayland）时整体退化：没有边距、
    /// 不认为有窗口最小化、没有堆叠顺序，速选仍可用，只是框会包含阴影、顺序靠面积猜。
    pub(super) struct X11Probe {
        inner: Option<Inner>,
    }

    struct Inner {
        connection: RustConnection,
        root: u32,
        gtk_extents: u32,
        net_extents: u32,
        wm_state: u32,
        client_list_stacking: u32,
    }

    /// ICCCM WM_STATE 的第一个值：0=Withdrawn，1=Normal，3=Iconic。
    const ICONIC_STATE: u32 = 3;

    impl X11Probe {
        pub(super) fn open() -> Self {
            Self {
                inner: Self::try_open(),
            }
        }

        fn try_open() -> Option<Inner> {
            let (connection, screen) = RustConnection::connect(None).ok()?;
            let root = connection.setup().roots.get(screen)?.root;
            let gtk_extents = atom(&connection, b"_GTK_FRAME_EXTENTS")?;
            let net_extents = atom(&connection, b"_NET_FRAME_EXTENTS")?;
            let wm_state = atom(&connection, b"WM_STATE")?;
            let client_list_stacking = atom(&connection, b"_NET_CLIENT_LIST_STACKING")?;
            Some(Inner {
                connection,
                root,
                gtk_extents,
                net_extents,
                wm_state,
                client_list_stacking,
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

        /// 窗口是否已最小化。读不到 WM_STATE 就当作没最小化——宁可多给一个候选，
        /// 也不要像 xcap 那样把肉眼可见的窗口误判成最小化后整体没得选。
        pub(super) fn is_iconified(&mut self, window: u32) -> bool {
            let Some(inner) = self.inner.as_ref() else {
                return false;
            };
            // WM_STATE 的类型就是 WM_STATE 自己，不是标准原子。
            let Ok(cookie) =
                inner
                    .connection
                    .get_property(false, window, inner.wm_state, inner.wm_state, 0, 2)
            else {
                return false;
            };
            cookie
                .reply()
                .ok()
                .and_then(|reply| reply.value32()?.next())
                .map(|state| state == ICONIC_STATE)
                .unwrap_or(false)
        }

        /// `_NET_CLIENT_LIST_STACKING`：由下到上的窗口顺序。
        pub(super) fn stacking_order(&self) -> Option<Vec<u32>> {
            let inner = self.inner.as_ref()?;
            let reply = inner
                .connection
                .get_property(
                    false,
                    inner.root,
                    inner.client_list_stacking,
                    AtomEnum::WINDOW,
                    0,
                    u32::MAX,
                )
                .ok()?
                .reply()
                .ok()?;
            let ids: Vec<u32> = reply.value32()?.collect();
            (!ids.is_empty()).then_some(ids)
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
mod x11_probe {
    use super::FrameExtents;

    pub(super) struct X11Probe;

    impl X11Probe {
        pub(super) fn open() -> Self {
            Self
        }

        pub(super) fn extents(&mut self, _window: u32) -> Option<FrameExtents> {
            None
        }

        pub(super) fn is_iconified(&mut self, _window: u32) -> bool {
            false
        }

        pub(super) fn stacking_order(&self) -> Option<Vec<u32>> {
            None
        }
    }
}

use x11_probe::X11Probe;

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

    fn frame(monitor_id: u32) -> CapturedMonitorFrame {
        CapturedMonitorFrame {
            monitor_id,
            x: 0,
            y: 0,
            logical_width: 1920,
            logical_height: 1200,
            pixel_width: 2560,
            pixel_height: 1600,
            scale_x: 4.0 / 3.0,
            scale_y: 4.0 / 3.0,
            rgba: std::sync::Arc::from(Vec::new()),
        }
    }

    fn shell_window(x: i32, y: i32, width: i32, height: i32, title: &str) -> ShellWindow {
        ShellWindow {
            x,
            y,
            width,
            height,
            title: title.to_string(),
            wm_class: String::new(),
            pid: 0,
        }
    }

    #[test]
    fn shell_candidates_keep_stacking_order_and_need_no_conversion() {
        // 实测载荷：终端在最上层，Clash Verge 在下面，两者不重叠也无所谓——
        // 关键是下发顺序必须原样保留，覆盖层靠它判断遮挡。
        let windows = vec![
            shell_window(848, 37, 924, 1157, "terminal"),
            shell_window(67, 270, 940, 700, "clash"),
        ];
        let result = candidates_from_shell(&[frame(1)], &windows);
        let candidates = result.get(&1).expect("应有候选");
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.title.as_str())
                .collect::<Vec<_>>(),
            vec!["terminal", "clash"]
        );
        // 逻辑像素直接落地：不再折算，也不再裁阴影。
        assert_eq!(
            (
                candidates[0].x,
                candidates[0].y,
                candidates[0].width,
                candidates[0].height
            ),
            (848.0, 37.0, 924.0, 1157.0)
        );
    }

    #[test]
    fn shell_candidates_drop_own_windows_and_slivers() {
        let own = std::process::id();
        let mut mine = shell_window(0, 0, 380, 500, "Clippy");
        mine.pid = own;
        let windows = vec![
            mine,
            shell_window(100, 100, 4, 400, "sliver"),
            shell_window(200, 200, 600, 400, "keeper"),
        ];
        let result = candidates_from_shell(&[frame(1)], &windows);
        let candidates = result.get(&1).expect("应有候选");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].title, "keeper");
    }

    #[test]
    fn stacking_order_puts_the_topmost_window_first() {
        // _NET_CLIENT_LIST_STACKING 由下到上：0x30 在最上层。
        let mut candidates = vec![
            (Some(0x10), rect(0, 0, 100, 100), "bottom".to_string()),
            (Some(0x30), rect(0, 0, 900, 900), "top".to_string()),
            (Some(0x20), rect(0, 0, 500, 500), "middle".to_string()),
        ];
        order_x11_candidates(&mut candidates, Some(&[0x10, 0x20, 0x30]));
        assert_eq!(
            candidates
                .iter()
                .map(|(_, _, title)| title.as_str())
                .collect::<Vec<_>>(),
            // 面积排序会给出完全相反的答案，这个断言就是用来锁住"堆叠序优先"的。
            vec!["top", "middle", "bottom"]
        );
    }

    #[test]
    fn unmanaged_windows_sink_to_the_bottom_of_the_stack() {
        let mut candidates = vec![
            (None, rect(0, 0, 100, 100), "unmanaged".to_string()),
            (Some(0x10), rect(0, 0, 900, 900), "managed".to_string()),
        ];
        order_x11_candidates(&mut candidates, Some(&[0x10]));
        assert_eq!(candidates[0].2, "managed");
    }

    #[test]
    fn without_stacking_order_smaller_windows_win() {
        let mut candidates = vec![
            (Some(0x10), rect(0, 0, 900, 900), "big".to_string()),
            (Some(0x20), rect(0, 0, 100, 100), "small".to_string()),
        ];
        order_x11_candidates(&mut candidates, None);
        assert_eq!(candidates[0].2, "small");
        order_x11_candidates(&mut candidates, Some(&[]));
        assert_eq!(candidates[0].2, "small");
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
