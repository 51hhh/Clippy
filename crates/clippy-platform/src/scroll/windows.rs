use super::{coordinates, ControlWindow, Direction};
use crate::capture::{resolve_native_monitor, Region};
use anyhow::{bail, ensure, Context, Result};
use std::{
    io, mem,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::{HANDLE, HWND, POINT},
    Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, IsValidSid,
        TokenIntegrityLevel, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
    },
    System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    },
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_HWHEEL,
            MOUSEEVENTF_WHEEL, MOUSEINPUT, MOUSE_EVENT_FLAGS, VK_CONTROL, VK_LBUTTON, VK_LWIN,
            VK_MBUTTON, VK_MENU, VK_RBUTTON, VK_RWIN, VK_SHIFT, VK_XBUTTON1, VK_XBUTTON2,
        },
        WindowsAndMessaging::{
            GetAncestor, GetDesktopWindow, GetForegroundWindow, GetPhysicalCursorPos,
            GetShellWindow, GetWindowThreadProcessId, IsWindowVisible, SetForegroundWindow,
            SetPhysicalCursorPos, WindowFromPhysicalPoint, GA_ROOT, WHEEL_DELTA,
        },
    },
};

type Point = (i32, i32);

#[derive(Clone, Copy, PartialEq, Eq)]
struct WindowIdentity {
    window: HWND,
    pid: u32,
}

pub(super) struct Target {
    anchor: Point,
    locked: WindowIdentity,
    integrity: u32,
    original_pointer: Option<Point>,
}

impl Target {
    pub(super) fn open(region: &Region) -> Result<Option<Self>> {
        let monitor = resolve_native_monitor(&region.output, region.geometry)?;
        ensure!(
            (monitor.width()?, monitor.height()?) == region.source_size,
            "Capture output changed size"
        );
        let anchor = coordinates::physical(
            [region.geometry.x, region.geometry.y],
            region.source_size,
            region.bounds,
        )?;
        let Some(locked) = identity_at(anchor).filter(|target| target.pid != std::process::id())
        else {
            return Ok(None);
        };
        let integrity = process_integrity(unsafe { GetCurrentProcess() })?;
        let target = Self {
            anchor,
            locked,
            integrity,
            original_pointer: None,
        };
        target.check_integrity()?;
        Ok(Some(target))
    }

    fn check_target(&self) -> Result<()> {
        ensure!(
            identity_at(self.anchor) == Some(self.locked),
            "Scroll target changed"
        );
        Ok(())
    }

    fn check_integrity(&self) -> Result<()> {
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, self.locked.pid) };
        if process.is_null() {
            return Err(io::Error::last_os_error()).context("Could not open the scroll target");
        }
        // OpenProcess 转移了该句柄的所有权。
        let process = unsafe { OwnedHandle::from_raw_handle(process) };
        ensure!(
            process_integrity(process.as_raw_handle())? <= self.integrity,
            "Automatic scrolling is unavailable for a window running with higher privileges"
        );
        Ok(())
    }
}

impl super::Target for Target {
    fn control_is_hidden(&self, ControlWindow(window): ControlWindow) -> Result<bool> {
        Ok(unsafe { IsWindowVisible(window.get() as HWND) } == 0)
    }

    fn activate(&self) -> Result<()> {
        self.check_target()?;
        self.check_integrity()?;
        ensure_unmodified()?;
        ensure!(
            unsafe { SetForegroundWindow(self.locked.window) } != 0,
            "Windows did not activate the selected window"
        );
        Ok(())
    }

    fn is_active(&self) -> bool {
        top_level(unsafe { GetForegroundWindow() }) == Some(self.locked.window)
    }

    fn position_pointer(&mut self) -> Result<()> {
        self.check_target()?;
        self.check_integrity()?;
        ensure!(
            self.is_active(),
            "Scroll target is no longer in the foreground"
        );
        ensure_unmodified()?;
        self.original_pointer = Some(pointer()?);
        move_pointer(self.anchor)?;
        self.check()
    }

    fn check(&self) -> Result<()> {
        ensure!(
            pointer()? == self.anchor,
            "Pointer moved. Automatic scrolling stopped"
        );
        ensure!(
            self.is_active(),
            "Scroll target is no longer in the foreground"
        );
        self.check_target()?;
        ensure_unmodified()
    }

