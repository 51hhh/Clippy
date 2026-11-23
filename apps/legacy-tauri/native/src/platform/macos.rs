//! macOS TCC 权限探测与请求。
//!
//! 查询函数不会弹窗，可安全用于能力 IPC；请求函数只应由用户明确触发的粘贴授权或截图
//! 动作调用。辅助功能提示是异步的，调用后仍以再次预检的结果为准。

use std::{
    ffi::{c_long, c_void},
    sync::atomic::{AtomicBool, Ordering},
};

type CfTypeRef = *const c_void;

/// CoreGraphics 只公开“当前是否已授权”的布尔预检，无法区分未决定、拒绝与撤销。
///
/// 同一进程里第一次明确截图动作可以触发系统请求；此后持续用 preflight 观察用户是否已在
/// 系统设置中授权，但不反复调用请求 API。进程重启后允许再请求一次，兼顾用户重置 TCC 后恢复。
static SCREEN_CAPTURE_REQUESTED: AtomicBool = AtomicBool::new(false);

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

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

pub(crate) fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

pub(crate) fn request_accessibility_permission() {
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

pub(crate) fn screen_capture_trusted() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}

pub(crate) fn request_screen_capture_permission() -> bool {
    if screen_capture_trusted() {
        return true;
    }
    if SCREEN_CAPTURE_REQUESTED.swap(true, Ordering::AcqRel) {
        return false;
    }
    unsafe { CGRequestScreenCaptureAccess() }
}

/// 让贴图同时出现在全部 Spaces，并能伴随其它应用的原生全屏窗口显示。
///
/// `WebviewWindow::set_visible_on_all_workspaces` 只设置 `CanJoinAllSpaces`；AppKit 把
/// “进入全屏 Space”单独建模为 `FullScreenAuxiliary`，所以这里必须在原生 NSWindow 上
/// 合并两项而不是覆盖 Tauri 已经设置的其它行为。
pub(crate) unsafe fn configure_pin_collection_behavior(
    raw_window: *mut c_void,
) -> Result<(), &'static str> {
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};

    if raw_window.is_null() {
        return Err("NSWindow 指针为空");
    }
    let window: &NSWindow = unsafe { &*raw_window.cast() };
    let behavior = window.collectionBehavior()
        | NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::FullScreenAuxiliary;
    window.setCollectionBehavior(behavior);
    Ok(())
}
