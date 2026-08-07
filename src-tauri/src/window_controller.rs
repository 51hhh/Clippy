use tauri::{Manager, Monitor, PhysicalPosition, PhysicalSize, Position, Size};

const EDGE_MARGIN: i32 = 12;
type MonitorTarget = Option<(Monitor, Option<PhysicalPosition<f64>>)>;

pub fn show_main_window(app: &tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "找不到 main 窗口".to_string())?;
    let _ = window.set_decorations(false);
    let _ = window.set_always_on_top(true);

    if let Some((monitor, cursor)) = target_monitor(app, &window)? {
        let work = WorkArea::from_monitor(&monitor);
        let size = window.outer_size().map_err(|error| error.to_string())?;
        let size = work.clamp_size(size, EDGE_MARGIN);
        window
            .set_size(Size::Physical(size))
            .map_err(|error| error.to_string())?;
        let raw = cursor
            .map(|position| {
                PhysicalPosition::new(
                    position.x.round() as i32 + EDGE_MARGIN,
                    position.y.round() as i32 + EDGE_MARGIN,
                )
            })
            .unwrap_or_else(|| work.top_right(size, EDGE_MARGIN));
        window
            .set_position(Position::Physical(work.clamp_position(
                raw,
                size,
                EDGE_MARGIN,
            )))
            .map_err(|error| error.to_string())?;
    }

    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())
}

pub fn resize_main_window(
    app: &tauri::AppHandle,
    logical_width: f64,
    logical_height: f64,
) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "找不到 main 窗口".to_string())?;
    let monitor = window
        .current_monitor()
        .map_err(|error| error.to_string())?
        .or(window
            .primary_monitor()
            .map_err(|error| error.to_string())?);
    let Some(monitor) = monitor else {
        return window
            .set_size(tauri::LogicalSize::new(logical_width, logical_height))
            .map_err(|error| error.to_string());
    };

    let work = WorkArea::from_monitor(&monitor);
    let scale = monitor.scale_factor().max(0.1);
    let requested = PhysicalSize::new(
        (logical_width * scale).round().max(1.0) as u32,
        (logical_height * scale).round().max(1.0) as u32,
    );
    let size = work.clamp_size(requested, EDGE_MARGIN);
    let current = window
        .outer_position()
        .unwrap_or_else(|_| work.top_right(size, EDGE_MARGIN));
    window
        .set_size(Size::Physical(size))
        .map_err(|error| error.to_string())?;
    window
        .set_position(Position::Physical(work.clamp_position(
            current,
            size,
            EDGE_MARGIN,
        )))
        .map_err(|error| error.to_string())
}

fn target_monitor(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
) -> Result<MonitorTarget, String> {
    let cursor = app.cursor_position().ok();
    if let Some(cursor) = cursor {
        if let Some(monitor) = app
            .monitor_from_point(cursor.x, cursor.y)
            .map_err(|error| error.to_string())?
        {
            return Ok(Some((monitor, Some(cursor))));
        }
    }
    if let Some(monitor) = window
        .current_monitor()
        .map_err(|error| error.to_string())?
    {
        return Ok(Some((monitor, cursor)));
    }
    Ok(window
        .primary_monitor()
        .map_err(|error| error.to_string())?
        .map(|monitor| (monitor, cursor)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WorkArea {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl WorkArea {
    fn from_monitor(monitor: &Monitor) -> Self {
        let work = monitor.work_area();
        Self {
            left: work.position.x,
            top: work.position.y,
            right: work.position.x + work.size.width as i32,
            bottom: work.position.y + work.size.height as i32,
        }
    }

    fn clamp_size(self, size: PhysicalSize<u32>, margin: i32) -> PhysicalSize<u32> {
        let width = (self.right - self.left - margin * 2).max(1) as u32;
        let height = (self.bottom - self.top - margin * 2).max(1) as u32;
        PhysicalSize::new(size.width.min(width), size.height.min(height))
    }

    fn top_right(self, size: PhysicalSize<u32>, margin: i32) -> PhysicalPosition<i32> {
        PhysicalPosition::new(self.right - size.width as i32 - margin, self.top + margin)
    }

    fn clamp_position(
        self,
        position: PhysicalPosition<i32>,
        size: PhysicalSize<u32>,
        margin: i32,
    ) -> PhysicalPosition<i32> {
        PhysicalPosition::new(
            clamp_axis(
                position.x,
                self.left + margin,
                self.right - margin,
                size.width as i32,
            ),
            clamp_axis(
                position.y,
                self.top + margin,
                self.bottom - margin,
                size.height as i32,
            ),
        )
    }
}

fn clamp_axis(value: i32, min: i32, max: i32, window_size: i32) -> i32 {
    let upper = max - window_size;
    if upper <= min {
        min
    } else {
        value.clamp(min, upper)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_position_to_negative_origin_work_area() {
        let work = WorkArea {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1080,
        };
        let size = PhysicalSize::new(760, 500);
        assert_eq!(
            work.clamp_position(PhysicalPosition::new(-100, 900), size, 12),
            PhysicalPosition::new(-772, 568)
        );
    }

    #[test]
    fn clamps_oversized_window_before_positioning() {
        let work = WorkArea {
            left: 0,
            top: 24,
            right: 1280,
            bottom: 720,
        };
        assert_eq!(
            work.clamp_size(PhysicalSize::new(1600, 900), 12),
            PhysicalSize::new(1256, 672)
        );
    }
}
