use crate::commands::AppState;
use crate::models::MainWindowPosition;
use std::sync::atomic::Ordering;
use tauri::{Manager, Monitor, PhysicalPosition, PhysicalSize, Position, Size};

const EDGE_MARGIN: i32 = 12;
pub(crate) const MAIN_WINDOW_BASE_WIDTH: f64 = 380.0;
pub(crate) const MAIN_WINDOW_PANEL_WIDTH: f64 = 400.0;
pub(crate) const MAIN_WINDOW_HEIGHT: f64 = 500.0;
const POSITION_SAVE_DEBOUNCE_MS: u64 = 300;
type MonitorTarget = Option<(Monitor, Option<PhysicalPosition<f64>>)>;

pub fn show_main_window(app: &tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "找不到 main 窗口".to_string())?;
    let _ = window.set_decorations(false);
    let _ = window.set_always_on_top(true);
    let layout = app
        .try_state::<AppState>()
        .map(|state| MainWindowLayout::from_state(&state))
        .transpose()?
        .unwrap_or_default();
    let remembered = app
        .try_state::<AppState>()
        .and_then(|state| state.config.lock().ok()?.main_window_position);

    if let Some((monitor, raw)) = remembered_target(&window, remembered, layout)?.or(
        target_monitor(app, &window)?.map(|(monitor, cursor)| {
            let work = WorkArea::from_monitor(&monitor);
            let size = work.clamp_size(layout.physical_size(monitor.scale_factor()), EDGE_MARGIN);
            let raw = cursor
                .map(|position| {
                    PhysicalPosition::new(
                        position.x.round() as i32 + EDGE_MARGIN,
                        position.y.round() as i32 + EDGE_MARGIN,
                    )
                })
                .unwrap_or_else(|| work.top_right(size, EDGE_MARGIN));
            (monitor, raw)
        }),
    ) {
        let work = WorkArea::from_monitor(&monitor);
        let size = layout.physical_size(monitor.scale_factor());
        let size = work.clamp_size(size, EDGE_MARGIN);
        set_fixed_size(&window, size)?;
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

/// 打开（或重开）设置窗口。托盘菜单和 IPC 命令共用这一处，
/// 标题按界面语言取，避免两边各写一份几何与文案。
pub(crate) fn open_settings_window(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.close();
    }
    tauri::WebviewWindowBuilder::new(
        app,
        "settings",
        tauri::WebviewUrl::App("settings.html".into()),
    )
    .title(native_text(app).settings_title)
    .inner_size(720.0, 560.0)
    .min_inner_size(480.0, 400.0)
    .center()
    .resizable(true)
    .build()
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// managed state 尚未注入时（例如 setup 早期失败路径）退回英文文案。
pub(crate) fn native_text(app: &tauri::AppHandle) -> crate::i18n::NativeText {
    app.try_state::<AppState>()
        .map(|state| state.native_text())
        .unwrap_or_else(|| crate::i18n::native_text(crate::i18n::Locale::En))
}

pub(crate) fn remember_main_window_position(
    window: &tauri::Window,
    position: PhysicalPosition<i32>,
) {
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    let Ok(monitors) = window.available_monitors() else {
        return;
    };
    let work_areas: Vec<_> = monitors.iter().map(WorkArea::from_monitor).collect();
    if !is_valid_remembered_position(position, &work_areas) {
        return;
    }
    let app_handle = window.app_handle().clone();
    let Some(state) = app_handle.try_state::<AppState>() else {
        return;
    };
    let generation = state
        .main_window_position_generation
        .fetch_add(1, Ordering::AcqRel)
        + 1;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(POSITION_SAVE_DEBOUNCE_MS));
        let Some(state) = app_handle.try_state::<AppState>() else {
            return;
        };
        if state
            .main_window_position_generation
            .load(Ordering::Acquire)
            != generation
        {
            return;
        }
        let mut config = match state.config.lock() {
            Ok(config) => config,
            Err(error) => {
                log::warn!("保存主窗口位置时读取配置失败: {error}");
                return;
            }
        };
        let remembered = MainWindowPosition {
            x: position.x,
            y: position.y,
        };
        if config.main_window_position == Some(remembered) {
            return;
        }
        config.main_window_position = Some(remembered);
        crate::config::save_config(&state.config_path, &config);
    });
}

pub fn resize_main_window(app: &tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "找不到 main 窗口".to_string())?;
    let layout = app
        .try_state::<AppState>()
        .map(|state| MainWindowLayout::from_state(&state))
        .transpose()?
        .unwrap_or_default();
    let monitor = window
        .current_monitor()
        .map_err(|error| error.to_string())?
        .or(window
            .primary_monitor()
            .map_err(|error| error.to_string())?);
    let Some(monitor) = monitor else {
        let size = layout.logical_size();
        set_fixed_logical_size(&window, size)?;
        return Ok(());
    };

    let work = WorkArea::from_monitor(&monitor);
    let requested = layout.physical_size(monitor.scale_factor());
    let size = work.clamp_size(requested, EDGE_MARGIN);
    let current = window
        .outer_position()
        .unwrap_or_else(|_| work.top_right(size, EDGE_MARGIN));
    set_fixed_size(&window, size)?;
    window
        .set_position(Position::Physical(work.clamp_position(
            current,
            size,
            EDGE_MARGIN,
        )))
        .map_err(|error| error.to_string())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MainWindowLayout {
    pub preview_visible: bool,
    pub codec_visible: bool,
}

