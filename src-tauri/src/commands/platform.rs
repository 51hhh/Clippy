/// 返回后端实际检测到的平台、桌面会话与功能能力。
#[tauri::command]
pub fn get_platform_info() -> crate::platform::PlatformInfo {
    crate::platform::current_info()
}
