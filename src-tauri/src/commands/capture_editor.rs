use super::AppState;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use tauri::{LogicalSize, Manager, Size, State};

/// 编辑器外壳占掉的逻辑尺寸：左侧工具栏 260 + 分隔线与画布内边距；
/// 顶部标题栏 44 + 底部动作栏 52 + 状态条 32 + 画布上下内边距（见 styles/capture.css）。
const EDITOR_CHROME_WIDTH: f64 = 288.0;
const EDITOR_CHROME_HEIGHT: f64 = 152.0;
/// 侧栏工具太多，窗口再小就没法用了；与 min_inner_size 保持一致。
const EDITOR_MIN_WIDTH: f64 = 820.0;
const EDITOR_MIN_HEIGHT: f64 = 560.0;
/// 给窗口装饰和任务栏留边，全屏截图不至于把编辑器顶到屏幕外。
const EDITOR_WORK_AREA_MARGIN: f64 = 48.0;

/// 启动冻结屏幕截图覆盖层。
#[tauri::command]
pub async fn show_capture_editor(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    crate::capture::show_capture_overlay_for_app(app_handle, &state).await
}

pub(crate) async fn show_capture_editor_for_app(
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let state = app_handle.state::<AppState>();
    crate::capture::show_capture_overlay_for_app(app_handle.clone(), &state).await
}

/// 串行写入截图并打开编辑器，避免不同来源的 generation 与窗口事件交错。
pub(crate) fn queue_capture_for_editor(
    app_handle: &tauri::AppHandle,
    state: &AppState,
    png_base64: String,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let _transition = state
        .capture_editor_transition
        .lock()
        .map_err(|error| error.to_string())?;
    let generation = store_latest_capture(state, png_base64, width, height)?;
    state
        .capture_window_generation
        .store(generation, Ordering::Release);
    open_capture_window(app_handle, state, generation, (width, height))
}

/// 让编辑器窗口刚好容纳 1:1 的截图：物理像素先按缩放折算成逻辑尺寸，
/// 再加上外壳，最后夹在最小尺寸与工作区之间（工作区比最小尺寸还小时以工作区为准）。
fn editor_window_size(image: (u32, u32), scale_factor: f64, work_area: (f64, f64)) -> (f64, f64) {
    let scale = if scale_factor.is_finite() && scale_factor > 0.1 {
        scale_factor
    } else {
        1.0
    };
    let requested = (
        image.0 as f64 / scale + EDITOR_CHROME_WIDTH,
        image.1 as f64 / scale + EDITOR_CHROME_HEIGHT,
    );
    let limit = (
        (work_area.0 - EDITOR_WORK_AREA_MARGIN).max(1.0),
        (work_area.1 - EDITOR_WORK_AREA_MARGIN).max(1.0),
    );
    (
        requested.0.clamp(EDITOR_MIN_WIDTH.min(limit.0), limit.0),
        requested.1.clamp(EDITOR_MIN_HEIGHT.min(limit.1), limit.1),
    )
}

/// 取截图所在屏幕的缩放与工作区，算出编辑器窗口的逻辑尺寸；拿不到显示器时退回默认值。
fn editor_window_size_for_app(app_handle: &tauri::AppHandle, image: (u32, u32)) -> (f64, f64) {
    let monitor = app_handle
        .get_webview_window("capture")
        .and_then(|window| window.current_monitor().ok().flatten())
        .or_else(|| app_handle.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return (EDITOR_MIN_WIDTH, EDITOR_MIN_HEIGHT);
    };
    let scale = monitor.scale_factor();
    let work = monitor.work_area();
    let work_logical = if scale > 0.1 {
        (
            work.size.width as f64 / scale,
            work.size.height as f64 / scale,
        )
    } else {
        (work.size.width as f64, work.size.height as f64)
    };
    editor_window_size(image, scale, work_logical)
}

