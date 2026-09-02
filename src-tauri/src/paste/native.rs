//! Windows / macOS 原生自动粘贴。
//!
//! 面板出现前先记录前台目标；选中条目、面板隐藏后恢复该目标，再注入系统对应的粘贴
//! 组合键。目标失效、焦点恢复失败或权限不足都返回结构化错误，由 command 层安全降级为
//! “内容已复制到剪贴板”。

use super::{PasteBackend, PasteError};
use enigo::{
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Settings,
};

fn inject_paste(modifier: Key, modifier_name: &str) -> Result<(), PasteError> {
    let injection = |action: &str| {
        let action = action.to_string();
        move |error: enigo::InputError| PasteError::KeyInjection {
            action,
            detail: error.to_string(),
        }
    };
    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|error| PasteError::InputBackendUnavailable(error.to_string()))?;
    enigo
        .key(modifier, Press)
        .map_err(injection(&format!("按下 {modifier_name}")))?;
    let click = enigo.key(Key::Unicode('v'), Click);
    let release = enigo.key(modifier, Release);
    click.map_err(injection("按下 V"))?;
    release.map_err(injection(&format!("释放 {modifier_name}")))?;
    Ok(())
}

#[cfg(target_os = "windows")]
mod implementation {
    use super::*;
    use std::ffi::c_void;
    use std::io;
    use std::ptr::null_mut;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, IsValidSid,
        TokenIntegrityLevel, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    type Hwnd = *mut c_void;

    #[derive(Clone)]
    pub struct Target {
        window: usize,
        process_id: u32,
    }

    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: 句柄由 OpenProcess/OpenProcessToken 返回，只在这里关闭一次。
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetForegroundWindow() -> Hwnd;
        fn GetWindowThreadProcessId(window: Hwnd, process_id: *mut u32) -> u32;
        fn IsWindow(window: Hwnd) -> i32;
        fn SetForegroundWindow(window: Hwnd) -> i32;
    }

    pub fn backend() -> PasteBackend {
        PasteBackend::WindowsSendInput
    }

    pub fn capture_target() -> Result<Target, PasteError> {
        let window = unsafe { GetForegroundWindow() };
        if window.is_null() {
            return Err(PasteError::NativeTargetMissing);
        }
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(window, &mut process_id) };
        if process_id == 0 || process_id == std::process::id() {
            return Err(PasteError::NativeTargetMissing);
        }
        Ok(Target {
            window: window as usize,
            process_id,
        })
    }

    pub fn permission_ready() -> bool {
        true
    }

    pub fn can_request_permission() -> bool {
        false
    }

    pub fn permission_detail() -> &'static str {
        "Windows input injection is unavailable"
    }

    pub fn request_permission() {}

    /// 读取进程访问令牌的 Mandatory Integrity Level RID。
    ///
    /// Windows 的 UIPI 只允许向相同或更低完整性级别注入输入。SendInput 本身不会明确报告
    /// UIPI 拒绝，所以必须在调用 enigo 之前完成这项检查，查询失败也按纯复制安全降级。
    fn process_integrity_rid(process: HANDLE) -> Result<u32, PasteError> {
        let mut raw_token = null_mut();
        // SAFETY: process 是当前进程伪句柄或 OwnedHandle 管理的有效进程句柄；输出写入栈变量。
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw_token) } == 0 {
            return Err(PasteError::WindowsIntegrityQuery(
                io::Error::last_os_error().to_string(),
            ));
        }
        let token = OwnedHandle(raw_token);

        let mut required = 0u32;
        // 第一次调用按 Win32 合同只查询所需长度，失败且返回非零长度是正常路径。
        unsafe {
            GetTokenInformation(token.0, TokenIntegrityLevel, null_mut(), 0, &mut required);
        }
        if required < std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 {
            return Err(PasteError::WindowsIntegrityQuery(
                "TokenIntegrityLevel 未返回有效缓冲区长度".to_string(),
            ));
        }

        // usize 缓冲区确保 TOKEN_MANDATORY_LABEL 具备正确对齐。
        let word_size = std::mem::size_of::<usize>();
        let mut words = vec![0usize; (required as usize).div_ceil(word_size)];
        // SAFETY: 缓冲区至少为 required 字节且正确对齐，token 在调用期间有效。
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenIntegrityLevel,
                words.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        } == 0
        {
            return Err(PasteError::WindowsIntegrityQuery(
                io::Error::last_os_error().to_string(),
            ));
        }

        // SAFETY: 成功的 TokenIntegrityLevel 查询保证缓冲区以 TOKEN_MANDATORY_LABEL 开头。
        let sid = unsafe {
            (*(words.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()))
                .Label
                .Sid
        };
        if sid.is_null() || unsafe { IsValidSid(sid) } == 0 {
            return Err(PasteError::WindowsIntegrityQuery(
                "TokenIntegrityLevel 返回了无效 SID".to_string(),
            ));
        }

        // 完整性 RID 是 Mandatory Label SID 的最后一个 sub-authority。
        let count = unsafe { GetSidSubAuthorityCount(sid) };
        if count.is_null() || unsafe { *count } == 0 {
            return Err(PasteError::WindowsIntegrityQuery(
                "Mandatory Label SID 缺少 sub-authority".to_string(),
            ));
        }
        let index = u32::from(unsafe { *count } - 1);
        let rid = unsafe { GetSidSubAuthority(sid, index) };
        if rid.is_null() {
            return Err(PasteError::WindowsIntegrityQuery(
                "无法读取 Mandatory Label RID".to_string(),
            ));
        }
        Ok(unsafe { *rid })
    }

    fn ensure_target_integrity(current_rid: u32, target_rid: u32) -> Result<(), PasteError> {
        if target_rid > current_rid {
            Err(PasteError::WindowsIntegrityBoundary {
                current_rid,
                target_rid,
            })
        } else {
            Ok(())
        }
    }

    pub fn paste(target: Option<Target>) -> Result<(), PasteError> {
        let target = target.ok_or(PasteError::NativeTargetMissing)?;
        let window = target.window as Hwnd;
        if unsafe { IsWindow(window) } == 0 {
            return Err(PasteError::NativeTargetInvalid);
        }
        let mut current_process_id = 0;
        unsafe { GetWindowThreadProcessId(window, &mut current_process_id) };
        if current_process_id == 0 || current_process_id != target.process_id {
            // HWND 可能在捕获后被销毁并复用，不能把按键发给新的窗口所有者。
            return Err(PasteError::NativeTargetInvalid);
        }

        let process =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, target.process_id) };
        if process.is_null() {
            return Err(PasteError::WindowsIntegrityQuery(format!(
                "无法打开目标进程: {}",
                io::Error::last_os_error()
            )));
        }
        let process = OwnedHandle(process);
        let current_rid = process_integrity_rid(unsafe { GetCurrentProcess() })?;
        let target_rid = process_integrity_rid(process.0)?;
        ensure_target_integrity(current_rid, target_rid)?;

        if unsafe { SetForegroundWindow(window) } == 0 {
            return Err(PasteError::NativeFocusNotRestored(
                "SetForegroundWindow 被系统拒绝".to_string(),
            ));
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if unsafe { GetForegroundWindow() } == window {
                return inject_paste(Key::Control, "Control");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(PasteError::NativeFocusNotRestored(
            "前台窗口在 500ms 内未切回目标".to_string(),
        ))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn rejects_only_higher_integrity_targets() {
            assert!(ensure_target_integrity(0x2000, 0x1000).is_ok());
            assert!(ensure_target_integrity(0x2000, 0x2000).is_ok());
            let error = ensure_target_integrity(0x2000, 0x3000).unwrap_err();
            assert_eq!(error.code(), "windows_integrity_boundary");
        }
    }
}

