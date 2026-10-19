use crate::commands::AppState;
use crate::models::MainWindowPosition;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager, Monitor, PhysicalPosition, PhysicalSize, Position, Size};

const EDGE_MARGIN: i32 = 12;
const MAIN_WINDOW_WILL_HIDE: &str = "main-window-will-hide";
#[cfg(target_os = "linux")]
const SETTINGS_WINDOW_BUILD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
pub(crate) const MAIN_WINDOW_BASE_WIDTH: f64 = 380.0;
pub(crate) const MAIN_WINDOW_PANEL_WIDTH: f64 = 400.0;
/// 高度对所有面板组合恒定：开关预览会连带改变列表能显示的行数，
/// 列表跟着重排比"翻译区挤一点"更难用。翻译区靠 `.translation-host`
/// 的高度上限与自身滚动在这 500 里落位（见 styles/components.css）。
pub(crate) const MAIN_WINDOW_HEIGHT: f64 = 500.0;
const POSITION_SAVE_DEBOUNCE_MS: u64 = 300;
type MonitorTarget = Option<(Monitor, Option<PhysicalPosition<f64>>)>;

#[cfg(test)]
mod target_tests;

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

    if let Some((monitor, raw)) =
        select_main_window_target(remembered_target(&window, remembered, layout), || {
            Ok(target_monitor(app, &window)?.map(|(monitor, cursor)| {
                let work = WorkArea::from_monitor(&monitor);
                let size =
                    work.clamp_size(layout.physical_size(monitor.scale_factor()), EDGE_MARGIN);
                let raw = cursor
                    .map(|position| {
                        PhysicalPosition::new(
                            position.x.round() as i32 + EDGE_MARGIN,
                            position.y.round() as i32 + EDGE_MARGIN,
                        )
                    })
                    .unwrap_or_else(|| work.top_right(size, EDGE_MARGIN));
                (monitor, raw)
            }))
        })?
    {
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

fn select_main_window_target<T, E>(
    remembered: Result<Option<T>, E>,
    fallback: impl FnOnce() -> Result<Option<T>, E>,
) -> Result<Option<T>, E> {
    // 已有保存目标时不访问后备原生查询，避免无关的后备错误阻止显示。
    match remembered? {
        Some(target) => Ok(Some(target)),
        None => fallback(),
    }
}

/// 显式隐藏主窗口前先通知 WebView 释放长列表、预览和翻译快照。
///
/// `window.hide()` 不保证产生新的 blur：窗口可能已经失去焦点，或由 D-Bus/托盘直接切换。
/// 只依赖前端 blur 会让最多 10,000 条列表在隐藏后继续常驻。事件投递失败不应阻止窗口隐藏，
/// 但要留下诊断线索；前端清理本身是幂等的，和正常 blur 重复执行没有副作用。
pub(crate) fn hide_main_window(app: &tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "找不到 main 窗口".to_string())?;
    if let Err(error) = app.emit_to("main", MAIN_WINDOW_WILL_HIDE, ()) {
        log::warn!("通知主窗口释放隐藏态内存失败: {error}");
    }
    window.hide().map_err(|error| error.to_string())
}

/// 打开（或重开）设置窗口。托盘菜单和 IPC 命令共用这一处，
/// 标题按界面语言取，避免两边各写一份几何与文案。
pub(crate) fn open_settings_window(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("settings") {
        window.unminimize().map_err(|error| error.to_string())?;
        window.show().map_err(|error| error.to_string())?;
        return window.set_focus().map_err(|error| error.to_string());
    }

    #[cfg(target_os = "linux")]
    {
        build_settings_with_optional_related(
            app.get_webview_window("main"),
            |main| build_related_settings_window(app, main),
            || build_independent_settings_window(app),
        )
    }

    #[cfg(not(target_os = "linux"))]
    build_independent_settings_window(app)
}

fn build_independent_settings_window(app: &tauri::AppHandle) -> Result<(), String> {
    build_settings_window(
        tauri::WebviewWindowBuilder::new(
            app,
            "settings",
            tauri::WebviewUrl::App("settings.html".into()),
        ),
        native_text(app).settings_title,
    )
}

