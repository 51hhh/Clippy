use super::{FrozenFrame, MonitorInfo};
use anyhow::{bail, Result};
use image::{ImageBuffer, RgbaImage};

#[derive(Debug, Clone, Copy)]
enum Axis {
    X,
    Y,
}

#[derive(Debug, Clone, Copy)]
struct AxisSegment {
    start: i32,
    end: i32,
    offset: u32,
    scale: f32,
}

#[derive(Debug, Clone)]
struct AxisMapper {
    segments: Vec<AxisSegment>,
}

impl AxisMapper {
    fn from_frames(monitors: &[MonitorInfo], frames: &[FrozenFrame], axis: Axis) -> Result<Self> {
        let mut edges = Vec::with_capacity(monitors.len() * 2);
        for monitor in monitors {
            let (start, length) = monitor_axis_bounds(monitor, axis);
            edges.push(start);
            edges.push(start.saturating_add_unsigned(length));
        }
        edges.sort_unstable();
        edges.dedup();

        if edges.len() < 2 {
            bail!("显示器坐标轴为空");
        }

        let mut offset = 0_u32;
        let mut segments = Vec::with_capacity(edges.len().saturating_sub(1));
        for pair in edges.windows(2) {
            let start = pair[0];
            let end = pair[1];
            if end <= start {
                continue;
            }

            let scale = monitors
                .iter()
                .filter(|monitor| monitor_axis_overlaps(monitor, axis, start, end))
                .map(|monitor| {
                    let frame = frames.iter().find(|frame| frame.monitor_id == monitor.id);
                    monitor_axis_scale(monitor, frame, axis)
                })
                .filter(|scale| scale.is_finite() && *scale > 0.0)
                .fold(0.0_f32, f32::max);
            let scale = if scale > 0.0 { scale } else { 1.0 };
            segments.push(AxisSegment {
                start,
                end,
                offset,
                scale,
            });
            offset = offset.saturating_add(scaled_axis_length(end - start, scale));
        }

        if segments.is_empty() {
            bail!("显示器坐标轴没有有效区间");
        }

        Ok(Self { segments })
    }

    fn map(&self, coordinate: i32) -> u32 {
        let mut last_end = 0_u32;
        for segment in &self.segments {
            if coordinate < segment.start {
                return segment.offset;
            }
            let segment_end = segment.offset.saturating_add(scaled_axis_length(
                segment.end - segment.start,
                segment.scale,
            ));
            if coordinate <= segment.end {
                return segment.offset.saturating_add(scaled_axis_length(
                    coordinate - segment.start,
                    segment.scale,
                ));
            }
            last_end = segment_end;
        }
        last_end
    }
}

pub(super) fn compose_desktop_image(
    monitors: &[MonitorInfo],
    frames: &[FrozenFrame],
) -> Result<(Vec<u8>, u32, u32)> {
    if monitors.is_empty() || frames.is_empty() {
        bail!("没有可用截图帧");
    }

    let x_mapper = AxisMapper::from_frames(monitors, frames, Axis::X)?;
    let y_mapper = AxisMapper::from_frames(monitors, frames, Axis::Y)?;
    let width = frames
        .iter()
        .filter_map(|frame| {
            let monitor = monitors.iter().find(|m| m.id == frame.monitor_id)?;
            let x = x_mapper.map(monitor.rect.x);
            Some(x.saturating_add(frame.width))
        })
        .max()
        .unwrap_or(1);
    let height = frames
        .iter()
        .filter_map(|frame| {
            let monitor = monitors.iter().find(|m| m.id == frame.monitor_id)?;
            let y = y_mapper.map(monitor.rect.y);
            Some(y.saturating_add(frame.height))
        })
        .max()
        .unwrap_or(1);

    if width == 0 || height == 0 {
        bail!("组合截图为空");
    }

    let mut canvas = ImageBuffer::from_pixel(width, height, image::Rgba([0, 0, 0, 0]));
    for frame in frames {
        let Some(monitor) = monitors.iter().find(|m| m.id == frame.monitor_id) else {
            continue;
        };
        let x = x_mapper.map(monitor.rect.x);
        let y = y_mapper.map(monitor.rect.y);
        let Some(frame_image) = RgbaImage::from_raw(frame.width, frame.height, frame.rgba.to_vec())
        else {
            log::warn!(
                "截图帧尺寸和像素数据不匹配，跳过 monitor {}",
                frame.monitor_id
            );
            continue;
        };
        image::imageops::overlay(&mut canvas, &frame_image, i64::from(x), i64::from(y));
    }

    Ok((canvas.into_raw(), width, height))
}

fn monitor_axis_bounds(monitor: &MonitorInfo, axis: Axis) -> (i32, u32) {
    match axis {
        Axis::X => (monitor.rect.x, monitor.rect.width),
        Axis::Y => (monitor.rect.y, monitor.rect.height),
    }
}

fn monitor_axis_overlaps(monitor: &MonitorInfo, axis: Axis, start: i32, end: i32) -> bool {
    let (monitor_start, monitor_length) = monitor_axis_bounds(monitor, axis);
    let monitor_end = monitor_start.saturating_add_unsigned(monitor_length);
    monitor_start < end && monitor_end > start
}

fn monitor_axis_scale(monitor: &MonitorInfo, frame: Option<&FrozenFrame>, axis: Axis) -> f32 {
    if let Some(frame) = frame {
        let (_, monitor_length) = monitor_axis_bounds(monitor, axis);
        let frame_length = match axis {
            Axis::X => frame.width,
            Axis::Y => frame.height,
        };
        if monitor_length > 0 && frame_length > 0 {
            return frame_length as f32 / monitor_length as f32;
        }
        if frame.scale_factor.is_finite() && frame.scale_factor > 0.0 {
            return frame.scale_factor;
        }
    }

    monitor.scale_factor
}

fn scaled_axis_length(value: i32, scale_factor: f32) -> u32 {
    if value <= 0 {
        return 0;
    }
    ((value as f32) * scale_factor.max(f32::EPSILON)).round() as u32
}