#[cfg(target_os = "macos")]
mod implementation {
    use super::*;
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};
    use std::time::{Duration, Instant};

    pub type Target = i32;

    pub fn backend() -> PasteBackend {
        PasteBackend::MacosQuartz
    }

    pub fn capture_target() -> Result<Target, PasteError> {
        let application = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .ok_or(PasteError::NativeTargetMissing)?;
        let process_id = application.processIdentifier();
        if process_id <= 0 || process_id as u32 == std::process::id() {
            return Err(PasteError::NativeTargetMissing);
        }
        Ok(process_id)
    }

    pub fn permission_ready() -> bool {
        crate::platform::macos_accessibility_trusted()
    }

    pub fn can_request_permission() -> bool {
        true
    }

    pub fn permission_detail() -> &'static str {
        "macOS Accessibility permission is required to paste automatically"
    }

    pub fn request_permission() {
        crate::platform::request_macos_accessibility_permission();
    }

    pub fn paste(target: Option<Target>) -> Result<(), PasteError> {
        if !permission_ready() {
            return Err(PasteError::MacosAccessibilityPermissionRequired);
        }
        let process_id = target.ok_or(PasteError::NativeTargetMissing)?;
        let application = NSRunningApplication::runningApplicationWithProcessIdentifier(process_id)
            .ok_or(PasteError::NativeTargetInvalid)?;
        if !application.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows) {
            return Err(PasteError::NativeFocusNotRestored(
                "NSRunningApplication 拒绝激活目标应用".to_string(),
            ));
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if application.isActive() {
                return inject_paste(Key::Meta, "Command");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(PasteError::NativeFocusNotRestored(
            "前台应用在 500ms 内未切回目标".to_string(),
        ))
    }
}

pub(super) use implementation::*;
