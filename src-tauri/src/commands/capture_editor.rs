use super::AppState;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use tauri::{Manager, State};

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
    open_capture_window(app_handle, state, generation)
}

fn open_capture_window(
    app_handle: &tauri::AppHandle,
    state: &AppState,
    generation: u64,
) -> Result<(), String> {
    let result = if let Some(window) = app_handle.get_webview_window("capture") {
        window
            .url()
            .map(|url| capture_window_url(url, Some(generation)))
            .map_err(|error| error.to_string())
            .and_then(|url| window.navigate(url).map_err(|error| error.to_string()))
            .and_then(|_| window.show().map_err(|error| error.to_string()))
            .map(|_| {
                let _ = window.set_focus();
            })
    } else {
        tauri::WebviewWindowBuilder::new(
            app_handle,
            "capture",
            tauri::WebviewUrl::App(format!("capture.html?generation={generation}").into()),
        )
        .title("Clippy Screenshot")
        .inner_size(1180.0, 760.0)
        .min_inner_size(820.0, 560.0)
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

/// 将前端生成的 PNG 保存到 Pictures/Clippy。
#[tauri::command]
pub fn save_screenshot_image(png_base64: String) -> Result<String, String> {
    let png = crate::screenshot::decode_png_base64(&png_base64).map_err(|e| e.to_string())?;
    let path = crate::image_io::save_png(&png, "clippy-screenshot")?;
    Ok(path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        capture_window_url, clear_capture_if_generation, read_capture_if_generation,
        release_capture_generation,
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
