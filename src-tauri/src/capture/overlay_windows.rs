//! 截图覆盖层窗口的创建与生命周期。
//!
//! **不要用 Tauri 的 `position()` / `set_position()` / `set_size()` 来摆覆盖层。**
//! Wayland 协议不允许客户端决定自己的位置，GNOME 会直接忽略这些调用，窗口既不在正确的
//! 显示器上、也不是显示器尺寸，webview 背景透出来就是用户看到的"截图全黑"。
//! 参考 flashot（MIT）的做法：拿到底层 GTK 窗口，用 `fullscreen_on_monitor` 让**合成器**
//! 把窗口铺满目标显示器；目标显示器按"与冻结帧矩形重叠面积最大"从 GDK 显示器里挑，
//! 而不是相信我们自己传进去的坐标。

use super::error::CaptureError;
use super::manager::{CaptureManager, CaptureStart};
use super::types::OverlaySpec;
use crate::commands::AppState;
use tauri::Manager;

/// 前端迟迟不报告"首帧已画好"时的兜底显示时限。
///
/// 正常路径是 `mark_capture_overlay_ready` 把窗口显示出来；这个定时器只为覆盖
/// webview 加载失败或 JS 抛异常的情况——否则会留下一个隐藏但仍然占用会话的覆盖层，
/// 用户既看不到它，也没法按 Esc 取消。
const READY_FALLBACK_MS: u64 = 2500;

pub(super) fn create(
    app: &tauri::AppHandle,
    manager: &CaptureManager,
    start: &CaptureStart,
) -> Result<(), CaptureError> {
    create_checked_with(
        &start.overlays,
        || manager.ensure_current(&start.session_id),
        |spec| build_overlay(app, spec),
        |labels| close(app, labels),
        |specs| spawn_ready_fallback(app, specs),
    )
}

/// 创建期的可注入事务 seam。只有 liveness 失败会自行关闭本次已建的精确 labels；
/// build/configure 失败留给外层按精确 session id 认领并统一 cleanup。
pub(super) fn create_checked_with<E, L, B, C, F>(
    specs: &[OverlaySpec],
    mut ensure_current: L,
    mut build: B,
    mut close_created: C,
    register_fallback: F,
) -> Result<(), E>
where
    L: FnMut() -> Result<(), E>,
    B: FnMut(&OverlaySpec) -> Result<(), E>,
    C: FnMut(&[String]),
    F: FnOnce(&[OverlaySpec]),
{
    let mut created = Vec::new();
    ensure_current()?;
    for (index, spec) in specs.iter().enumerate() {
        if index > 0 {
            if let Err(error) = ensure_current() {
                close_created(&created);
                return Err(error);
            }
        }
        if let Err(build_error) = build(spec) {
            // configure 可能在窗口已经 build 后才失败；若这期间会话又被终结，
            // 外层已无资格 cleanup，因此本 invocation 必须收走可能存在的 attempted label。
            if ensure_current().is_err() {
                created.push(spec.label.clone());
                close_created(&created);
            }
            return Err(build_error);
        }
        created.push(spec.label.clone());
        if let Err(error) = ensure_current() {
            close_created(&created);
            return Err(error);
        }
    }
    register_fallback(specs);
    Ok(())
}

fn build_overlay(app: &tauri::AppHandle, spec: &OverlaySpec) -> Result<(), CaptureError> {
    let builder = tauri::WebviewWindowBuilder::new(
        app,
        &spec.label,
        tauri::WebviewUrl::App(format!("capture-overlay.html?label={}", spec.label).into()),
    )
    .title("")
    .position(spec.x as f64, spec.y as f64)
    .inner_size(spec.width as f64, spec.height as f64)
    .decorations(false)
    .skip_taskbar(true)
    .shadow(false)
    .resizable(false)
    .focused(false)
    // 窗口与 webview 的底色都设成不透明黑：webview 默认底色是白的，
    // 铺满整屏时任何一帧没画完的画面都是刺眼的白闪。
    .background_color(tauri::window::Color(0, 0, 0, 255))
    // 隐藏建窗，等前端把冻结帧画完再显示（见 `READY_FALLBACK_MS`）。
    .visible(false);
    #[cfg(target_os = "linux")]
    let builder = if crate::platform::is_wayland() {
        builder
    } else {
        builder.always_on_top(true)
    };
    #[cfg(not(target_os = "linux"))]
    let builder = builder.always_on_top(true);
    let window = builder
        .build()
        .map_err(|error| CaptureError::OverlayCreate(error.to_string()))?;
    configure_platform_overlay(&window, spec)?;
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
        if let Some((label, primary)) =
            reveal_fallback_with(labels, |label| -> Result<(), CaptureError> {
                let Some(window) = app.get_webview_window(label) else {
                    return Ok(());
                };
                // 查询失败时按"还没显示"处理：重复 show 无害，隐藏着的会话才是死局。
                if window.is_visible().unwrap_or(false) {
                    return Ok(());
                }
                log::warn!("覆盖层 {label} 超时未报告首帧，按兜底路径直接显示");
                super::action_lifecycle::complete_overlay_reveal(
                    window.show().map_err(CaptureError::window),
                    || {
                        let state = app.try_state::<AppState>().ok_or_else(|| {
                            CaptureError::StateLock("AppState 已不可用".to_string())
                        })?;
                        super::terminate_capture_overlay(&app, &state, label)
                    },
                )?;
                let _ = window.set_focus();
                Ok(())
            })
        {
            // 首次失败已经终结并关闭整个会话，不能再尝试后续旧 label。
            log::error!("覆盖层 {label} 兜底显示失败: {primary}");
        }
    });
}

