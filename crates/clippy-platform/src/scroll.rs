use crate::capture::Region;
use anyhow::Result;
use raw_window_handle::{RawWindowHandle, WindowHandle};

#[cfg(any(target_os = "windows", target_os = "macos", test))]
mod coordinates;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "linux")]
mod x11;

#[cfg(not(target_os = "windows"))]
type ControlId = u32;
#[cfg(target_os = "windows")]
type ControlId = std::num::NonZeroIsize;

#[derive(Clone, Copy)]
pub struct ControlWindow(ControlId);

impl ControlWindow {
    pub fn from_handle(handle: WindowHandle<'_>) -> Result<Self> {
        match handle.as_raw() {
            #[cfg(target_os = "linux")]
            RawWindowHandle::Xlib(handle) => Ok(Self(u32::try_from(handle.window)?)),
            #[cfg(target_os = "linux")]
            RawWindowHandle::Xcb(handle) => Ok(Self(handle.window.get())),
            #[cfg(target_os = "windows")]
            RawWindowHandle::Win32(handle) => Ok(Self(handle.hwnd)),
            #[cfg(target_os = "macos")]
            RawWindowHandle::AppKit(handle) => macos::control_window(handle.ns_view).map(Self),
            _ => anyhow::bail!("Could not identify the native scrolling controls"),
        }
    }
}

#[derive(Clone, Copy)]
pub enum Direction {
    Down,
    Up,
    Left,
    Right,
}

pub trait Target {
    fn control_is_hidden(&self, window: ControlWindow) -> Result<bool>;
    fn activate(&self) -> Result<()> {
        Ok(())
    }
    fn is_active(&self) -> bool {
        true
    }
    fn position_pointer(&mut self) -> Result<()>;
    fn check(&self) -> Result<()>;
    fn verify_frame_target(&self) -> Result<()> {
        self.check()
    }
    fn step(&self, direction: Direction) -> Result<()>;
    fn finish(&mut self, restore_pointer: bool) -> Result<()>;
}

fn is_x11() -> bool {
    cfg!(target_os = "linux")
        && !crate::session::is_wayland()
        && std::env::var_os("DISPLAY").is_some_and(|value| !value.is_empty())
}

pub fn uses_native_target() -> bool {
    cfg!(any(target_os = "windows", target_os = "macos")) || is_x11()
}

pub fn for_region(region: &Region) -> Result<Option<Box<dyn Target>>> {
    #[cfg(target_os = "windows")]
    let target = windows::Target::open(region)?;
    #[cfg(target_os = "macos")]
    let target = macos::Target::open(region)?;
    #[cfg(target_os = "linux")]
    let target = if is_x11() {
        let rect = crate::capture::x11_region_rect(region.geometry, region.bounds)?;
        x11::Target::open(rect)?
    } else {
        None
    };
    Ok(target.map(|target| Box::new(target) as Box<dyn Target>))
}
