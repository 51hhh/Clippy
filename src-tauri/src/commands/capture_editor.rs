use super::AppState;
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

/// 读取截图而不消费它，允许初始化请求和 capture-loaded 事件请求安全重叠。
/// 生命周期结束时由 clear_pending_capture 或窗口销毁事件统一清理。
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
pub fn clear_pending_capture(state: State<AppState>) -> Result<(), String> {
    clear_latest_capture(&state);
    Ok(())
}

pub fn clear_latest_capture(state: &AppState) {
    if let Ok(mut latest) = state.latest_capture.lock() {
        *latest = None;
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
    use super::read_latest_capture;
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
        }));

        let first = read_latest_capture(&latest).expect("first read should succeed");
        let second = read_latest_capture(&latest).expect("overlapping read should also succeed");

        assert_eq!(first.png_base64, second.png_base64);
        assert_eq!((first.width, first.height), (8, 6));
        assert_eq!((second.width, second.height), (8, 6));
    }
}