impl MainWindowLayout {
    fn from_state(state: &AppState) -> Result<Self, String> {
        Ok(Self {
            preview_visible: *state
                .preview_visible
                .lock()
                .map_err(|error| format!("读取预览面板状态失败: {error}"))?,
            codec_visible: *state
                .codec_visible
                .lock()
                .map_err(|error| format!("读取编解码面板状态失败: {error}"))?,
        })
    }

    pub(crate) fn logical_size(self) -> (f64, f64) {
        (
            MAIN_WINDOW_BASE_WIDTH
                + if self.preview_visible {
                    MAIN_WINDOW_PANEL_WIDTH
                } else {
                    0.0
                }
                + if self.codec_visible {
                    MAIN_WINDOW_PANEL_WIDTH
                } else {
                    0.0
                },
            MAIN_WINDOW_HEIGHT,
        )
    }

    fn physical_size(self, scale_factor: f64) -> PhysicalSize<u32> {
        let (width, height) = self.logical_size();
        let scale = scale_factor.max(0.1);
        PhysicalSize::new(
            (width * scale).round().max(1.0) as u32,
            (height * scale).round().max(1.0) as u32,
        )
    }
}

fn set_fixed_size(window: &tauri::WebviewWindow, size: PhysicalSize<u32>) -> Result<(), String> {
    let size = Size::Physical(size);
    clear_size_constraints(window)?;
    window.set_size(size).map_err(|error| error.to_string())?;
    window
        .set_min_size(Some(size))
        .map_err(|error| error.to_string())?;
    window
        .set_max_size(Some(size))
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn set_fixed_logical_size(window: &tauri::WebviewWindow, size: (f64, f64)) -> Result<(), String> {
    let size = Size::Logical(tauri::LogicalSize::new(size.0, size.1));
    clear_size_constraints(window)?;
    window.set_size(size).map_err(|error| error.to_string())?;
    window
        .set_min_size(Some(size))
        .map_err(|error| error.to_string())?;
    window
        .set_max_size(Some(size))
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn clear_size_constraints(window: &tauri::WebviewWindow) -> Result<(), String> {
    window
        .set_min_size(None::<Size>)
        .map_err(|error| error.to_string())?;
    window
        .set_max_size(None::<Size>)
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

fn remembered_target(
    window: &tauri::WebviewWindow,
    remembered: Option<MainWindowPosition>,
    layout: MainWindowLayout,
) -> Result<Option<(Monitor, PhysicalPosition<i32>)>, String> {
    let Some(remembered) = remembered else {
        return Ok(None);
    };
    let monitors = window
        .available_monitors()
        .map_err(|error| error.to_string())?;
    let work_areas: Vec<_> = monitors.iter().map(WorkArea::from_monitor).collect();
    let remembered = PhysicalPosition::new(remembered.x, remembered.y);
    let Some(index) = remembered_monitor_index(remembered, &work_areas) else {
        return Ok(None);
    };
    let monitor = monitors[index].clone();
    let work = work_areas[index];
    let size = work.clamp_size(layout.physical_size(monitor.scale_factor()), EDGE_MARGIN);
    Ok(Some((
        monitor,
        work.clamp_position(remembered, size, EDGE_MARGIN),
    )))
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

    fn contains(self, position: PhysicalPosition<i32>) -> bool {
        position.x >= self.left
            && position.x < self.right
            && position.y >= self.top
            && position.y < self.bottom
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

fn remembered_monitor_index(
    position: PhysicalPosition<i32>,
    work_areas: &[WorkArea],
) -> Option<usize> {
    work_areas.iter().position(|work| work.contains(position))
}

fn is_valid_remembered_position(position: PhysicalPosition<i32>, work_areas: &[WorkArea]) -> bool {
    remembered_monitor_index(position, work_areas).is_some()
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

    #[test]
    fn main_window_layout_uses_base_and_visible_panel_widths() {
        assert_eq!(MainWindowLayout::default().logical_size(), (380.0, 500.0));
        assert_eq!(
            (MainWindowLayout {
                preview_visible: true,
                codec_visible: false,
            })
            .logical_size(),
            (780.0, 500.0)
        );
        assert_eq!(
            (MainWindowLayout {
                preview_visible: true,
                codec_visible: true,
            })
            .logical_size(),
            (1180.0, 500.0)
        );
    }

    #[test]
    fn main_window_layout_scales_and_clamps_to_work_area() {
        let layout = MainWindowLayout {
            preview_visible: true,
            codec_visible: true,
        };
        assert_eq!(layout.physical_size(1.0), PhysicalSize::new(1180, 500));
        assert_eq!(
            WorkArea {
                left: 0,
                top: 0,
                right: 1000,
                bottom: 600,
            }
            .clamp_size(layout.physical_size(1.0), EDGE_MARGIN),
            PhysicalSize::new(976, 500)
        );
    }

    #[test]
    fn remembered_position_selects_negative_origin_monitor_and_clamps() {
        let work_areas = [
            WorkArea {
                left: -1920,
                top: 0,
                right: 0,
                bottom: 1080,
            },
            WorkArea {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
        ];
        let remembered = PhysicalPosition::new(-100, 900);

        let index = remembered_monitor_index(remembered, &work_areas).unwrap();
        assert_eq!(index, 0);
        assert_eq!(
            work_areas[index].clamp_position(remembered, PhysicalSize::new(380, 500), EDGE_MARGIN,),
            PhysicalPosition::new(-392, 568)
        );
    }

    #[test]
    fn remembered_position_rejects_removed_monitor() {
        let work_areas = [WorkArea {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        }];
        assert_eq!(
            remembered_monitor_index(PhysicalPosition::new(-800, 200), &work_areas),
            None
        );
        assert!(!is_valid_remembered_position(
            PhysicalPosition::new(i32::MAX, i32::MAX),
            &work_areas,
        ));
    }
}
