//! 截图覆盖层窗口的创建与生命周期。
//!
//! **不要用 Tauri 的 `position()` / `set_position()` / `set_size()` 来摆覆盖层。**
//! Wayland 协议不允许客户端决定自己的位置，GNOME 会直接忽略这些调用，窗口既不在正确的
//! 显示器上、也不是显示器尺寸，webview 背景透出来就是用户看到的"截图全黑"。
//! 参考 flashot（MIT）的做法：拿到底层 GTK 窗口，用 `fullscreen_on_monitor` 让**合成器**
//! 把窗口铺满目标显示器；目标显示器按"与冻结帧矩形重叠面积最大"从 GDK 显示器里挑，
//! 而不是相信我们自己传进去的坐标。

use super::error::CaptureError;
use super::types::OverlaySpec;
use tauri::Manager;

/// 前端迟迟不报告"首帧已画好"时的兜底显示时限。
///
/// 正常路径是 `mark_capture_overlay_ready` 把窗口显示出来；这个定时器只为覆盖
/// webview 加载失败或 JS 抛异常的情况——否则会留下一个隐藏但仍然占用会话的覆盖层，
/// 用户既看不到它，也没法按 Esc 取消。
const READY_FALLBACK_MS: u64 = 2500;

pub(super) fn create(app: &tauri::AppHandle, specs: &[OverlaySpec]) -> Result<(), CaptureError> {
    for spec in specs {
        let window = tauri::WebviewWindowBuilder::new(
            app,
            &spec.label,
            tauri::WebviewUrl::App(format!("capture-overlay.html?label={}", spec.label).into()),
        )
        .title("")
        .position(spec.x as f64, spec.y as f64)
        .inner_size(spec.width as f64, spec.height as f64)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(false)
        .resizable(false)
        .focused(false)
        // 窗口与 webview 的底色都设成不透明黑：webview 默认底色是白的，
        // 铺满整屏时任何一帧没画完的画面都是刺眼的白闪。
        .background_color(tauri::window::Color(0, 0, 0, 255))
        // 隐藏建窗，等前端把冻结帧画完再显示（见 `READY_FALLBACK_MS`）。
        .visible(false)
        .build()
        .map_err(|error| CaptureError::OverlayCreate(error.to_string()))?;
        configure_platform_overlay(&window, spec)?;
    }
    spawn_ready_fallback(app, specs);
    Ok(())
}

/// 前端报告首帧画好之后把覆盖层显示出来。`take_focus` 由 `CaptureManager::reveal` 决定。
pub(super) fn reveal(
    app: &tauri::AppHandle,
    label: &str,
    take_focus: bool,
) -> Result<(), CaptureError> {
    let Some(window) = app.get_webview_window(label) else {
        return Err(CaptureError::OverlayNotInSession);
    };
    window.show().map_err(CaptureError::window)?;
    // 覆盖层要吃键盘（Esc 取消、Enter 提交），所以必须有一块拿到焦点。
    if take_focus {
        let _ = window.set_focus();
    }
    Ok(())
}

fn spawn_ready_fallback(app: &tauri::AppHandle, specs: &[OverlaySpec]) {
    let app = app.clone();
    let labels: Vec<String> = specs.iter().map(|spec| spec.label.clone()).collect();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(READY_FALLBACK_MS)).await;
        for label in labels {
            let Some(window) = app.get_webview_window(&label) else {
                continue;
            };
            // 查询失败时按"还没显示"处理：重复 show 无害，隐藏着的会话才是死局。
            if window.is_visible().unwrap_or(false) {
                continue;
            }
            log::warn!("覆盖层 {label} 超时未报告首帧，按兜底路径直接显示");
            let _ = window.show();
            let _ = window.set_focus();
        }
    });
}

/// 覆盖层的目标矩形，单位是逻辑像素，与 GDK 显示器几何同一个坐标系。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OverlayRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl From<&OverlaySpec> for OverlayRect {
    fn from(spec: &OverlaySpec) -> Self {
        Self {
            x: spec.x,
            y: spec.y,
            width: spec.width,
            height: spec.height,
        }
    }
}

/// 两个矩形的重叠面积。用 i64 是因为 4K 多屏下 `width * height` 会接近 i32 上限。
pub(super) fn overlap_area(a: OverlayRect, b: OverlayRect) -> i64 {
    let left = a.x.max(b.x) as i64;
    let top = a.y.max(b.y) as i64;
    let right = (a.x as i64 + a.width as i64).min(b.x as i64 + b.width as i64);
    let bottom = (a.y as i64 + a.height as i64).min(b.y as i64 + b.height as i64);
    (right - left).max(0) * (bottom - top).max(0)
}

/// 在候选显示器里挑与目标矩形重叠面积最大的那个，重叠为 0 就返回 None（交给全屏兜底）。
pub(super) fn best_monitor_index(target: OverlayRect, monitors: &[OverlayRect]) -> Option<i32> {
    monitors
        .iter()
        .enumerate()
        .map(|(index, monitor)| (index as i32, overlap_area(target, *monitor)))
        .filter(|(_, area)| *area > 0)
        .max_by_key(|(_, area)| *area)
        .map(|(index, _)| index)
}