fn open_capture_window(
    app_handle: &tauri::AppHandle,
    state: &AppState,
    generation: u64,
    image: (u32, u32),
) -> Result<(), String> {
    let title = state.native_text().screenshot_title;
    // 窗口按截图尺寸开：参考项目截完图就是 1:1 呈现，缩放只是小屏的兜底。
    let (window_width, window_height) = editor_window_size_for_app(app_handle, image);
    let result = if let Some(window) = app_handle.get_webview_window("capture") {
        window
            .url()
            .map(|url| capture_window_url(url, Some(generation)))
            .map_err(|error| error.to_string())
            .and_then(|url| window.navigate(url).map_err(|error| error.to_string()))
            .and_then(|_| window.show().map_err(|error| error.to_string()))
            .map(|_| {
                // 复用已有窗口时语言可能已经变过，标题跟着刷新；尺寸按新截图重算。
                let _ = window.set_title(title);
                let _ =
                    window.set_size(Size::Logical(LogicalSize::new(window_width, window_height)));
                let _ = window.center();
                let _ = window.set_focus();
            })
    } else {
        tauri::WebviewWindowBuilder::new(
            app_handle,
            "capture",
            tauri::WebviewUrl::App(format!("capture.html?generation={generation}").into()),
        )
        .title(title)
        .inner_size(window_width, window_height)
        .min_inner_size(EDITOR_MIN_WIDTH, EDITOR_MIN_HEIGHT)
        .center()
        .resizable(true)
        .build()
        .map_err(|error| error.to_string())
        .map(|window| {
            let _ = window.set_focus();
        })
    };
    if result.is_err() {
        let _ = state.capture_window_generation.compare_exchange(
            generation,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        clear_capture_if_generation(&state.latest_capture, generation);
        reset_capture_window_page(app_handle);
    }
    result
}

fn capture_window_url(mut url: tauri::Url, generation: Option<u64>) -> tauri::Url {
    url.set_path("/capture.html");
    url.set_query(
        generation
            .map(|value| format!("generation={value}"))
            .as_deref(),
    );
    url.set_fragment(None);
    url
}

/// 关闭编辑器时由后端释放当前代次，前端是否已挂载都不会泄漏待编辑缓存。
pub(crate) fn release_capture_window(app_handle: &tauri::AppHandle, state: &AppState) {
    let _transition = match state.capture_editor_transition.lock() {
        Ok(transition) => transition,
        Err(poisoned) => poisoned.into_inner(),
    };
    release_capture_generation(&state.capture_window_generation, &state.latest_capture);
    reset_capture_window_page(app_handle);
}

fn reset_capture_window_page(app_handle: &tauri::AppHandle) {
    if let Some(window) = app_handle.get_webview_window("capture") {
        if let Ok(url) = window.url() {
            if let Err(error) = window.navigate(capture_window_url(url, None)) {
                log::warn!("重置截图编辑器空闲页失败: {}", error);
            }
        }
    }
}

fn release_capture_generation(
    capture_window_generation: &std::sync::atomic::AtomicU64,
    latest_capture: &Mutex<Option<crate::screenshot::CapturedScreenshot>>,
) {
    let generation = capture_window_generation.swap(0, Ordering::AcqRel);
    if generation > 0 {
        clear_capture_if_generation(latest_capture, generation);
    }
}

/// 返回最近一次截图编辑器待处理的截图。
#[tauri::command]
pub fn get_pending_capture(
    state: State<AppState>,
    generation: u64,
) -> Result<crate::screenshot::CapturedScreenshot, String> {
    read_capture_if_generation(&state.latest_capture, generation)
}

/// 写入最新截图并分配单调代次，用于避免旧编辑窗口清理新截图。
fn store_latest_capture(
    state: &AppState,
    png_base64: String,
    width: u32,
    height: u32,
) -> Result<u64, String> {
    let mut latest = state
        .latest_capture
        .lock()
        .map_err(|error| error.to_string())?;
    // 代次分配与替换必须受同一把锁排序，否则旧线程可能拿到较小代次却最后覆盖新截图。
    let generation = state.capture_generation.fetch_add(1, Ordering::AcqRel) + 1;
    *latest = Some(crate::screenshot::CapturedScreenshot {
        png_base64,
        width,
        height,
        generation,
    });
    Ok(generation)
}

/// 只返回 URL 所有的截图代次，旧页面不能读取后来导航页面的数据。
fn read_capture_if_generation(
    latest_capture: &Mutex<Option<crate::screenshot::CapturedScreenshot>>,
    generation: u64,
) -> Result<crate::screenshot::CapturedScreenshot, String> {
    latest_capture
        .lock()
        .map_err(|error| error.to_string())?
        .as_ref()
        .filter(|capture| capture.generation == generation)
        .cloned()
        .ok_or_else(|| "没有待编辑截图".to_string())
}

/// 清理未消费的截图缓存。
#[tauri::command]
pub fn clear_pending_capture(state: State<AppState>, generation: u64) -> Result<(), String> {
    let _transition = state
        .capture_editor_transition
        .lock()
        .map_err(|error| error.to_string())?;
    clear_capture_if_generation(&state.latest_capture, generation);
    let _ = state.capture_window_generation.compare_exchange(
        generation,
        0,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
    Ok(())
}

fn clear_capture_if_generation(
    latest_capture: &Mutex<Option<crate::screenshot::CapturedScreenshot>>,
    generation: u64,
) {
    if let Ok(mut latest) = latest_capture.lock() {
        if latest
            .as_ref()
            .is_some_and(|capture| capture.generation == generation)
        {
            *latest = None;
        }
    }
}

/// 将前端生成的 PNG 写入系统剪贴板。
#[tauri::command]
pub async fn copy_screenshot_image(png_base64: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let png = crate::screenshot::decode_png_base64(&png_base64).map_err(|e| e.to_string())?;
        crate::image_io::copy_png_to_clipboard(&png)
    })
    .await
    .map_err(|e| format!("截图复制线程异常: {e}"))?
}

