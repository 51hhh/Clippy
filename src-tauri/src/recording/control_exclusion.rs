//! 录屏控制窗的平台排除能力与原生配置。
//!
//! 能力结论同时驱动几何规划和原生 API 调用。几何后备平台不得尝试原生排除，否则旧版 Windows
//! 会把 `WDA_EXCLUDEFROMCAPTURE` 退化为黑块，甚至直接让录屏启动失败。

use super::control_window::WindowExclusionCapability;

pub(super) fn control_exclusion_capability() -> WindowExclusionCapability {
    #[cfg(target_os = "windows")]
    {
        let version = windows_version::OsVersion::current();
        windows_control_exclusion_capability(version.major, version.build)
    }
    #[cfg(not(target_os = "windows"))]
    {
        WindowExclusionCapability::GeometryOnly
    }
}

pub(super) fn configure_control_exclusion(
    window: &tauri::WebviewWindow,
    capability: WindowExclusionCapability,
) -> Result<(), String> {
    if requires_native_control_exclusion(capability) {
        exclude_control_from_capture(window)
    } else {
        Ok(())
    }
}

fn requires_native_control_exclusion(capability: WindowExclusionCapability) -> bool {
    matches!(capability, WindowExclusionCapability::Native)
}

#[cfg(any(target_os = "windows", test))]
fn windows_control_exclusion_capability(major: u32, build: u32) -> WindowExclusionCapability {
    if supports_windows_native_exclusion(major, build) {
        WindowExclusionCapability::Native
    } else {
        // Windows 10 2004 前会把 0x11 退化为 WDA_MONITOR；继续要求几何排除，避免留下黑块。
        WindowExclusionCapability::GeometryOnly
    }
}

#[cfg(any(target_os = "windows", test))]
fn supports_windows_native_exclusion(major: u32, build: u32) -> bool {
    major > 10 || (major == 10 && build >= 19_041)
}

#[cfg(target_os = "windows")]
fn exclude_control_from_capture(window: &tauri::WebviewWindow) -> Result<(), String> {
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
    };

    let hwnd = window.hwnd().map_err(|error| error.to_string())?;
    let succeeded = unsafe { SetWindowDisplayAffinity(hwnd.0, WDA_EXCLUDEFROMCAPTURE) };
    if succeeded == 0 {
        let error = unsafe { GetLastError() };
        return Err(format!(
            "Windows 无法把录屏控制窗排除出捕获，错误码 {error}"
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn exclude_control_from_capture(_window: &tauri::WebviewWindow) -> Result<(), String> {
    Err("当前平台不支持原生录屏窗口排除".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_window_exclusion_requires_windows_10_2004() {
        assert!(!supports_windows_native_exclusion(6, 9_600));
        assert!(!supports_windows_native_exclusion(10, 18_363));
        assert!(supports_windows_native_exclusion(10, 19_041));
        assert!(supports_windows_native_exclusion(10, 22_000));
        assert!(supports_windows_native_exclusion(11, 1));
        assert_eq!(
            windows_control_exclusion_capability(10, 19_040),
            WindowExclusionCapability::GeometryOnly
        );
        assert_eq!(
            windows_control_exclusion_capability(10, 19_041),
            WindowExclusionCapability::Native
        );
        assert!(!requires_native_control_exclusion(
            windows_control_exclusion_capability(10, 19_040)
        ));
        assert!(requires_native_control_exclusion(
            windows_control_exclusion_capability(10, 19_041)
        ));
    }
}