fn build_settings_window<'a, R, M>(
    builder: tauri::WebviewWindowBuilder<'a, R, M>,
    title: &'static str,
) -> Result<(), String>
where
    R: tauri::Runtime,
    M: tauri::Manager<R>,
{
    builder
        .title(title)
        .inner_size(720.0, 560.0)
        .min_inner_size(480.0, 400.0)
        .center()
        .resizable(true)
        .decorations(false)
        .build()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "linux")]
fn build_related_settings_window(
    app: &tauri::AppHandle,
    main: tauri::WebviewWindow,
) -> Result<(), String> {
    let app_handle = app.clone();
    let title = native_text(app).settings_title;

    dispatch_settings_window_build(
        SETTINGS_WINDOW_BUILD_TIMEOUT,
        move |callback| {
            main.with_webview(callback)
                .map_err(|error| error.to_string())
        },
        move |webview| {
            build_settings_window(
                tauri::WebviewWindowBuilder::new(
                    &app_handle,
                    "settings",
                    tauri::WebviewUrl::App("settings.html".into()),
                )
                .with_related_view(webview.inner()),
                title,
            )
        },
    )
}

#[cfg(any(target_os = "linux", test))]
fn build_settings_with_optional_related<T>(
    related: Option<T>,
    build_related: impl FnOnce(T) -> Result<(), String>,
    build_independent: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    match related {
        Some(related) => build_related(related),
        None => build_independent(),
    }
}

#[cfg(any(target_os = "linux", test))]
fn dispatch_settings_window_build<T: 'static>(
    timeout: std::time::Duration,
    dispatch: impl FnOnce(Box<dyn FnOnce(T) + Send + 'static>) -> Result<(), String>,
    build: impl FnOnce(T) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    dispatch(Box::new(move |context| {
        // 调用方可能已经因超时离开；迟到结果只应被丢弃，不能触发独立窗口回退。
        let _ = sender.send(build(context));
    }))
    .map_err(|error| format!("安排设置窗口关联创建失败: {error}"))?;

    match receiver.recv_timeout(timeout) {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            Err("等待设置窗口关联创建超时".to_string())
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            Err("设置窗口关联创建结果通道已断开".to_string())
        }
    }
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
    let remembered = MainWindowPosition {
        x: position.x,
        y: position.y,
    };
    let Ok(mut pending) = state.main_window_position_pending.lock() else {
        return;
    };
    *pending = Some(remembered);
    drop(pending);
    let generation = state
        .main_window_position_generation
        .fetch_add(1, Ordering::AcqRel)
        + 1;
    // 一次拖动会以 60 Hz 左右产生 Moved；旧实现每个事件都创建一个睡眠线程。
    // 这里只让第一个事件启动 worker，其余事件仅覆盖 pending 并推进代次。
    if !claim_position_save_worker(&state.main_window_position_worker_scheduled) {
        return;
    }
    std::thread::spawn(move || {
        let mut observed = generation;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(POSITION_SAVE_DEBOUNCE_MS));
            let Some(state) = app_handle.try_state::<AppState>() else {
                return;
            };
            let current = state
                .main_window_position_generation
                .load(Ordering::Acquire);
            if current != observed {
                observed = current;
                continue;
            }

            let remembered = state
                .main_window_position_pending
                .lock()
                .ok()
                .and_then(|mut pending| pending.take());
            if let Some(remembered) = remembered {
                match state.config.lock() {
                    Ok(mut config) if config.main_window_position != Some(remembered) => {
                        let mut updated = config.clone();
                        updated.main_window_position = Some(remembered);
                        match crate::config::save_config(&state.config_path, &updated) {
                            Ok(()) => *config = updated,
                            Err(error) => log::warn!("保存主窗口位置失败: {error}"),
                        }
                    }
                    Ok(_) => {}
                    Err(error) => log::warn!("保存主窗口位置时读取配置失败: {error}"),
                }
            }

            state
                .main_window_position_worker_scheduled
                .store(false, Ordering::Release);
            let latest = state
                .main_window_position_generation
                .load(Ordering::Acquire);
            if latest == current {
                return;
            }
            // 新事件可能恰好发生在 store(false) 前后。谁先把标志从 false 改回 true，
            // 谁负责下一轮；CAS 失败说明新 worker 已经接手，本线程直接退出。
            if !claim_position_save_worker(&state.main_window_position_worker_scheduled) {
                return;
            }
            observed = latest;
        }
    });
}

