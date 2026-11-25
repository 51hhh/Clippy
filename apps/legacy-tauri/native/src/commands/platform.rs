/// 返回后端实际检测到的平台、桌面会话与功能能力。
#[tauri::command]
pub async fn get_platform_info() -> Result<crate::platform::PlatformInfo, String> {
    tauri::async_runtime::spawn_blocking(crate::platform::current_info)
        .await
        .map_err(|error| format!("平台能力探测线程异常: {error}"))
}
