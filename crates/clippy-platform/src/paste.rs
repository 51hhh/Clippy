use anyhow::{bail, Result};
use std::path::Path;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
mod portal;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Target(Focus);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Focus {
    #[cfg(target_os = "linux")]
    X11(u32),
    #[cfg(target_os = "linux")]
    Wayland,
    #[cfg(target_os = "windows")]
    Win32(usize, u32),
    #[cfg(target_os = "macos")]
    Mac(i32),
}

/// 面板显示前取目标，粘贴时只确认焦点已经自然回到该目标。
pub fn capture_target() -> Option<Target> {
    #[cfg(target_os = "linux")]
    {
        if crate::session::is_wayland() {
            return Some(Target(Focus::Wayland));
        }
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::ConnectionExt;
        let (connection, screen) = x11rb::connect(None).ok()?;
        let focus = connection.get_input_focus().ok()?.reply().ok()?.focus;
        let root = connection.setup().roots[screen].root;
        (focus > 1 && focus != root).then_some(Target(Focus::X11(focus)))
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId,
        };
        let window = unsafe { GetForegroundWindow() };
        if window.is_null() {
            return None;
        }
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(window, &mut pid) };
        (pid != 0 && pid != std::process::id())
            .then_some(Target(Focus::Win32(window as usize, pid)))
    }
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;
        let application = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        let pid = application.processIdentifier();
        (pid > 0 && pid as u32 != std::process::id()).then_some(Target(Focus::Mac(pid)))
    }
}

type Completion = Box<dyn FnOnce(Result<()>) + Send>;

pub struct Paste {
    #[cfg(target_os = "linux")]
    portal: Option<portal::Worker>,
    #[cfg(target_os = "linux")]
    token_path: std::path::PathBuf,
}

impl Paste {
    pub fn new(data_directory: &Path) -> Self {
        let _ = data_directory;
        Self {
            #[cfg(target_os = "linux")]
            portal: None,
            #[cfg(target_os = "linux")]
            token_path: data_directory.join("remote-desktop-restore-token"),
        }
    }

    pub fn authorize(&mut self, complete: Completion) {
        #[cfg(target_os = "linux")]
        if crate::session::is_wayland() {
            self.dispatch_portal(portal::Operation::Authorize, complete);
            return;
        }
        #[cfg(target_os = "macos")]
        {
            let settings = enigo::Settings {
                open_prompt_to_get_permissions: true,
                ..Default::default()
            };
            complete(enigo::Enigo::new(&settings).map(|_| ()).map_err(Into::into));
        }
        #[cfg(not(target_os = "macos"))]
        complete(Ok(()));
    }

    pub fn inject(&mut self, target: Target, complete: Completion) {
        #[cfg(target_os = "linux")]
        if target.0 == Focus::Wayland {
            self.dispatch_portal(portal::Operation::Paste, complete);
            return;
        }
        complete(inject_native(target));
    }

    pub fn close(&mut self) {
        #[cfg(target_os = "linux")]
        {
            self.portal.take();
        }
    }

    #[cfg(target_os = "linux")]
    fn dispatch_portal(&mut self, operation: portal::Operation, complete: Completion) {
        if self.portal.is_none() {
            match portal::Worker::start(self.token_path.clone()) {
                Ok(worker) => self.portal = Some(worker),
                Err(error) => {
                    complete(Err(error.into()));
                    return;
                }
            }
        }
        self.portal
            .as_ref()
            .expect("Portal worker established")
            .dispatch(operation, complete);
    }
}

fn inject_native(target: Target) -> Result<()> {
    use enigo::{Direction::*, Enigo, Key, Keyboard, Settings};
    let settings = Settings {
        open_prompt_to_get_permissions: false,
        ..Settings::default()
    };
    let mut input = Enigo::new(&settings)?;
    wait_for_focus(target, capture_target)?;
    let modifier = if cfg!(target_os = "macos") {
        Key::Meta
    } else {
        Key::Control
    };
    input.key(modifier, Press)?;
    let key = Key::Unicode('v');
    let click = input.key(key, Click);
    if click.is_err() {
        let _ = input.key(key, Release);
    }
    let release = input.key(modifier, Release);
    click?;
    release?;
    Ok(())
}

fn wait_for_focus(target: Target, mut focused: impl FnMut() -> Option<Target>) -> Result<()> {
    let deadline = Instant::now() + Duration::from_millis(450);
    loop {
        match focused() {
            Some(current) if current == target => return Ok(()),
            Some(_) => bail!("Paste target changed"),
            None if Instant::now() >= deadline => bail!("Paste target did not regain focus"),
            None => std::thread::sleep(Duration::from_millis(15)),
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn accepts_own_target_after_focus_settles_and_rejects_third_window() {
        let target = Target(Focus::X11(90));
        let mut sequence = [None, Some(target)].into_iter();
        assert!(wait_for_focus(target, || sequence.next().unwrap()).is_ok());
        assert!(wait_for_focus(target, || Some(Target(Focus::X11(91)))).is_err());
    }
}
