//! Windows Pin 保存/恢复及工具条的原生快照到领域几何入口。
use super::model::{PinEntry, PinLogicalPosition};
use super::window::{visible_window_part, ToolbarBounds};
use super::windows_geometry::{NativeMonitor, PinNativeLayout};
use crate::storage::StoredPinPlacement;
use tauri::{PhysicalPosition, PhysicalSize};

#[cfg(test)]
mod windows_pin_workarea_tests;

#[derive(Debug, Clone)]
pub(super) struct NamedMonitor {
    pub native: NativeMonitor,
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct WindowSnapshot {
    pub outer_position: Option<PhysicalPosition<i32>>,
    pub outer_size: Option<PhysicalSize<u32>>,
    pub client_position: Option<PhysicalPosition<i32>>,
    pub client_size: Option<PhysicalSize<u32>>,
    pub scale: Option<f64>,
    pub owner: Option<NamedMonitor>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct WorkspaceLayout {
    pub restore_position: Option<PinLogicalPosition>,
    pub native: Option<PinNativeLayout>,
    pub scale: f64,
}

impl WorkspaceLayout {
    pub(super) fn apply(self, entry: &mut PinEntry) {
        entry.restore_position = self.restore_position;
        entry.native_layout = self.native;
        entry.device_scale = self.scale;
        entry.buffer_scale = self.scale;
    }
}

/// 已知原生 owner 优先；owner 不可用时按物理外框/客户区与显示器的最大交集择屏。
fn window_owner<'a>(
    snapshot: &'a WindowSnapshot,
    monitors: &'a [NamedMonitor],
    position: PhysicalPosition<i32>,
    size: Option<PhysicalSize<u32>>,
) -> Option<&'a NamedMonitor> {
    if let Some(owner) = snapshot.owner.as_ref().filter(|m| m.native.valid()) {
        return Some(owner);
    }
    let size = size.filter(|s| s.width > 0 && s.height > 0)?;
    let mut best: Option<(&NamedMonitor, u64)> = None;
    for monitor in monitors.iter().filter(|m| m.native.valid()) {
        let bounds = monitor.native.bounds;
        let left = i64::from(position.x).max(i64::from(bounds.x));
        let top = i64::from(position.y).max(i64::from(bounds.y));
        let right = (i64::from(position.x) + i64::from(size.width))
            .min(i64::from(bounds.x) + i64::from(bounds.width));
        let bottom = (i64::from(position.y) + i64::from(size.height))
            .min(i64::from(bounds.y) + i64::from(bounds.height));
        if right <= left || bottom <= top {
            continue;
        }
        let area = (right - left) as u64 * (bottom - top) as u64;
        if best.is_none_or(|(old, old_area)| {
            area > old_area || (area == old_area && monitor.native.primary && !old.native.primary)
        }) {
            best = Some((monitor, area));
        }
    }
    best.map(|(monitor, _)| monitor)
}

/// 保存值仍采用既有逻辑 reference 格式，位置和工作区使用同一 owner 比例。
pub(super) fn capture_placement(
    snapshot: &WindowSnapshot,
    monitors: &[NamedMonitor],
) -> Option<StoredPinPlacement> {
    let position = snapshot.outer_position?;
    let monitor = window_owner(snapshot, monitors, position, snapshot.outer_size)?;
    let scale = monitor.native.scale;
    let work = monitor.native.work;
    Some(StoredPinPlacement {
        x: f64::from(position.x) / scale,
        y: f64::from(position.y) / scale,
        display_name: monitor.name.clone(),
        display_x: f64::from(work.position.x) / scale,
        display_y: f64::from(work.position.y) / scale,
        display_width: f64::from(work.size.width) / scale,
        display_height: f64::from(work.size.height) / scale,
        display_scale: scale,
    })
}

fn valid_saved(saved: &StoredPinPlacement) -> bool {
    [
        saved.x,
        saved.y,
        saved.display_x,
        saved.display_y,
        saved.display_width,
        saved.display_height,
        saved.display_scale,
    ]
    .into_iter()
    .all(f64::is_finite)
        && saved.display_width > 0.0
        && saved.display_height > 0.0
        && saved.display_scale > 0.0
}

