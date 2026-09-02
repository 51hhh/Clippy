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

#[cfg(test)]
mod tests {
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
}
