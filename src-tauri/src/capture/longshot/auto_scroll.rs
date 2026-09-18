//! 自动长截图的平台输入边界。
//!
//! 第一阶段只在 X11 暴露能力：当前 Linux 依赖只启用了 Enigo 的 X11 后端。Wayland
//! 必须建立用户授权的 RemoteDesktop/libei 会话，不能借 XWayland 假装控制原生窗口。

use super::CaptureError;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[cfg(target_os = "linux")]
const SCROLL_TICKS: i32 = 5;
#[cfg(any(test, target_os = "linux"))]
const POINTER_TOLERANCE: i32 = 3;
#[cfg(target_os = "linux")]
const POINTER_SETTLE_MS: u64 = 45;
#[cfg(target_os = "linux")]
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
    #[cfg(target_os = "linux")]
    fn input(self) -> (enigo::Axis, i32) {
        match self {
            Self::Down => (enigo::Axis::Vertical, SCROLL_TICKS),
            Self::Up => (enigo::Axis::Vertical, -SCROLL_TICKS),
            Self::Right => (enigo::Axis::Horizontal, SCROLL_TICKS),
            Self::Left => (enigo::Axis::Horizontal, -SCROLL_TICKS),
        }
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(in crate::capture) struct LongshotAutoTarget {
    point: (i32, i32),
    /// X11 根窗口下的直接 child 是稳定的顶层窗口身份；首步锁定，后续逐步复核。
    window: Arc<Mutex<Option<u32>>>,
}

impl LongshotAutoTarget {
    pub(super) fn new(point: (i32, i32)) -> Self {
        Self {
            point,
            window: Arc::new(Mutex::new(None)),
        }
    }
}

#[cfg(target_os = "linux")]
struct CursorRestore {
    enigo: enigo::Enigo,
    original: (i32, i32),
}

#[cfg(target_os = "linux")]
impl Drop for CursorRestore {
    fn drop(&mut self) {
        if let Err(error) = move_pointer(self.original) {
            log::warn!("恢复自动长截图鼠标位置失败: {error}");
        }
    }
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
fn pointer_window() -> Result<u32, CaptureError> {
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
    Ok(reply.child)
}

#[cfg(any(test, target_os = "linux"))]
fn pointer_near(actual: (i32, i32), expected: (i32, i32)) -> bool {
    actual.0.abs_diff(expected.0) <= POINTER_TOLERANCE as u32
        && actual.1.abs_diff(expected.1) <= POINTER_TOLERANCE as u32
}

#[cfg(any(test, target_os = "linux"))]
fn lock_target_window(window: &Mutex<Option<u32>>, current: u32) -> Result<(), CaptureError> {
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

#[cfg(target_os = "linux")]
pub(super) fn with_scroll<T>(
    target: &LongshotAutoTarget,
    direction: LongshotAutoDirection,
    capture: impl FnOnce() -> Result<T, CaptureError>,
) -> Result<T, CaptureError> {
    use enigo::Mouse;

    if crate::platform::current_session() != crate::platform::DesktopSession::X11 {
        return Err(CaptureError::LongshotAutoUnsupported);
    }
    let settings = enigo::Settings {
        linux_delay: 0,
        open_prompt_to_get_permissions: false,
        ..enigo::Settings::default()
    };
    let enigo = enigo::Enigo::new(&settings)
        .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?;
    let original = enigo
        .location()
        .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?;
    let mut cursor = CursorRestore { enigo, original };
    move_pointer(target.point)?;
    std::thread::sleep(std::time::Duration::from_millis(POINTER_SETTLE_MS));
    let actual = cursor
        .enigo
        .location()
        .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?;
    if !pointer_near(actual, target.point) {
        return Err(CaptureError::LongshotAutoUserInterrupted);
    }

    let current_window = pointer_window()?;
    lock_target_window(&target.window, current_window)?;

    let (axis, length) = direction.input();
    cursor
        .enigo
        .scroll(length, axis)
        .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?;
    std::thread::sleep(std::time::Duration::from_millis(CONTENT_SETTLE_MS));
    let after_scroll = cursor
        .enigo
        .location()
        .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?;
    if !pointer_near(after_scroll, target.point) {
        return Err(CaptureError::LongshotAutoUserInterrupted);
    }
    if pointer_window()? != current_window {
        return Err(CaptureError::LongshotAutoTargetLost);
    }

    let result = capture()?;
    let after_capture = cursor
        .enigo
        .location()
        .map_err(|error| CaptureError::LongshotAutoInput(error.to_string()))?;
    if !pointer_near(after_capture, target.point) {
        return Err(CaptureError::LongshotAutoUserInterrupted);
    }
    Ok(result)
}

#[cfg(not(target_os = "linux"))]
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
        lock_target_window(&window, 42).unwrap();
        lock_target_window(&window, 42).unwrap();
        assert_eq!(*window.lock().unwrap(), Some(42));
        assert_eq!(
            lock_target_window(&window, 7).unwrap_err().code(),
            "longshot_auto_target_lost"
        );
        assert_eq!(*window.lock().unwrap(), Some(42));
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
