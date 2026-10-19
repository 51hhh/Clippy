//! 单次原生快照到图片 Pin 的布局与真实 Tauri 物理请求。
use super::model::PinOrigin;
use super::window::{fit_dimensions, outer_size};
use crate::screenshot::PhysicalMonitorBounds;
use tauri::{PhysicalPosition, PhysicalRect, PhysicalSize, Position, Size};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy)]
pub(super) struct NativeMonitor {
    pub bounds: PhysicalMonitorBounds,
    pub work: PhysicalRect<i32, u32>,
    pub scale: f64,
    pub primary: bool,
}

impl NativeMonitor {
    pub(super) fn valid(self) -> bool {
        self.bounds.is_valid()
            && self.scale.is_finite()
            && self.scale > 0.0
            && PhysicalMonitorBounds {
                x: self.work.position.x,
                y: self.work.position.y,
                width: self.work.size.width,
                height: self.work.size.height,
            }
            .is_valid()
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PinNativeLayout {
    pub monitor: NativeMonitor,
    anchor: Option<PhysicalPosition<f64>>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ImageLayout {
    pub width: f64,
    pub height: f64,
    pub scale: f64,
    pub native: Option<PinNativeLayout>,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum PlacementStage {
    Create,
    Reveal,
}

/// 源显示器用冻结物理边界匹配；未知或已失去原屏时按物理光标/主屏兜底。
/// 长截图 union 可以延伸到原屏外，不能用其左上角重新猜另一块屏。
pub(super) fn plan_image(
    monitors: &[NativeMonitor],
    cursor: Option<PhysicalPosition<f64>>,
    pixels: (f64, f64),
    origin: Option<PinOrigin>,
) -> ImageLayout {
    let monitors: Vec<_> = monitors.iter().copied().filter(|m| m.valid()).collect();
    let source = origin
        .and_then(|o| o.physical)
        .filter(|source| source.is_valid())
        .and_then(|source| {
            monitors
                .iter()
                .find(|m| m.bounds == source.monitor)
                .map(|m| (source, *m))
        });
    let cursor = cursor.filter(|p| p.x.is_finite() && p.y.is_finite());
    let selected = source.map(|(_, m)| m).or_else(|| {
        cursor
            .and_then(|p| {
                monitors
                    .iter()
                    .find(|m| m.bounds.contains(p.x, p.y))
                    .copied()
            })
            .or_else(|| monitors.iter().find(|m| m.primary).copied())
            .or_else(|| monitors.first().copied())
    });
    let scale = selected.map_or(1.0, |m| m.scale);
    let (width, height) = (pixels.0 / scale, pixels.1 / scale);
    let (width, height) = if source.is_some() {
        let (max_w, max_h) = selected.map_or((width, height), |m| {
            (
                (f64::from(m.work.size.width) / scale - 68.0).max(1.0),
                (f64::from(m.work.size.height) / scale - 72.0).max(1.0),
            )
        });
        let shrink = (max_w / width).min(max_h / height).min(1.0);
        (width * shrink, height * shrink)
    } else {
        let (max_w, max_h) = selected.map_or((900.0, 700.0), |m| {
            (
                f64::from(m.work.size.width) / scale * 0.72 - 44.0,
                f64::from(m.work.size.height) / scale * 0.72 - 48.0,
            )
        });
        fit_dimensions(width, height, max_w, max_h)
    };
    let anchor = source
        .map(|(source, _)| PhysicalPosition::new(source.x - 12.0 * scale, source.y - 12.0 * scale))
        .or_else(|| cursor.map(|p| PhysicalPosition::new(p.x.round() + 12.0, p.y.round() + 12.0)));
    ImageLayout {
        width,
        height,
        scale,
        native: selected.map(|monitor| PinNativeLayout { monitor, anchor }),
    }
}

impl PinNativeLayout {
    pub(super) fn at(
        monitor: NativeMonitor,
        anchor: Option<PhysicalPosition<f64>>,
    ) -> Option<Self> {
        (monitor.valid() && anchor.is_none_or(|p| p.x.is_finite() && p.y.is_finite()))
            .then_some(Self { monitor, anchor })
    }

    /// 创建/reveal 使用同一原生规划，不能交给窗口当前 DPI 再解释一遍。
    pub(super) fn requests(
        self,
        width: f64,
        height: f64,
        zoom: f64,
        _stage: PlacementStage,
    ) -> (Position, Size) {
        let (width, height) = outer_size(width, height, zoom);
        let work = self.monitor.work;
        let size = PhysicalSize::new(
            ((width * self.monitor.scale).round().max(1.0) as u32).min(work.size.width),
            ((height * self.monitor.scale).round().max(1.0) as u32).min(work.size.height),
        );
        // 用 f64 求钳位边界，避免超宽负原点桌面的 u32 -> i32 中间溢出。
        let start = PhysicalPosition::new(f64::from(work.position.x), f64::from(work.position.y));
        let raw = self.anchor.unwrap_or_else(|| {
            PhysicalPosition::new(
                start.x + f64::from(work.size.width.saturating_sub(size.width)) / 2.0,
                start.y + f64::from(work.size.height.saturating_sub(size.height)) / 2.0,
            )
        });
        let position = PhysicalPosition::new(
            raw.x
                .clamp(
                    start.x,
                    start.x + f64::from(work.size.width.saturating_sub(size.width)),
                )
                .round() as i32,
            raw.y
                .clamp(
                    start.y,
                    start.y + f64::from(work.size.height.saturating_sub(size.height)),
                )
                .round() as i32,
        );
        (Position::Physical(position), Size::Physical(size))
    }
}