    fn step(&self, direction: Direction) -> Result<()> {
        self.check_integrity()?;
        self.check()?;
        let (flags, delta) = wheel(direction);
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    mouseData: delta as u32,
                    dwFlags: flags,
                    ..MOUSEINPUT::default()
                },
            },
        };
        let delivered = unsafe { SendInput(1, &input, mem::size_of::<INPUT>() as i32) };
        ensure!(delivered == 1, "Windows did not accept the scroll input");
        Ok(())
    }

    fn finish(&mut self, restore_pointer: bool) -> Result<()> {
        if let Some(original) = self.original_pointer.take().filter(|_| restore_pointer) {
            if pointer()? == self.anchor {
                move_pointer(original)?;
            }
        }
        Ok(())
    }
}

fn top_level(window: HWND) -> Option<HWND> {
    if window.is_null() {
        return None;
    }
    let root = unsafe { GetAncestor(window, GA_ROOT) };
    (!root.is_null()).then_some(root)
}

fn identity_at((x, y): Point) -> Option<WindowIdentity> {
    let window = top_level(unsafe { WindowFromPhysicalPoint(POINT { x, y }) })?;
    if window == unsafe { GetDesktopWindow() }
        || window == unsafe { GetShellWindow() }
        || unsafe { IsWindowVisible(window) } == 0
    {
        return None;
    }
    let mut pid = 0;
    let thread = unsafe { GetWindowThreadProcessId(window, &mut pid) };
    (thread != 0 && pid != 0).then_some(WindowIdentity { window, pid })
}

fn pointer() -> Result<Point> {
    let mut pos = POINT::default();
    if unsafe { GetPhysicalCursorPos(&mut pos) } == 0 {
        return Err(io::Error::last_os_error()).context("Could not read the Windows pointer");
    }
    Ok((pos.x, pos.y))
}

fn move_pointer((x, y): Point) -> Result<()> {
    if unsafe { SetPhysicalCursorPos(x, y) } == 0 {
        return Err(io::Error::last_os_error()).context("Could not move the Windows pointer");
    }
    Ok(())
}

fn ensure_unmodified() -> Result<()> {
    ensure!(
        ![
            VK_SHIFT,
            VK_CONTROL,
            VK_MENU,
            VK_LWIN,
            VK_RWIN,
            VK_LBUTTON,
            VK_RBUTTON,
            VK_MBUTTON,
            VK_XBUTTON1,
            VK_XBUTTON2,
        ]
        .into_iter()
        .any(|key| unsafe { GetAsyncKeyState(i32::from(key)) } < 0),
        "Release keys and mouse buttons to use automatic scrolling"
    );
    Ok(())
}

fn wheel(direction: Direction) -> (MOUSE_EVENT_FLAGS, i32) {
    match direction {
        Direction::Up => (MOUSEEVENTF_WHEEL, WHEEL_DELTA as i32),
        Direction::Down => (MOUSEEVENTF_WHEEL, -(WHEEL_DELTA as i32)),
        Direction::Left => (MOUSEEVENTF_HWHEEL, -(WHEEL_DELTA as i32)),
        Direction::Right => (MOUSEEVENTF_HWHEEL, WHEEL_DELTA as i32),
    }
}

fn process_integrity(process: HANDLE) -> Result<u32> {
    let mut token = null_mut();
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error()).context("Could not open the Windows input token");
    }
    // 成功的 OpenProcessToken 返回由调用方关闭的句柄。
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut required = 0;
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenIntegrityLevel,
            null_mut(),
            0,
            &mut required,
        )
    };
    ensure!(
        required as usize >= mem::size_of::<TOKEN_MANDATORY_LABEL>(),
        "Invalid Windows token integrity length"
    );
    // TokenIntegrityLevel 以对齐的 TOKEN_MANDATORY_LABEL 开头，SID 也在缓冲区内。
    let mut data = vec![0usize; (required as usize).div_ceil(mem::size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenIntegrityLevel,
            data.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(io::Error::last_os_error()).context("Could not read Windows token integrity");
    }
    let sid = unsafe { (*data.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()).Label.Sid };
    ensure!(
        !sid.is_null() && unsafe { IsValidSid(sid) } != 0,
        "Invalid Windows integrity SID"
    );
    let count = unsafe { *GetSidSubAuthorityCount(sid) };
    if count == 0 {
        bail!("Missing Windows integrity RID");
    }
    Ok(unsafe { *GetSidSubAuthority(sid, u32::from(count) - 1) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_uses_one_native_detent_with_both_axis_signs() {
        assert_eq!(wheel(Direction::Up), (MOUSEEVENTF_WHEEL, 120));
        assert_eq!(wheel(Direction::Down), (MOUSEEVENTF_WHEEL, -120));
        assert_eq!(wheel(Direction::Left), (MOUSEEVENTF_HWHEEL, -120));
        assert_eq!(wheel(Direction::Right), (MOUSEEVENTF_HWHEEL, 120));
    }
}