/// fallback 的任一 show 失败会终结整个会话，因此后续 label 必须停止尝试。
fn reveal_fallback_with<E, F>(labels: Vec<String>, mut reveal: F) -> Option<(String, E)>
where
    F: FnMut(&str) -> Result<(), E>,
{
    for label in labels {
        if let Err(error) = reveal(&label) {
            return Some((label, error));
        }
    }
    None
}

/// 覆盖层的目标矩形，单位是逻辑像素，与 GDK 显示器几何同一个坐标系。
#[cfg(any(test, target_os = "linux"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OverlayRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[cfg(target_os = "linux")]
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
#[cfg(any(test, target_os = "linux"))]
pub(super) fn overlap_area(a: OverlayRect, b: OverlayRect) -> i64 {
    let left = a.x.max(b.x) as i64;
    let top = a.y.max(b.y) as i64;
    let right = (a.x as i64 + a.width as i64).min(b.x as i64 + b.width as i64);
    let bottom = (a.y as i64 + a.height as i64).min(b.y as i64 + b.height as i64);
    (right - left).max(0) * (bottom - top).max(0)
}

/// 在候选显示器里挑与目标矩形重叠面积最大的那个，重叠为 0 就返回 None（交给全屏兜底）。
#[cfg(any(test, target_os = "linux"))]
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
pub(in crate::capture) fn configure_platform_overlay(
    window: &tauri::WebviewWindow,
    spec: &OverlaySpec,
) -> Result<(), CaptureError> {
    use gtk::prelude::*;

    let gtk_window = window
        .gtk_window()
        .map_err(|error| CaptureError::OverlayCreate(error.to_string()))?;

    // 这些是 X11 窗口管理器提示；Wayland 不定义客户端全局置顶与跨工作区粘附，
    // 只在 X11 下设置，避免把不受支持的状态叠到合成器管理的全屏窗口上。
    if !crate::platform::is_wayland() {
        gtk_window.set_type_hint(gdk::WindowTypeHint::Splashscreen);
        gtk_window.set_keep_above(true);
        gtk_window.stick();
    }
    gtk_window.set_decorated(false);
    gtk_window.set_skip_taskbar_hint(true);

    let target = OverlayRect::from(spec);
    match (
        gtk::prelude::GtkWindowExt::screen(&gtk_window),
        gdk_monitor_for(&gtk_window, target),
    ) {
        (Some(screen), Some((index, geometry))) => {
            // **覆盖层绝不能比显示器大。** 全屏请求并不保证窗口被压到显示器尺寸：
            // 内容的最小尺寸比显示器大时，合成器给了 fullscreen 状态，GTK 仍按内容尺寸
            // 分配，于是窗口居中摆放、溢到隔壁显示器上，还把贴在右下的工具条推出屏幕外。
            // 冻结帧几何一旦算错（历史上就有过，见 `desktop_max_scale_factor`），
            // 症状就是"截图界面整体偏移 + 工具条不见了"，很难定位。
            // 这里以 GDK 的显示器几何为准兜一层，并把不一致大声记下来。
            if geometry.width != target.width || geometry.height != target.height {
                log::warn!(
                    "覆盖层 {} 的冻结帧几何 {}x{}@({},{}) 与 GDK 显示器 {}x{}@({},{}) 不一致，按显示器尺寸摆放",
                    spec.label,
                    target.width,
                    target.height,
                    target.x,
                    target.y,
                    geometry.width,
                    geometry.height,
                    geometry.x,
                    geometry.y,
                );
                gtk_window.resize(geometry.width.max(1) as i32, geometry.height.max(1) as i32);
            }
            gtk_window.fullscreen_on_monitor(&screen, index);
        }
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

/// 用 GDK 的显示器几何反查目标显示器序号与几何。
///
/// GDK 的几何是逻辑像素，和 `OverlaySpec` 同一坐标系，因此也是校验冻结帧几何的第二个信源。
#[cfg(target_os = "linux")]
fn gdk_monitor_for(
    gtk_window: &gtk::ApplicationWindow,
    target: OverlayRect,
) -> Option<(i32, OverlayRect)> {
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
    let index = best_monitor_index(target, &monitors)?;
    let geometry = *monitors.get(index as usize)?;
    Some((index, geometry))
}

#[cfg(not(target_os = "linux"))]
pub(in crate::capture) fn configure_platform_overlay(
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
    ["main", "launcher"]
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

pub(crate) fn restore(app: &tauri::AppHandle, labels: &[String]) {
    for label in labels {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

pub(crate) fn close(app: &tauri::AppHandle, labels: &[String]) {
    for label in labels {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{CaptureMode, CaptureModeGate};
    use std::cell::{Cell, RefCell};
    use std::sync::Arc;

    fn spec(label: &str) -> OverlaySpec {
        OverlaySpec {
            label: label.to_string(),
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        }
    }

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

    #[test]
    fn liveness_loss_stops_build_closes_only_created_labels_and_skips_fallback() {
        let specs = [spec("old-a"), spec("old-b")];
        let checks = Cell::new(0);
        let builds = RefCell::new(Vec::new());
        let closed = RefCell::new(Vec::new());
        let fallbacks = Cell::new(0);

        let result = create_checked_with(
            &specs,
            || {
                let call = checks.get() + 1;
                checks.set(call);
                if call == 2 {
                    Err("session_missing")
                } else {
                    Ok(())
                }
            },
            |spec| {
                builds.borrow_mut().push(spec.label.clone());
                Ok(())
            },
            |labels| closed.borrow_mut().extend_from_slice(labels),
            |_| fallbacks.set(fallbacks.get() + 1),
        );

        assert_eq!(result.unwrap_err(), "session_missing");
        assert_eq!(*builds.borrow(), ["old-a"]);
        assert_eq!(*closed.borrow(), ["old-a"]);
        assert_eq!(fallbacks.get(), 0);
    }

    #[test]
    fn build_failure_is_left_for_the_exact_session_cleanup_owner() {
        let specs = [spec("a"), spec("b")];
        let builds = Cell::new(0);
        let closed = Cell::new(0);
        let fallbacks = Cell::new(0);

        let result = create_checked_with(
            &specs,
            || Ok::<_, &'static str>(()),
            |_| {
                let call = builds.get() + 1;
                builds.set(call);
                if call == 2 {
                    Err("overlay_create")
                } else {
                    Ok(())
                }
            },
            |_| closed.set(closed.get() + 1),
            |_| fallbacks.set(fallbacks.get() + 1),
        );

        assert_eq!(result.unwrap_err(), "overlay_create");
        assert_eq!(builds.get(), 2);
        assert_eq!(closed.get(), 0);
        assert_eq!(fallbacks.get(), 0);
    }

    #[test]
    fn build_failure_after_liveness_loss_closes_created_and_attempted_labels() {
        let specs = [spec("old-a"), spec("old-b")];
        let checks = Cell::new(0);
        let builds = Cell::new(0);
        let closed = RefCell::new(Vec::new());

        let result = create_checked_with(
            &specs,
            || {
                let call = checks.get() + 1;
                checks.set(call);
                if call == 4 {
                    Err("session_missing")
                } else {
                    Ok(())
                }
            },
            |_| {
                let call = builds.get() + 1;
                builds.set(call);
                if call == 2 {
                    Err("overlay_create")
                } else {
                    Ok(())
                }
            },
            |labels| closed.borrow_mut().extend_from_slice(labels),
            |_| panic!("失败后不得注册 fallback"),
        );

        assert_eq!(result.unwrap_err(), "overlay_create");
        assert_eq!(builds.get(), 2);
        assert_eq!(*closed.borrow(), ["old-a", "old-b"]);
    }

    #[test]
    fn fallback_show_failure_never_attempts_later_closed_labels() {
        let gate = Arc::new(CaptureModeGate::new());
        let slot = RefCell::new(Some(
            gate.try_claim_owned(CaptureMode::Ordinary)
                .expect("测试应取得 Ordinary"),
        ));
        let events = RefCell::new(Vec::new());
        let failed = reveal_fallback_with(
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            |label| {
                events.borrow_mut().push(format!("show:{label}"));
                let show_result = if label == "b" {
                    Err::<(), _>("show error")
                } else {
                    Ok(())
                };
                super::super::action_lifecycle::complete_overlay_reveal(show_result, || {
                    super::super::action_lifecycle::finish_capture_session(
                        || Ok::<_, &str>(slot.borrow_mut().take()),
                        |_| events.borrow_mut().push("close".to_string()),
                        |_| events.borrow_mut().push("restore".to_string()),
                        |ownership| {
                            events.borrow_mut().push("finalize".to_string());
                            ownership.release().map_err(|_| "finalize error")
                        },
                    )
                })
            },
        );

        assert_eq!(failed, Some(("b".to_string(), "show error")));
        assert_eq!(
            *events.borrow(),
            ["show:a", "show:b", "close", "restore", "finalize"]
        );
        assert_eq!(gate.active_mode().unwrap(), None);
    }
}
