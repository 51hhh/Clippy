use super::AppState;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use tauri::{Emitter, Manager, State};

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

pub(crate) fn open_capture_window(app_handle: &tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("capture") {
        window.show().map_err(|error| error.to_string())?;
        let _ = window.set_focus();
        let _ = window.emit("capture-loaded", ());
        return Ok(());
    }

    let window = tauri::WebviewWindowBuilder::new(
        app_handle,
        "capture",
        tauri::WebviewUrl::App("capture.html".into()),
    )
    .title("Clippy Screenshot")
    .inner_size(1180.0, 760.0)
    .min_inner_size(820.0, 560.0)
    .center()
    .resizable(true)
    .build()
    .map_err(|error| error.to_string())?;
    let _ = window.set_focus();
    let _ = window.emit("capture-loaded", ());
    Ok(())
}

/// 返回最近一次截图编辑器待处理的截图。
#[tauri::command]
pub fn get_pending_capture(
    state: State<AppState>,
) -> Result<crate::screenshot::CapturedScreenshot, String> {
    read_latest_capture(&state.latest_capture)
}

/// 写入最新截图并分配单调代次，用于避免旧编辑窗口清理新截图。
pub(crate) fn store_latest_capture(
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

/// 读取截图而不消费它，允许初始化请求和 capture-loaded 事件请求安全重叠。
/// 前端卸载时携带读取到的 generation 清理，避免旧窗口误删后来写入的截图。
pub(crate) fn read_latest_capture(
    latest_capture: &Mutex<Option<crate::screenshot::CapturedScreenshot>>,
) -> Result<crate::screenshot::CapturedScreenshot, String> {
    latest_capture
        .lock()
        .map_err(|error| error.to_string())?
        .clone()
        .ok_or_else(|| "没有待编辑截图".to_string())
}

/// 清理未消费的截图缓存。
#[tauri::command]
pub fn clear_pending_capture(
    state: State<AppState>,
    generation: Option<u64>,
) -> Result<(), String> {
    if let Some(generation) = generation {
        clear_capture_if_generation(&state.latest_capture, generation);
    }
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
    use super::{clear_capture_if_generation, read_latest_capture};
    use crate::screenshot::CapturedScreenshot;
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
    fn reading_pending_capture_does_not_consume_the_latest_image() {
        let latest = Mutex::new(Some(CapturedScreenshot {
            png_base64: "cG5n".to_string(),
            width: 8,
            height: 6,
            generation: 1,
        }));

        let first = read_latest_capture(&latest).expect("first read should succeed");
        let second = read_latest_capture(&latest).expect("overlapping read should also succeed");

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
        assert_eq!(read_latest_capture(&latest).unwrap().generation, 2);

        clear_capture_if_generation(&latest, 2);
        assert!(read_latest_capture(&latest).is_err());
    }
}