fn claim_position_save_worker(scheduled: &AtomicBool) -> bool {
    scheduled
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
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
    fn settings_window_uses_independent_builder_when_main_is_missing() {
        let mut used_independent = false;
        build_settings_with_optional_related(
            None::<()>,
            |_| panic!("主窗口缺失时不应调用关联 builder"),
            || {
                used_independent = true;
                Ok(())
            },
        )
        .unwrap();
        assert!(used_independent);
    }

    #[test]
    fn settings_window_prefers_related_builder_when_main_exists() {
        let mut used_related = false;
        let mut used_independent = false;
        build_settings_with_optional_related(
            Some(()),
            |_| {
                used_related = true;
                Ok(())
            },
            || {
                used_independent = true;
                Ok(())
            },
        )
        .unwrap();
        assert!(used_related);
        assert!(!used_independent);
    }

    #[test]
    fn settings_window_build_accepts_synchronous_dispatch() {
        let result = dispatch_settings_window_build(
            std::time::Duration::from_millis(50),
            |callback| {
                callback(41_u8);
                Ok(())
            },
            |value| {
                assert_eq!(value, 41);
                Ok(())
            },
        );
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn settings_window_build_waits_for_asynchronous_dispatch() {
        let mut worker = None;
        let result = dispatch_settings_window_build(
            std::time::Duration::from_secs(1),
            |callback| {
                worker = Some(std::thread::spawn(move || callback(42_u8)));
                Ok(())
            },
            |value| {
                assert_eq!(value, 42);
                Ok(())
            },
        );
        worker.unwrap().join().unwrap();
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn settings_window_build_reports_dispatch_failure() {
        let result = dispatch_settings_window_build(
            std::time::Duration::from_millis(50),
            |_callback: Box<dyn FnOnce(()) + Send>| Err("队列已关闭".to_string()),
            |_| Ok(()),
        );
        assert_eq!(
            result,
            Err("安排设置窗口关联创建失败: 队列已关闭".to_string())
        );
    }

    #[test]
    fn settings_window_build_reports_builder_failure() {
        let result = dispatch_settings_window_build(
            std::time::Duration::from_millis(50),
            |callback| {
                callback(());
                Ok(())
            },
            |_| Err("WebView 创建失败".to_string()),
        );
        assert_eq!(result, Err("WebView 创建失败".to_string()));
    }

    #[test]
    fn settings_window_build_times_out_without_fallback() {
        let mut worker = None;
        let result = dispatch_settings_window_build(
            std::time::Duration::from_millis(5),
            |callback| {
                worker = Some(std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(30));
                    callback(())
                }));
                Ok(())
            },
            |_| Ok(()),
        );
        worker.unwrap().join().unwrap();
        assert_eq!(result, Err("等待设置窗口关联创建超时".to_string()));
    }

    #[test]
    fn moved_event_burst_claims_only_one_position_save_worker() {
        let scheduled = AtomicBool::new(false);
        assert!(claim_position_save_worker(&scheduled));
        for _ in 0..500 {
            assert!(!claim_position_save_worker(&scheduled));
        }
        scheduled.store(false, Ordering::Release);
        assert!(claim_position_save_worker(&scheduled));
    }

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
        // 面板只影响宽度：高度恒定，否则开关预览会连带改变列表可见行数
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
                preview_visible: false,
                codec_visible: true,
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
            // 宽度被工作区收窄，高度 500 本来就放得下
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
