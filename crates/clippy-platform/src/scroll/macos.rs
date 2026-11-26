use super::{coordinates, ControlWindow, Direction};
use crate::capture::{resolve_native_monitor, Region};
use anyhow::{ensure, Context, Result};
use objc2::{
    rc::{autoreleasepool, Retained},
    MainThreadMarker,
};
use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSView};
use objc2_core_foundation::{
    CFArray, CFDictionary, CFNumber, CFNumberType, CFRetained, CFString, CFType, CGPoint, CGRect,
};
use objc2_core_graphics::{
    kCGWindowAlpha, kCGWindowBounds, kCGWindowLayer, kCGWindowNumber, kCGWindowOwnerPID,
    CGDisplayBounds, CGError, CGEvent, CGEventFlags, CGEventSource, CGEventSourceStateID,
    CGEventTapLocation, CGMouseButton, CGPreflightPostEventAccess,
    CGRectMakeWithDictionaryRepresentation, CGScrollEventUnit, CGWarpMouseCursorPosition,
    CGWindowListCopyWindowInfo, CGWindowListOption,
};
use std::{ffi::c_void, ptr::NonNull};

type WindowInfo = CFDictionary<CFString, CFType>;

#[derive(Clone, Copy, PartialEq, Eq)]
struct WindowIdentity {
    id: u32,
    pid: i32,
    layer: i64,
}

pub(super) struct Target {
    anchor: CGPoint,
    locked: WindowIdentity,
    application: Retained<NSRunningApplication>,
    pointers: Option<(CGPoint, CGPoint)>,
}

pub(super) fn control_window(view: NonNull<c_void>) -> Result<u32> {
    MainThreadMarker::new().context("The macOS scrolling controls require the main thread")?;
    // 借用的 AppKit 窗口句柄保证 NSView 在这个主线程调用期间存活。
    let view = unsafe { view.cast::<NSView>().as_ref() };
    let window = view
        .window()
        .context("The macOS scrolling controls have no window")?;
    let number = u32::try_from(window.windowNumber())?;
    ensure!(
        number != 0,
        "The macOS scrolling controls have no window number"
    );
    Ok(number)
}

impl Target {
    pub(super) fn open(region: &Region) -> Result<Option<Self>> {
        ensure!(
            CGPreflightPostEventAccess(),
            "Enable Accessibility for Clippy, then start a new scrolling capture"
        );
        let display = resolve_native_monitor(&region.output, region.geometry)?;
        let display = CGDisplayBounds(display.id()?);
        let (x, y) = coordinates::logical(
            [display.origin.x, display.origin.y],
            [display.size.width, display.size.height],
            region.source_size,
            region.bounds,
        )?;
        let anchor = CGPoint { x, y };
        let Some(locked) = identity_at(anchor)?
            .filter(|window| window.layer == 0 && window.pid as u32 != std::process::id())
        else {
            return Ok(None);
        };
        let application = autoreleasepool(|_| {
            NSRunningApplication::runningApplicationWithProcessIdentifier(locked.pid)
        })
        .context("The selected application is no longer running")?;
        Ok(Some(Self {
            anchor,
            locked,
            application,
            pointers: None,
        }))
    }

    fn check_target(&self) -> Result<()> {
        ensure!(
            identity_at(self.anchor)? == Some(self.locked),
            "Scroll target changed"
        );
        Ok(())
    }
}

impl super::Target for Target {
    fn control_is_hidden(&self, ControlWindow(window): ControlWindow) -> Result<bool> {
        let windows = on_screen_windows()?;
        Ok(!windows.iter().any(|info| {
            integer(&info, unsafe { kCGWindowNumber }) == Some(i64::from(window))
                && integer(&info, unsafe { kCGWindowOwnerPID })
                    == Some(i64::from(std::process::id()))
        }))
    }

    fn activate(&self) -> Result<()> {
        ensure_permission()?;
        self.check_target()?;
        ensure_unmodified()?;
        ensure!(
            autoreleasepool(|_| self
                .application
                .activateWithOptions(NSApplicationActivationOptions::empty())),
            "macOS did not activate the selected application"
        );
        Ok(())
    }

    fn is_active(&self) -> bool {
        autoreleasepool(|_| self.application.isActive())
    }

    fn position_pointer(&mut self) -> Result<()> {
        ensure_permission()?;
        self.check_target()?;
        ensure!(
            self.is_active(),
            "Scroll target is no longer in the foreground"
        );
        ensure_unmodified()?;
        let original = pointer()?;
        move_pointer(self.anchor)?;
        let parked = pointer()?;
        ensure!(
            (parked.x - self.anchor.x)
                .abs()
                .max((parked.y - self.anchor.y).abs())
                <= 3.0,
            "Pointer moved. Automatic scrolling stopped"
        );
        self.pointers = Some((original, parked));
        Ok(())
    }

    fn check(&self) -> Result<()> {
        let (_, parked) = self.pointers.expect("自动滚动前已定位指针");
        ensure!(
            pointer()? == parked,
            "Pointer moved. Automatic scrolling stopped"
        );
        ensure!(
            self.is_active(),
            "Scroll target is no longer in the foreground"
        );
        ensure_unmodified()
    }

    fn verify_frame_target(&self) -> Result<()> {
        self.check_target()?;
        self.check()
    }

