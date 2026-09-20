//! 录屏控制窗的物理几何与平台排除策略。
//!
//! Windows/macOS 的原生帧源可显式排除控制窗；X11 根窗口取帧与 Wayland Portal 没有同等的任意窗口
//! 排除合同，只能把控制窗完整放到选区之外。若所有显示器都被选区覆盖，必须退回托盘/快捷键控制，
//! 不能把“可能被录进去”的浮窗当作可接受结果。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PhysicalRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ControlSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WindowExclusionCapability {
    Native,
    GeometryOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ControlWindowPlan {
    Visible(PhysicalRect),
    TrayAndShortcutsOnly,
}

pub(super) fn plan_control_window(
    selection: PhysicalRect,
    monitors: &[PhysicalRect],
    size: ControlSize,
    margin: u32,
    capability: WindowExclusionCapability,
) -> ControlWindowPlan {
    let Some(selection_bounds) = Bounds::from_rect(selection) else {
        return ControlWindowPlan::TrayAndShortcutsOnly;
    };
    if size.width == 0 || size.height == 0 {
        return ControlWindowPlan::TrayAndShortcutsOnly;
    }
    let monitor_bounds: Vec<_> = monitors
        .iter()
        .filter_map(|monitor| Bounds::from_rect(*monitor))
        .collect();
    if monitor_bounds.is_empty() {
        return ControlWindowPlan::TrayAndShortcutsOnly;
    }

    match capability {
        WindowExclusionCapability::Native => monitor_bounds
            .iter()
            .filter_map(|monitor| native_candidate(selection_bounds, *monitor, size, margin))
            .min_by_key(|candidate| candidate.score)
            .and_then(|candidate| candidate.bounds.to_rect())
            .map(ControlWindowPlan::Visible)
            .unwrap_or(ControlWindowPlan::TrayAndShortcutsOnly),
        WindowExclusionCapability::GeometryOnly => {
            let exclusion = selection_bounds.expand(i64::from(margin));
            monitor_bounds
                .iter()
                .flat_map(|monitor| geometry_candidates(selection_bounds, *monitor, size, margin))
                .filter(|candidate| !candidate.bounds.intersects(exclusion))
                .min_by_key(|candidate| candidate.score)
                .and_then(|candidate| candidate.bounds.to_rect())
                .map(ControlWindowPlan::Visible)
                .unwrap_or(ControlWindowPlan::TrayAndShortcutsOnly)
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Candidate {
    bounds: Bounds,
    score: i128,
}

#[derive(Debug, Clone, Copy)]
struct Bounds {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}

impl Bounds {
    fn from_rect(rect: PhysicalRect) -> Option<Self> {
        if rect.width == 0 || rect.height == 0 {
            return None;
        }
        Some(Self {
            left: i64::from(rect.x),
            top: i64::from(rect.y),
            right: i64::from(rect.x).checked_add(i64::from(rect.width))?,
            bottom: i64::from(rect.y).checked_add(i64::from(rect.height))?,
        })
    }

    fn width(self) -> i64 {
        self.right - self.left
    }

    fn height(self) -> i64 {
        self.bottom - self.top
    }

    fn center(self) -> (i64, i64) {
        ((self.left + self.right) / 2, (self.top + self.bottom) / 2)
    }

    fn expand(self, margin: i64) -> Self {
        Self {
            left: self.left.saturating_sub(margin),
            top: self.top.saturating_sub(margin),
            right: self.right.saturating_add(margin),
            bottom: self.bottom.saturating_add(margin),
        }
    }

    fn contains(self, other: Self) -> bool {
        other.left >= self.left
            && other.top >= self.top
            && other.right <= self.right
            && other.bottom <= self.bottom
    }

    fn intersects(self, other: Self) -> bool {
        self.left < other.right
            && self.right > other.left
            && self.top < other.bottom
            && self.bottom > other.top
    }

    fn to_rect(self) -> Option<PhysicalRect> {
        Some(PhysicalRect {
            x: i32::try_from(self.left).ok()?,
            y: i32::try_from(self.top).ok()?,
            width: u32::try_from(self.width()).ok()?,
            height: u32::try_from(self.height()).ok()?,
        })
    }
}

fn native_candidate(
    selection: Bounds,
    monitor: Bounds,
    size: ControlSize,
    margin: u32,
) -> Option<Candidate> {
    let width = i64::from(size.width);
    let height = i64::from(size.height);
    if width > monitor.width() || height > monitor.height() {
        return None;
    }
    let (selection_x, _) = selection.center();
    let left = clamp(selection_x - width / 2, monitor.left, monitor.right - width);
    let preferred_top = selection
        .bottom
        .saturating_sub(i64::from(margin))
        .saturating_sub(height);
    let top = clamp(preferred_top, monitor.top, monitor.bottom - height);
    candidate(selection, monitor, left, top, width, height, 0)
}

fn geometry_candidates(
    selection: Bounds,
    monitor: Bounds,
    size: ControlSize,
    margin: u32,
) -> impl Iterator<Item = Candidate> {
    let width = i64::from(size.width);
    let height = i64::from(size.height);
    let margin = i64::from(margin);
    let (selection_x, selection_y) = selection.center();
    let centered_x = clamp(
        selection_x - width / 2,
        monitor.left,
        monitor.right.saturating_sub(width),
    );
    let centered_y = clamp(
        selection_y - height / 2,
        monitor.top,
        monitor.bottom.saturating_sub(height),
    );
    [
        (centered_x, selection.bottom.saturating_add(margin), 0),
        (
            centered_x,
            selection.top.saturating_sub(margin).saturating_sub(height),
            1,
        ),
        (selection.right.saturating_add(margin), centered_y, 2),
        (
            selection.left.saturating_sub(margin).saturating_sub(width),
            centered_y,
            3,
        ),
        (monitor.left, monitor.top, 4),
        (monitor.right.saturating_sub(width), monitor.top, 5),
        (monitor.left, monitor.bottom.saturating_sub(height), 6),
        (
            monitor.right.saturating_sub(width),
            monitor.bottom.saturating_sub(height),
            7,
        ),
    ]
    .into_iter()
    .filter_map(move |(left, top, preference)| {
        candidate(selection, monitor, left, top, width, height, preference)
    })
}

fn candidate(
    selection: Bounds,
    monitor: Bounds,
    left: i64,
    top: i64,
    width: i64,
    height: i64,
    preference: i128,
) -> Option<Candidate> {
    if width <= 0 || height <= 0 {
        return None;
    }
    let bounds = Bounds {
        left,
        top,
        right: left.checked_add(width)?,
        bottom: top.checked_add(height)?,
    };
    if !monitor.contains(bounds) {
        return None;
    }
    let (selection_x, selection_y) = selection.center();
    let (candidate_x, candidate_y) = bounds.center();
    let dx = i128::from(candidate_x - selection_x);
    let dy = i128::from(candidate_y - selection_y);
    Some(Candidate {
        bounds,
        score: dx * dx + dy * dy + preference,
    })
}

fn clamp(value: i64, minimum: i64, maximum: i64) -> i64 {
    value.max(minimum).min(maximum)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: u32, height: u32) -> PhysicalRect {
        PhysicalRect {
            x,
            y,
            width,
            height,
        }
    }

    const CONTROL: ControlSize = ControlSize {
        width: 240,
        height: 48,
    };

    #[test]
    fn geometry_only_places_toolbar_below_selection_when_safe() {
        assert_eq!(
            plan_control_window(
                rect(200, 100, 800, 500),
                &[rect(0, 0, 1920, 1080)],
                CONTROL,
                12,
                WindowExclusionCapability::GeometryOnly,
            ),
            ControlWindowPlan::Visible(rect(480, 612, 240, 48))
        );
    }

    #[test]
    fn full_desktop_selection_uses_tray_instead_of_recording_toolbar() {
        assert_eq!(
            plan_control_window(
                rect(0, 0, 1920, 1080),
                &[rect(0, 0, 1920, 1080)],
                CONTROL,
                12,
                WindowExclusionCapability::GeometryOnly,
            ),
            ControlWindowPlan::TrayAndShortcutsOnly
        );
    }

    #[test]
    fn adjacent_monitor_is_used_when_selected_monitor_has_no_safe_space() {
        let plan = plan_control_window(
            rect(0, 0, 1920, 1080),
            &[rect(0, 0, 1920, 1080), rect(1920, 0, 2560, 1440)],
            CONTROL,
            12,
            WindowExclusionCapability::GeometryOnly,
        );
        let ControlWindowPlan::Visible(control) = plan else {
            panic!("第二块显示器应提供安全位置");
        };
        assert!(control.x >= 1920);
        assert!(!Bounds::from_rect(control)
            .unwrap()
            .intersects(Bounds::from_rect(rect(0, 0, 1920, 1080)).unwrap()));
    }

    #[test]
    fn negative_monitor_coordinates_remain_valid_physical_positions() {
        let plan = plan_control_window(
            rect(-1700, 100, 900, 700),
            &[rect(-1920, 0, 1920, 1080), rect(0, 0, 2560, 1440)],
            CONTROL,
            8,
            WindowExclusionCapability::GeometryOnly,
        );
        let ControlWindowPlan::Visible(control) = plan else {
            panic!("负坐标显示器应能规划控制窗");
        };
        assert!(control.x < 0);
        assert!(control.y >= 808);
    }

    #[test]
    fn native_exclusion_allows_toolbar_inside_full_monitor_capture() {
        let plan = plan_control_window(
            rect(0, 0, 1920, 1080),
            &[rect(0, 0, 1920, 1080)],
            CONTROL,
            12,
            WindowExclusionCapability::Native,
        );
        let ControlWindowPlan::Visible(control) = plan else {
            panic!("原生排除平台应显示控制窗");
        };
        assert!(Bounds::from_rect(control)
            .unwrap()
            .intersects(Bounds::from_rect(rect(0, 0, 1920, 1080)).unwrap()));
    }

    #[test]
    fn invalid_or_oversized_control_never_escapes_monitor_bounds() {
        assert_eq!(
            plan_control_window(
                rect(0, 0, 100, 100),
                &[rect(0, 0, 100, 100)],
                ControlSize {
                    width: 101,
                    height: 48,
                },
                0,
                WindowExclusionCapability::Native,
            ),
            ControlWindowPlan::TrayAndShortcutsOnly
        );
        assert_eq!(
            plan_control_window(
                rect(0, 0, 100, 100),
                &[rect(0, 0, 100, 100)],
                ControlSize {
                    width: 0,
                    height: 48,
                },
                0,
                WindowExclusionCapability::GeometryOnly,
            ),
            ControlWindowPlan::TrayAndShortcutsOnly
        );
    }
}
