//! 截图的入口：托盘与全局快捷键都从这里拉起冻结画面覆盖层。
//!
//! 早先这里还有一个独立的"截图编辑器窗口"（`capture` label + 待编辑截图代次缓存），
//! 以及给那个窗口用的"复制 / 保存 / 另存为 PNG"三个命令。现在标注直接发生在
//! 覆盖层里，`commit_capture_action` 只收选区与 v2 操作层，再用后端冻结帧合成。
//! 旧窗口、代次缓存和那三个只接 base64 就写文件/剪贴板的命令全部删掉了。

use super::AppState;

/// 拿到 `AppState` 后走和 IPC 命令 `show_capture_overlay` 同一条路径。
pub(crate) async fn trigger_capture_overlay(app_handle: tauri::AppHandle) -> Result<(), String> {
    let state = tauri::Manager::state::<AppState>(&app_handle);
    crate::capture::show_capture_overlay_for_app(app_handle.clone(), &state).await
}

#[cfg(any(test, target_os = "linux"))]
fn claim_capture_request(
    gate: &std::sync::Arc<crate::capture::CaptureModeGate>,
) -> Result<crate::capture::CaptureModeOwnership, crate::capture::CaptureError> {
    gate.try_claim_owned(crate::capture::CaptureMode::Ordinary)
}

/// GNOME gsettings 的截图请求在 D-Bus 应答前先认领 gate。
///
/// `true` 只表示请求已经取得唯一消费权并排入后台；后续截图、窗口与会话错误仍按原路径
/// 记录和补偿。`false` 表示已有截图或录屏会话占用该模式。
#[cfg(target_os = "linux")]
pub(crate) fn queue_capture_overlay(app_handle: tauri::AppHandle) -> bool {
    let ownership = {
        let state = tauri::Manager::state::<AppState>(&app_handle);
        match claim_capture_request(&state.capture_mode_gate) {
            Ok(ownership) => ownership,
            Err(error) => {
                log::info!("截图快捷键请求未接收: {error}");
                return false;
            }
        }
    };

    tauri::async_runtime::spawn(async move {
        let state = tauri::Manager::state::<AppState>(&app_handle);
        if let Err(error) = crate::capture::start_claimed_capture_overlay_for_app(
            app_handle.clone(),
            &state,
            ownership,
        )
        .await
        {
            log::warn!("已接收的截图快捷键请求启动失败: {error}");
        }
    });
    true
}

/// 录屏从独立入口进入同一套冻结选区覆盖层；后端把会话用途固定为 Recording。
pub(crate) async fn trigger_recording_overlay(app_handle: tauri::AppHandle) -> Result<(), String> {
    if !crate::recording::product_entry_available() {
        return Err("当前构建或桌面会话尚未开放录屏入口".to_string());
    }
    let state = tauri::Manager::state::<AppState>(&app_handle);
    crate::capture::show_recording_overlay_for_app(app_handle.clone(), &state).await
}

/// 类型化动作复用同一个截图入口，并保留结构化领域错误供动作层映射稳定错误码。
pub(crate) async fn start_capture_overlay(
    app_handle: tauri::AppHandle,
) -> Result<String, crate::capture::CaptureError> {
    let state = tauri::Manager::state::<AppState>(&app_handle);
    crate::capture::start_capture_overlay_for_app(app_handle.clone(), &state).await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::capture::{CaptureError, CaptureMode, CaptureModeGate};

    use super::claim_capture_request;

    /// 编辑器窗口已经删掉，能力清单里不能再留 `capture`——留着就是一个
    /// 谁都进不去、却仍然被授权的窗口面。
    #[test]
    fn default_capability_no_longer_lists_the_removed_editor_window() {
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

        assert!(!windows.iter().any(|item| item == "capture"));
        assert!(windows.iter().any(|item| item == "main"));
    }

    #[test]
    fn capture_request_ack_is_rejected_before_background_work_when_gate_is_busy() {
        let gate = Arc::new(CaptureModeGate::new());
        let first = claim_capture_request(&gate).expect("首个请求应取得 gate");

        assert!(matches!(
            claim_capture_request(&gate),
            Err(CaptureError::CaptureModeBusy)
        ));
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Ordinary));

        first.release().expect("测试结束应释放 gate");
        assert_eq!(gate.active_mode().unwrap(), None);
    }
}
