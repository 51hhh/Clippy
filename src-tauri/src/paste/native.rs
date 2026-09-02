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
    use std::time::{Duration, Instant};

    type Hwnd = *mut c_void;
    pub type Target = usize;

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
        Ok(window as usize)
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

    pub fn paste(target: Option<Target>) -> Result<(), PasteError> {
        let window = target.ok_or(PasteError::NativeTargetMissing)? as Hwnd;
        if unsafe { IsWindow(window) } == 0 {
            return Err(PasteError::NativeTargetInvalid);
        }
        if unsafe { SetForegroundWindow(window) } == 0 {
            return Err(PasteError::NativeFocusNotRestored(
                "SetForegroundWindow 被系统拒绝（目标可能具有更高完整性级别）".to_string(),
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
}

#[cfg(target_os = "macos")]
mod implementation {
    use super::*;
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};
    use std::ffi::{c_long, c_void};
    use std::time::{Duration, Instant};

    pub type Target = i32;
    type CfTypeRef = *const c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> u8;
        fn AXIsProcessTrustedWithOptions(options: CfTypeRef) -> u8;
        static kAXTrustedCheckOptionPrompt: CfTypeRef;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFBooleanTrue: CfTypeRef;
        fn CFDictionaryCreate(
            allocator: CfTypeRef,
            keys: *const CfTypeRef,
            values: *const CfTypeRef,
            count: c_long,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> CfTypeRef;
        fn CFRelease(value: CfTypeRef);
    }

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
        unsafe { AXIsProcessTrusted() != 0 }
    }

    pub fn can_request_permission() -> bool {
        true
    }

    pub fn permission_detail() -> &'static str {
        "macOS Accessibility permission is required to paste automatically"
    }

    pub fn request_permission() {
        let keys = [unsafe { kAXTrustedCheckOptionPrompt }];
        let values = [unsafe { kCFBooleanTrue }];
        let options = unsafe {
            CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if !options.is_null() {
            unsafe {
                AXIsProcessTrustedWithOptions(options);
                CFRelease(options);
            }
        }
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
