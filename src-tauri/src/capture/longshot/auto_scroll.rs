//! 自动长截图的平台输入边界。
//!
//! X11、Windows 与 macOS 都沿用同一逐步事务：移动到后端冻结的选区中心、锁定该点下的
//! 原生窗口、注入一格滚动、重新捕获，并在提交前后复核指针和目标。Wayland 必须建立用户
//! 授权的 RemoteDesktop/libei 会话，不能借 XWayland 假装控制原生窗口。

use super::CaptureError;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
const SCROLL_TICKS: i32 = 5;
#[cfg(any(test, target_os = "linux", target_os = "windows", target_os = "macos"))]
const POINTER_TOLERANCE: i32 = 3;
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
const POINTER_SETTLE_MS: u64 = 45;
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
const CONTENT_SETTLE_MS: u64 = 360;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LongshotAutoDirection {
    Down,
    Up,
    Right,
    Left,
}

impl LongshotAutoDirection {
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    fn input(self) -> (enigo::Axis, i32) {
        match self {
            Self::Down => (enigo::Axis::Vertical, SCROLL_TICKS),
            Self::Up => (enigo::Axis::Vertical, -SCROLL_TICKS),
            Self::Right => (enigo::Axis::Horizontal, SCROLL_TICKS),
            Self::Left => (enigo::Axis::Horizontal, -SCROLL_TICKS),
        }
    }
}

/// 原生窗口句柄必须与进程一起比较，避免句柄销毁后被另一进程复用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowIdentity {
    native_id: u64,
    process_id: u32,
}

#[derive(Debug, Clone)]
#[cfg_attr(
    not(any(target_os = "linux", target_os = "windows", target_os = "macos")),
    allow(dead_code)
)]
pub(in crate::capture) struct LongshotAutoTarget {
    point: (i32, i32),
    /// 首次真实输入前锁定，后续每一步逐次复核。
    window: Arc<Mutex<Option<WindowIdentity>>>,
}

impl LongshotAutoTarget {
    pub(super) fn new(point: (i32, i32)) -> Self {
        Self {
            point,
            window: Arc::new(Mutex::new(None)),
        }
    }
}

fn checked_identity(
    native_id: u64,
    process_id: u32,
    current_process_id: u32,
) -> Result<WindowIdentity, CaptureError> {
    if native_id == 0 || (process_id != 0 && process_id == current_process_id) {
        return Err(CaptureError::LongshotAutoTargetLost);
    }
    Ok(WindowIdentity {
        native_id,
        process_id,
    })
}

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
struct CursorRestore {
    original: (i32, i32),
    armed: bool,
}

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
impl CursorRestore {
    fn new(original: (i32, i32)) -> Self {
        Self {
            original,
            armed: true,
        }
    }

    /// 用户主动移动鼠标后不能再把光标抢回步骤开始位置。
    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
impl Drop for CursorRestore {
    fn drop(&mut self) {
        if self.armed {
            if let Err(error) = move_pointer(self.original) {
                log::warn!("恢复自动长截图鼠标位置失败: {error}");
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn x11_pointer_reply() -> Result<x11rb::protocol::xproto::QueryPointerReply, CaptureError> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;

    let (connection, screen) =
        x11rb::connect(None).map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?;
    let root = connection
        .setup()
        .roots
        .get(screen)
        .ok_or_else(|| CaptureError::LongshotAutoInput("X11 screen 不存在".to_string()))?
        .root;
    let reply = connection
        .query_pointer(root)
        .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?
        .reply()
        .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?;
    Ok(reply)
}

#[cfg(target_os = "linux")]
fn move_pointer(point: (i32, i32)) -> Result<(), CaptureError> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;

    let x = i16::try_from(point.0)
        .map_err(|_| CaptureError::LongshotAutoInput("X11 横坐标超出范围".to_string()))?;
    let y = i16::try_from(point.1)
        .map_err(|_| CaptureError::LongshotAutoInput("X11 纵坐标超出范围".to_string()))?;
    let (connection, screen) =
        x11rb::connect(None).map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?;
    let root = connection
        .setup()
        .roots
        .get(screen)
        .ok_or_else(|| CaptureError::LongshotAutoInput("X11 screen 不存在".to_string()))?
        .root;
    connection
        .warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, x, y)
        .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?
        .check()
        .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?;
    connection
        .flush()
        .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))
}