/// 名称失效时用保存的工作区及其比例恢复物理 reference，避免 DPI 变更破坏几何匹配。
fn reference_matches(saved: &StoredPinPlacement, monitor: &NamedMonitor) -> bool {
    let work = monitor.native.work;
    [
        (
            saved.display_x * saved.display_scale,
            f64::from(work.position.x),
        ),
        (
            saved.display_y * saved.display_scale,
            f64::from(work.position.y),
        ),
        (
            saved.display_width * saved.display_scale,
            f64::from(work.size.width),
        ),
        (
            saved.display_height * saved.display_scale,
            f64::from(work.size.height),
        ),
    ]
    .into_iter()
    .all(|(old, current)| (old - current).abs() < 1.0)
}

pub(super) fn restore_layout(
    monitors: &[NamedMonitor],
    cursor: Option<PhysicalPosition<f64>>,
    saved: Option<&StoredPinPlacement>,
    outer: (f64, f64),
) -> WorkspaceLayout {
    if ![outer.0, outer.1]
        .into_iter()
        .all(|value| value.is_finite() && value > 0.0)
    {
        return WorkspaceLayout {
            restore_position: None,
            native: None,
            scale: 1.0,
        };
    }
    let monitors: Vec<_> = monitors.iter().filter(|m| m.native.valid()).collect();
    let cursor = cursor.filter(|p| p.x.is_finite() && p.y.is_finite());
    let saved = saved.filter(|s| valid_saved(s));
    let selected = saved
        .and_then(|saved| {
            saved
                .display_name
                .as_deref()
                .and_then(|name| {
                    monitors
                        .iter()
                        .copied()
                        .find(|m| m.name.as_deref() == Some(name))
                })
                .or_else(|| {
                    monitors
                        .iter()
                        .copied()
                        .find(|m| reference_matches(saved, m))
                })
                .or_else(|| monitors.iter().copied().find(|m| m.native.primary))
                .or_else(|| monitors.first().copied())
        })
        .or_else(|| {
            cursor
                .and_then(|p| {
                    monitors
                        .iter()
                        .copied()
                        .find(|m| m.native.bounds.contains(p.x, p.y))
                })
                .or_else(|| monitors.iter().copied().find(|m| m.native.primary))
                .or_else(|| monitors.first().copied())
        });
    let anchor = selected
        .and_then(|m| {
            saved.map(|saved| {
                // 超出 reference 的旧位置仍按原有钳位语义贴边；先钳比例避免有限极值相减/相除溢出。
                let rx = ((saved.x - saved.display_x) / saved.display_width).clamp(0.0, 1.0);
                let ry = ((saved.y - saved.display_y) / saved.display_height).clamp(0.0, 1.0);
                PhysicalPosition::new(
                    f64::from(m.native.work.position.x) + rx * f64::from(m.native.work.size.width),
                    f64::from(m.native.work.position.y) + ry * f64::from(m.native.work.size.height),
                )
            })
        })
        .or_else(|| cursor.map(|p| PhysicalPosition::new(p.x.round() + 12.0, p.y.round() + 12.0)));
    let native = selected.and_then(|m| PinNativeLayout::at(m.native, anchor));
    WorkspaceLayout {
        restore_position: None,
        native,
        scale: selected.map_or(1.0, |m| m.native.scale),
    }
}

/// 全部求交在物理空间完成，输出只按窗口实际 DPR 换算为 WebView 局部 CSS。
pub(super) fn toolbar_bounds(
    snapshot: &WindowSnapshot,
    monitors: &[NamedMonitor],
) -> ToolbarBounds {
    let Some(scale) = snapshot.scale.filter(|s| s.is_finite() && *s > 0.0) else {
        return ToolbarBounds::UNKNOWN;
    };
    let Some(size) = snapshot.client_size.filter(|s| s.width > 0 && s.height > 0) else {
        return ToolbarBounds::UNKNOWN;
    };
    let whole = ToolbarBounds {
        x: 0.0,
        y: 0.0,
        width: f64::from(size.width) / scale,
        height: f64::from(size.height) / scale,
    };
    let Some(position) = snapshot.client_position else {
        return whole;
    };
    let Some(monitor) = window_owner(snapshot, monitors, position, Some(size)) else {
        return whole;
    };
    let work = monitor.native.work;
    let physical = visible_window_part(
        (f64::from(position.x), f64::from(position.y)),
        (f64::from(size.width), f64::from(size.height)),
        (
            f64::from(work.position.x),
            f64::from(work.position.y),
            f64::from(work.size.width),
            f64::from(work.size.height),
        ),
    );
    let bounds = ToolbarBounds {
        x: physical.x / scale,
        y: physical.y / scale,
        width: physical.width / scale,
        height: physical.height / scale,
    };
    if bounds.width < 1.0 || bounds.height < 1.0 {
        whole
    } else {
        bounds
    }
}