#[cfg(target_os = "linux")]
fn configure_platform_overlay(
    window: &tauri::WebviewWindow,
    spec: &OverlaySpec,
) -> Result<(), CaptureError> {
    use gtk::prelude::*;

    let gtk_window = window
        .gtk_window()
        .map_err(|error| CaptureError::OverlayCreate(error.to_string()))?;

    // X11 下这些提示决定覆盖层能不能盖住面板、不进任务栏、跟随工作区；
    // Wayland 会忽略它们，但设置本身无害，两条路走同一段代码。
    gtk_window.set_type_hint(gdk::WindowTypeHint::Splashscreen);
    gtk_window.set_decorated(false);
    gtk_window.set_skip_taskbar_hint(true);
    gtk_window.set_keep_above(true);
    gtk_window.stick();

    let target = OverlayRect::from(spec);
    match (
        gtk::prelude::GtkWindowExt::screen(&gtk_window),
        gdk_monitor_index_for(&gtk_window, target),
    ) {
        (Some(screen), Some(index)) => gtk_window.fullscreen_on_monitor(&screen, index),
        _ => {
            // 认不出目标显示器时退化成"当前显示器全屏"：尺寸一定对，多屏下可能选错屏，
            // 但比留一个错位的小窗口（用户看到的就是黑屏）好得多。
            log::warn!(
                "覆盖层 {} 无法匹配 GDK 显示器，退化为当前显示器全屏",
                spec.label
            );
            gtk_window.fullscreen();
        }
    }
    Ok(())
}

/// 用 GDK 的显示器几何反查目标显示器序号。GDK 的几何是逻辑像素，和 `OverlaySpec` 同一坐标系。
#[cfg(target_os = "linux")]
fn gdk_monitor_index_for(gtk_window: &gtk::ApplicationWindow, target: OverlayRect) -> Option<i32> {
    use gdk::prelude::MonitorExt;
    use gtk::prelude::*;

    let display = gtk_window.display();
    let monitors: Vec<OverlayRect> = (0..display.n_monitors())
        .map(|index| {
            display
                .monitor(index)
                .map(|monitor| {
                    let geometry = monitor.geometry();
                    OverlayRect {
                        x: geometry.x(),
                        y: geometry.y(),
                        width: geometry.width().max(0) as u32,
                        height: geometry.height().max(0) as u32,
                    }
                })
                // 读不到几何的显示器用空矩形占位，保住序号与 GDK 的一致。
                .unwrap_or(OverlayRect {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                })
        })
        .collect();
    best_monitor_index(target, &monitors)
}

#[cfg(not(target_os = "linux"))]
fn configure_platform_overlay(
    window: &tauri::WebviewWindow,
    spec: &OverlaySpec,
) -> Result<(), CaptureError> {
    window
        .set_position(tauri::LogicalPosition::new(spec.x as f64, spec.y as f64))
        .map_err(CaptureError::window)?;
    window
        .set_size(tauri::LogicalSize::new(
            spec.width as f64,
            spec.height as f64,
        ))
        .map_err(CaptureError::window)
}

pub(super) fn hide_sources(app: &tauri::AppHandle) -> Vec<String> {
    ["main"]
        .into_iter()
        .filter_map(|label| {
            let window = app.get_webview_window(label)?;
            if window.is_visible().unwrap_or(false) {
                let _ = window.hide();
                Some(label.to_string())
            } else {
                None
            }
        })
        .collect()
}

pub(super) fn restore(app: &tauri::AppHandle, labels: &[String]) {
    for label in labels {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

pub(super) fn close(app: &tauri::AppHandle, labels: &[String]) {
    for label in labels {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: u32, height: u32) -> OverlayRect {
        OverlayRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn overlap_area_is_zero_for_disjoint_rects() {
        assert_eq!(
            overlap_area(rect(0, 0, 100, 100), rect(200, 0, 100, 100)),
            0
        );
        assert_eq!(
            overlap_area(rect(0, 0, 100, 100), rect(0, 100, 100, 100)),
            0
        );
    }

    #[test]
    fn overlap_area_survives_4k_multi_monitor_without_overflowing() {
        // 单个 i32 相乘就会溢出的量级，必须走 i64。
        let huge = rect(0, 0, 61_440, 34_560);
        assert_eq!(overlap_area(huge, huge), 61_440i64 * 34_560);
    }

    #[test]
    fn best_monitor_picks_the_frame_owner_not_the_first_one() {
        let monitors = [rect(0, 0, 1920, 1200), rect(1920, 0, 2560, 1440)];
        assert_eq!(
            best_monitor_index(rect(1920, 0, 2560, 1440), &monitors),
            Some(1)
        );
        assert_eq!(
            best_monitor_index(rect(0, 0, 1920, 1200), &monitors),
            Some(0)
        );
    }

    #[test]
    fn best_monitor_prefers_the_largest_overlap_when_the_frame_straddles_two() {
        let monitors = [rect(0, 0, 1000, 1000), rect(1000, 0, 1000, 1000)];
        // 右屏占 700 列，左屏只占 300 列。
        assert_eq!(
            best_monitor_index(rect(700, 0, 1000, 1000), &monitors),
            Some(1)
        );
    }

    #[test]
    fn best_monitor_returns_none_so_the_caller_can_fall_back_to_plain_fullscreen() {
        let monitors = [rect(0, 0, 1920, 1200)];
        assert_eq!(
            best_monitor_index(rect(5000, 5000, 800, 600), &monitors),
            None
        );
        assert_eq!(best_monitor_index(rect(0, 0, 1920, 1200), &[]), None);
    }

    #[test]
    fn best_monitor_skips_placeholder_rects_without_shifting_indices() {
        // 读不到几何的显示器用空矩形占位，序号必须仍与 GDK 对齐。
        let monitors = [rect(0, 0, 0, 0), rect(0, 0, 1920, 1200)];
        assert_eq!(
            best_monitor_index(rect(0, 0, 1920, 1200), &monitors),
            Some(1)
        );
    }
}