#[cfg(target_os = "linux")]
fn pointer_location() -> Result<(i32, i32), CaptureError> {
    let reply = x11_pointer_reply()?;
    Ok((i32::from(reply.root_x), i32::from(reply.root_y)))
}

#[cfg(target_os = "linux")]
fn pointer_window() -> Result<WindowIdentity, CaptureError> {
    let reply = x11_pointer_reply()?;
    checked_identity(u64::from(reply.child), 0, std::process::id())
}

#[cfg(target_os = "windows")]
fn pointer_location() -> Result<(i32, i32), CaptureError> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetPhysicalCursorPos;

    let mut point = POINT { x: 0, y: 0 };
    if unsafe { GetPhysicalCursorPos(&mut point) } == 0 {
        return Err(CaptureError::LongshotAutoInput(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    Ok((point.x, point.y))
}

#[cfg(target_os = "windows")]
fn move_pointer(point: (i32, i32)) -> Result<(), CaptureError> {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetPhysicalCursorPos;

    if unsafe { SetPhysicalCursorPos(point.0, point.1) } == 0 {
        return Err(CaptureError::LongshotAutoInput(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn pointer_window() -> Result<WindowIdentity, CaptureError> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetWindowThreadProcessId, IsWindow, WindowFromPhysicalPoint, GA_ROOT,
    };

    let point = pointer_location()?;
    let child = unsafe {
        WindowFromPhysicalPoint(POINT {
            x: point.0,
            y: point.1,
        })
    };
    if child.is_null() {
        return Err(CaptureError::LongshotAutoTargetLost);
    }
    let root = unsafe { GetAncestor(child, GA_ROOT) };
    let window = if root.is_null() { child } else { root };
    if unsafe { IsWindow(window) } == 0 {
        return Err(CaptureError::LongshotAutoTargetLost);
    }
    let mut process_id = 0u32;
    unsafe { GetWindowThreadProcessId(window, &mut process_id) };
    if process_id == 0 {
        return Err(CaptureError::LongshotAutoTargetLost);
    }
    checked_identity(window as usize as u64, process_id, std::process::id())
}

#[cfg(target_os = "macos")]
fn macos_point(value: objc2_core_foundation::CGPoint) -> Result<(i32, i32), CaptureError> {
    if !value.x.is_finite()
        || !value.y.is_finite()
        || value.x < f64::from(i32::MIN)
        || value.x > f64::from(i32::MAX)
        || value.y < f64::from(i32::MIN)
        || value.y > f64::from(i32::MAX)
    {
        return Err(CaptureError::LongshotAutoInput(
            "macOS 指针坐标无效".to_string(),
        ));
    }
    Ok((value.x.round() as i32, value.y.round() as i32))
}

#[cfg(target_os = "macos")]
fn pointer_location() -> Result<(i32, i32), CaptureError> {
    use objc2_core_graphics::CGEvent;

    let event = CGEvent::new(None).ok_or_else(|| {
        CaptureError::LongshotAutoInput("无法创建 macOS 指针查询事件".to_string())
    })?;
    macos_point(CGEvent::location(Some(event.as_ref())))
}

#[cfg(target_os = "macos")]
fn move_pointer(point: (i32, i32)) -> Result<(), CaptureError> {
    use objc2_core_foundation::CGPoint;
    use objc2_core_graphics::{CGError, CGWarpMouseCursorPosition};

    let result = CGWarpMouseCursorPosition(CGPoint {
        x: f64::from(point.0),
        y: f64::from(point.1),
    });
    if result != CGError::Success {
        return Err(CaptureError::LongshotAutoInput(format!(
            "CGWarpMouseCursorPosition 失败: {}",
            result.0
        )));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_dictionary_value(
    dictionary: &objc2_core_foundation::CFDictionary,
    key: &str,
) -> Option<*const std::ffi::c_void> {
    use objc2_core_foundation::CFString;

    let key = CFString::from_str(key);
    let value = unsafe { dictionary.value((key.as_ref() as *const CFString).cast()) };
    (!value.is_null()).then_some(value)
}

#[cfg(target_os = "macos")]
fn macos_number(dictionary: &objc2_core_foundation::CFDictionary, key: &str) -> Option<i64> {
    use objc2_core_foundation::{CFNumber, CFNumberType};

    let number = macos_dictionary_value(dictionary, key)?.cast::<CFNumber>();
    let mut value = 0i64;
    let valid = unsafe {
        (*number).value(
            CFNumberType::LongLongType,
            (&mut value as *mut i64).cast::<std::ffi::c_void>(),
        )
    };
    valid.then_some(value)
}

#[cfg(target_os = "macos")]
fn macos_bounds(
    dictionary: &objc2_core_foundation::CFDictionary,
) -> Option<objc2_core_foundation::CGRect> {
    use objc2_core_foundation::{CFDictionary, CGRect};
    use objc2_core_graphics::CGRectMakeWithDictionaryRepresentation;

    let bounds = macos_dictionary_value(dictionary, "kCGWindowBounds")?.cast::<CFDictionary>();
    let mut rect = CGRect::default();
    unsafe { CGRectMakeWithDictionaryRepresentation(Some(&*bounds), &mut rect) }.then_some(rect)
}

#[cfg(target_os = "macos")]
fn macos_rect_contains(rect: objc2_core_foundation::CGRect, point: (i32, i32)) -> bool {
    let right = rect.origin.x + rect.size.width;
    let bottom = rect.origin.y + rect.size.height;
    rect.origin.x.is_finite()
        && rect.origin.y.is_finite()
        && rect.size.width.is_finite()
        && rect.size.height.is_finite()
        && rect.size.width > 0.0
        && rect.size.height > 0.0
        && f64::from(point.0) >= rect.origin.x
        && f64::from(point.0) < right
        && f64::from(point.1) >= rect.origin.y
        && f64::from(point.1) < bottom
}

#[cfg(target_os = "macos")]
fn pointer_window() -> Result<WindowIdentity, CaptureError> {
    use objc2_core_foundation::CFDictionary;
    use objc2_core_graphics::{CGWindowListCopyWindowInfo, CGWindowListOption};

    let point = pointer_location()?;
    let windows = CGWindowListCopyWindowInfo(
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
        0,
    )
    .ok_or_else(|| CaptureError::LongshotAutoInput("无法读取 macOS 窗口列表".to_string()))?;
    for index in 0..windows.count() {
        let dictionary = windows.value_at_index(index).cast::<CFDictionary>();
        if dictionary.is_null() {
            continue;
        }
        // SAFETY: CGWindowListCopyWindowInfo 返回的数组元素在 windows 生命周期内是 CFDictionary。
        let dictionary = unsafe { &*dictionary };
        if macos_number(dictionary, "kCGWindowLayer") != Some(0) {
            continue;
        }
        let Some(bounds) = macos_bounds(dictionary) else {
            continue;
        };
        if !macos_rect_contains(bounds, point) {
            continue;
        }
        let Some(window_id) = macos_number(dictionary, "kCGWindowNumber") else {
            continue;
        };
        let Some(process_id) = macos_number(dictionary, "kCGWindowOwnerPID") else {
            continue;
        };
        if window_id <= 0
            || window_id > i64::from(u32::MAX)
            || process_id <= 0
            || process_id > i64::from(u32::MAX)
        {
            continue;
        }
        return checked_identity(window_id as u64, process_id as u32, std::process::id());
    }
    Err(CaptureError::LongshotAutoTargetLost)
}

#[cfg(any(test, target_os = "linux", target_os = "windows", target_os = "macos"))]
fn pointer_near(actual: (i32, i32), expected: (i32, i32)) -> bool {
    actual.0.abs_diff(expected.0) <= POINTER_TOLERANCE as u32
        && actual.1.abs_diff(expected.1) <= POINTER_TOLERANCE as u32
}

fn lock_target_window(
    window: &Mutex<Option<WindowIdentity>>,
    current: WindowIdentity,
) -> Result<(), CaptureError> {
    let mut expected = window.lock().map_err(CaptureError::state_lock)?;
    match *expected {
        Some(value) if value != current => Err(CaptureError::LongshotAutoTargetLost),
        None => {
            *expected = Some(current);
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
fn ensure_backend_available() -> Result<(), CaptureError> {
    #[cfg(target_os = "linux")]
    if crate::platform::current_session() != crate::platform::DesktopSession::X11 {
        return Err(CaptureError::LongshotAutoUnsupported);
    }
    #[cfg(target_os = "macos")]
    if !crate::platform::macos_accessibility_trusted() {
        return Err(CaptureError::LongshotAutoPermissionRequired);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn ensure_target_can_receive_input(_target: WindowIdentity) -> Result<(), CaptureError> {
    Ok(())
}

#[cfg(target_os = "windows")]
fn ensure_target_can_receive_input(target: WindowIdentity) -> Result<(), CaptureError> {
    use std::ffi::c_void;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetForegroundWindow, GetWindowThreadProcessId, IsWindow, SetForegroundWindow,
        GA_ROOT,
    };

    crate::platform::ensure_windows_input_target_integrity(target.process_id)
        .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?;
    let window = target.native_id as usize as *mut c_void;
    if unsafe { IsWindow(window) } == 0 {
        return Err(CaptureError::LongshotAutoTargetLost);
    }
    let mut process_id = 0u32;
    unsafe { GetWindowThreadProcessId(window, &mut process_id) };
    if process_id != target.process_id || unsafe { SetForegroundWindow(window) } == 0 {
        return Err(CaptureError::LongshotAutoTargetLost);
    }
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        let foreground = unsafe { GetForegroundWindow() };
        let root = if foreground.is_null() {
            foreground
        } else {
            let ancestor = unsafe { GetAncestor(foreground, GA_ROOT) };
            if ancestor.is_null() {
                foreground
            } else {
                ancestor
            }
        };
        if root == window {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(CaptureError::LongshotAutoTargetLost)
}

#[cfg(target_os = "macos")]
fn ensure_target_can_receive_input(target: WindowIdentity) -> Result<(), CaptureError> {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    use std::time::{Duration, Instant};

    if !crate::platform::macos_accessibility_trusted() {
        return Err(CaptureError::LongshotAutoPermissionRequired);
    }
    let process_id =
        i32::try_from(target.process_id).map_err(|_| CaptureError::LongshotAutoTargetLost)?;
    let application = NSRunningApplication::runningApplicationWithProcessIdentifier(process_id)
        .ok_or(CaptureError::LongshotAutoTargetLost)?;
    if !application.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows) {
        return Err(CaptureError::LongshotAutoTargetLost);
    }
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        if application.isActive() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(CaptureError::LongshotAutoTargetLost)
}

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
fn user_interrupted(restore: &mut CursorRestore) -> CaptureError {
    restore.disarm();
    CaptureError::LongshotAutoUserInterrupted
}

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub(super) fn with_scroll<T>(
    target: &LongshotAutoTarget,
    direction: LongshotAutoDirection,
    capture: impl FnOnce() -> Result<T, CaptureError>,
) -> Result<T, CaptureError> {
    use enigo::Mouse;

    ensure_backend_available()?;
    let settings = enigo::Settings {
        linux_delay: 0,
        open_prompt_to_get_permissions: false,
        ..enigo::Settings::default()
    };
    let mut enigo = enigo::Enigo::new(&settings)
        .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?;
    let original = pointer_location()?;
    let mut restore = CursorRestore::new(original);
    move_pointer(target.point)?;
    std::thread::sleep(std::time::Duration::from_millis(POINTER_SETTLE_MS));
    let actual = pointer_location()?;
    if !pointer_near(actual, target.point) {
        return Err(user_interrupted(&mut restore));
    }

    let current_window = pointer_window()?;
    lock_target_window(&target.window, current_window)?;
    ensure_target_can_receive_input(current_window)?;
    if !pointer_near(pointer_location()?, target.point) {
        return Err(user_interrupted(&mut restore));
    }
    if pointer_window()? != current_window {
        return Err(CaptureError::LongshotAutoTargetLost);
    }

    let (axis, length) = direction.input();
    enigo
        .scroll(length, axis)
        .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?;
    std::thread::sleep(std::time::Duration::from_millis(CONTENT_SETTLE_MS));
    let after_scroll = pointer_location()?;
    if !pointer_near(after_scroll, target.point) {
        return Err(user_interrupted(&mut restore));
    }
    if pointer_window()? != current_window {
        return Err(CaptureError::LongshotAutoTargetLost);
    }

    let result = capture()?;
    let after_capture = pointer_location()?;
    if !pointer_near(after_capture, target.point) {
        return Err(user_interrupted(&mut restore));
    }
    if pointer_window()? != current_window {
        return Err(CaptureError::LongshotAutoTargetLost);
    }
    Ok(result)
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub(super) fn with_scroll<T>(
    _target: &LongshotAutoTarget,
    _direction: LongshotAutoDirection,
    _capture: impl FnOnce() -> Result<T, CaptureError>,
) -> Result<T, CaptureError> {
    Err(CaptureError::LongshotAutoUnsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_tolerance_has_a_hard_boundary() {
        assert!(pointer_near((100, 100), (103, 97)));
        assert!(!pointer_near((100, 100), (104, 100)));
    }

    #[test]
    fn target_window_is_locked_once_and_a_replacement_is_rejected() {
        let window = Mutex::new(None);
        let first = WindowIdentity {
            native_id: 42,
            process_id: 100,
        };
        lock_target_window(&window, first).unwrap();
        lock_target_window(&window, first).unwrap();
        assert_eq!(*window.lock().unwrap(), Some(first));
        assert_eq!(
            lock_target_window(
                &window,
                WindowIdentity {
                    native_id: 42,
                    process_id: 101,
                },
            )
            .unwrap_err()
            .code(),
            "longshot_auto_target_lost"
        );
        assert_eq!(
            lock_target_window(
                &window,
                WindowIdentity {
                    native_id: 43,
                    process_id: 100,
                },
            )
            .unwrap_err()
            .code(),
            "longshot_auto_target_lost"
        );
        assert_eq!(*window.lock().unwrap(), Some(first));
    }

    #[test]
    fn missing_and_own_windows_are_rejected_before_locking() {
        assert_eq!(
            checked_identity(0, 0, 12).unwrap_err().code(),
            "longshot_auto_target_lost"
        );
        assert_eq!(
            checked_identity(7, 12, 12).unwrap_err().code(),
            "longshot_auto_target_lost"
        );
        assert_eq!(
            checked_identity(7, 0, 12).unwrap(),
            WindowIdentity {
                native_id: 7,
                process_id: 0,
            }
        );
    }

    #[test]
    fn directions_have_stable_wire_values() {
        assert_eq!(
            serde_json::to_string(&LongshotAutoDirection::Down).unwrap(),
            "\"down\""
        );
        assert_eq!(
            serde_json::from_str::<LongshotAutoDirection>("\"left\"").unwrap(),
            LongshotAutoDirection::Left
        );
    }
}