/// 将前端生成的 PNG 保存到配置的截图目录。
#[tauri::command]
pub fn save_screenshot_image(png_base64: String, state: State<AppState>) -> Result<String, String> {
    let png = crate::screenshot::decode_png_base64(&png_base64).map_err(|e| e.to_string())?;
    let path = crate::image_io::save_png(&png, "clippy-screenshot", &state.save_target())?;
    Ok(path.to_string_lossy().to_string())
}

/// 另存为：弹系统对话框让用户选目录与文件名，用户取消时返回 None。
#[tauri::command]
pub async fn save_screenshot_image_as(
    png_base64: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let png = crate::screenshot::decode_png_base64(&png_base64).map_err(|e| e.to_string())?;
    let target = state.save_target();
    // 对话框阻塞到用户操作完，必须离开 IPC 的 async 线程。
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = crate::dialogs::choose_png_save_path(&app_handle, &target) else {
            return Ok(None);
        };
        crate::image_io::save_png_as(&png, &path).map(|saved| Some(saved.to_string_lossy().into()))
    })
    .await
    .map_err(|error| format!("另存为线程异常: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::{
        capture_window_url, clear_capture_if_generation, editor_window_size,
        read_capture_if_generation, release_capture_generation, EDITOR_MIN_HEIGHT,
        EDITOR_MIN_WIDTH,
    };
    use crate::screenshot::CapturedScreenshot;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    #[test]
    fn default_capability_includes_capture_window() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("capabilities")
            .join("default.json");
        let json = std::fs::read_to_string(path).expect("default capability should be readable");
        let value: serde_json::Value =
            serde_json::from_str(&json).expect("default capability should be valid JSON");
        let windows = value
            .get("windows")
            .and_then(serde_json::Value::as_array)
            .expect("default capability should list windows");

        assert!(windows.iter().any(|item| item == "capture"));
    }

    #[test]
    fn editor_window_wraps_the_capture_at_one_to_one() {
        // 1200×700 的截图在 1x 屏上：加上外壳正好放得下，不该被缩
        assert_eq!(
            editor_window_size((1200, 700), 1.0, (1920.0, 1080.0)),
            (1488.0, 852.0)
        );
        // HiDPI：物理像素先折算成逻辑尺寸，否则窗口会大一倍
        assert_eq!(
            editor_window_size((2400, 1400), 2.0, (1920.0, 1080.0)),
            (1488.0, 852.0)
        );
    }

    #[test]
    fn editor_window_respects_work_area_and_minimum() {
        // 全屏截图：夹到工作区内并留边
        assert_eq!(
            editor_window_size((1920, 1080), 1.0, (1920.0, 1080.0)),
            (1872.0, 1032.0)
        );
        // 小选区：不能小到侧栏放不下
        assert_eq!(
            editor_window_size((120, 40), 1.0, (1920.0, 1080.0)),
            (EDITOR_MIN_WIDTH, EDITOR_MIN_HEIGHT)
        );
        // 工作区比最小尺寸还小时以工作区为准，窗口不越界
        assert_eq!(
            editor_window_size((120, 40), 1.0, (640.0, 480.0)),
            (592.0, 432.0)
        );
    }

    #[test]
    fn editor_window_survives_a_bogus_scale_factor() {
        assert_eq!(
            editor_window_size((1200, 700), 0.0, (1920.0, 1080.0)),
            (1488.0, 852.0)
        );
        assert_eq!(
            editor_window_size((1200, 700), f64::NAN, (1920.0, 1080.0)),
            (1488.0, 852.0)
        );
    }

    #[test]
    fn capture_navigation_preserves_origin_and_replaces_page_state() {
        let url = tauri::Url::parse("http://localhost:1420/old.html?stale=1#fragment").unwrap();

        assert_eq!(
            capture_window_url(url, Some(7)).as_str(),
            "http://localhost:1420/capture.html?generation=7"
        );

        let active = tauri::Url::parse("tauri://localhost/capture.html?generation=7").unwrap();
        assert_eq!(
            capture_window_url(active, None).as_str(),
            "tauri://localhost/capture.html"
        );
    }

    #[test]
    fn reading_pending_capture_does_not_consume_the_latest_image() {
        let latest = Mutex::new(Some(CapturedScreenshot {
            png_base64: "cG5n".to_string(),
            width: 8,
            height: 6,
            generation: 1,
        }));

        let first = read_capture_if_generation(&latest, 1).expect("first read should succeed");
        let second =
            read_capture_if_generation(&latest, 1).expect("overlapping read should succeed");

        assert_eq!(first.png_base64, second.png_base64);
        assert_eq!((first.width, first.height), (8, 6));
        assert_eq!((second.width, second.height), (8, 6));
    }

    #[test]
    fn stale_capture_cleanup_does_not_remove_a_newer_generation() {
        let latest = Mutex::new(Some(CapturedScreenshot {
            png_base64: "bmV3".to_string(),
            width: 10,
            height: 10,
            generation: 2,
        }));

        clear_capture_if_generation(&latest, 1);
        assert!(read_capture_if_generation(&latest, 1).is_err());
        assert_eq!(
            read_capture_if_generation(&latest, 2).unwrap().generation,
            2
        );

        clear_capture_if_generation(&latest, 2);
        assert!(read_capture_if_generation(&latest, 2).is_err());
    }

    #[test]
    fn closing_editor_releases_only_the_registered_generation() {
        let registered = AtomicU64::new(1);
        let latest = Mutex::new(Some(CapturedScreenshot {
            png_base64: "bmV3".to_string(),
            width: 10,
            height: 10,
            generation: 2,
        }));

        release_capture_generation(&registered, &latest);

        assert_eq!(registered.load(Ordering::Acquire), 0);
        assert_eq!(
            read_capture_if_generation(&latest, 2).unwrap().generation,
            2
        );

        registered.store(2, Ordering::Release);
        release_capture_generation(&registered, &latest);
        assert!(read_capture_if_generation(&latest, 2).is_err());
    }
}