    fn step(&self, direction: Direction) -> Result<()> {
        ensure_permission()?;
        let (vertical, horizontal) = wheel(direction);
        let event = CGEvent::new_scroll_wheel_event2(
            None,
            CGScrollEventUnit::Line,
            2,
            vertical,
            horizontal,
            0,
        )
        .context("Could not create the macOS scroll event")?;
        CGEvent::set_location(Some(&event), self.anchor);
        self.verify_frame_target()?;
        CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
        Ok(())
    }

    fn finish(&mut self, restore_pointer: bool) -> Result<()> {
        if let Some((original, parked)) = self.pointers.take().filter(|_| restore_pointer) {
            if pointer()? == parked && CGPreflightPostEventAccess() {
                move_pointer(original)?;
            }
        }
        Ok(())
    }
}

fn pointer() -> Result<CGPoint> {
    let event = CGEvent::new(None).context("Could not read the macOS pointer")?;
    Ok(CGEvent::location(Some(&event)))
}

fn move_pointer(point: CGPoint) -> Result<()> {
    let result = CGWarpMouseCursorPosition(point);
    ensure!(
        result == CGError::Success,
        "Could not move the macOS pointer ({})",
        result.0
    );
    Ok(())
}

fn ensure_permission() -> Result<()> {
    ensure!(
        CGPreflightPostEventAccess(),
        "macOS Accessibility permission is required to scroll"
    );
    Ok(())
}

fn ensure_unmodified() -> Result<()> {
    let state = CGEventSourceStateID::HIDSystemState;
    let modifiers = CGEventFlags::MaskShift
        | CGEventFlags::MaskControl
        | CGEventFlags::MaskAlternate
        | CGEventFlags::MaskCommand;
    ensure!(
        !CGEventSource::flags_state(state).intersects(modifiers)
            && ![
                CGMouseButton::Left,
                CGMouseButton::Right,
                CGMouseButton::Center
            ]
            .into_iter()
            .any(|button| CGEventSource::button_state(state, button)),
        "Release keys and mouse buttons to use automatic scrolling"
    );
    Ok(())
}

fn on_screen_windows() -> Result<CFRetained<CFArray<WindowInfo>>> {
    let windows = CGWindowListCopyWindowInfo(
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
        0,
    )
    .context("Could not read the macOS window list")?;
    // CoreGraphics 返回只读数组，元素是以 CFString 为键的窗口字典。
    Ok(unsafe { CFRetained::cast_unchecked(windows) })
}

fn identity_at(point: CGPoint) -> Result<Option<WindowIdentity>> {
    let windows = on_screen_windows()?;
    for info in windows.iter() {
        let Some(rect) = bounds(&info) else {
            continue;
        };
        if point.x >= rect.origin.x
            && point.x < rect.origin.x + rect.size.width
            && point.y >= rect.origin.y
            && point.y < rect.origin.y + rect.size.height
            && alpha(&info).is_none_or(|alpha| alpha > 0.0)
        {
            let id = integer(&info, unsafe { kCGWindowNumber })
                .and_then(|number| u32::try_from(number).ok())
                .filter(|number| *number != 0)
                .context("The scroll target has no macOS window number")?;
            let pid = integer(&info, unsafe { kCGWindowOwnerPID })
                .and_then(|value| i32::try_from(value).ok())
                .filter(|value| *value > 0)
                .context("The scroll target has no macOS owner")?;
            let layer = integer(&info, unsafe { kCGWindowLayer })
                .context("The scroll target has no macOS window layer")?;
            return Ok(Some(WindowIdentity { id, pid, layer }));
        }
    }
    Ok(None)
}

fn bounds(info: &WindowInfo) -> Option<CGRect> {
    let encoded = info
        .get(unsafe { kCGWindowBounds })?
        .downcast::<CFDictionary>()
        .ok()?;
    let mut rect = CGRect::default();
    unsafe { CGRectMakeWithDictionaryRepresentation(Some(&encoded), &mut rect) }.then_some(rect)
}

fn integer(info: &WindowInfo, key: &CFString) -> Option<i64> {
    let number = info.get(key)?.downcast::<CFNumber>().ok()?;
    let mut value = 0i64;
    unsafe { number.value(CFNumberType::LongLongType, (&mut value as *mut i64).cast()) }
        .then_some(value)
}

fn alpha(info: &WindowInfo) -> Option<f64> {
    let number = info
        .get(unsafe { kCGWindowAlpha })?
        .downcast::<CFNumber>()
        .ok()?;
    let mut value = 0.0f64;
    unsafe { number.value(CFNumberType::DoubleType, (&mut value as *mut f64).cast()) }
        .then_some(value)
}

fn wheel(direction: Direction) -> (i32, i32) {
    match direction {
        Direction::Up => (1, 0),
        Direction::Down => (-1, 0),
        Direction::Left => (0, 1),
        Direction::Right => (0, -1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_uses_quartz_line_directions() {
        assert_eq!(wheel(Direction::Up), (1, 0));
        assert_eq!(wheel(Direction::Down), (-1, 0));
        assert_eq!(wheel(Direction::Left), (0, 1));
        assert_eq!(wheel(Direction::Right), (0, -1));
    }
}
