use crate::identity::PlatformIdentity;
use anyhow::{bail, Result};
use global_hotkey::{hotkey::HotKey, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::sync::{Arc, Mutex};

#[cfg(target_os = "linux")]
mod gnome;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Open,
    Pin,
    Capture,
}

type Dispatch = Arc<dyn Fn(Action) + Send + Sync>;
type Report = Arc<dyn Fn(String) + Send + Sync>;

pub struct Shortcuts {
    native: Option<NativeShortcuts>,
    #[cfg(target_os = "linux")]
    wayland: Option<wayland::Worker>,
}

pub struct Bindings([(Action, HotKey); 3]);

struct NativeShortcuts {
    manager: GlobalHotKeyManager,
    registered: Vec<HotKey>,
    active: Arc<Mutex<Vec<(Action, HotKey)>>>,
}

pub fn can_parse(shortcut: &str) -> bool {
    shortcut.parse::<HotKey>().is_ok()
}

pub fn validate(panel: &str, pin: &str, capture: &str) -> Result<Bindings> {
    let panel = panel
        .parse::<HotKey>()
        .map_err(|error| anyhow::anyhow!("Clipboard shortcut: {error}"))?;
    let pin = pin
        .parse::<HotKey>()
        .map_err(|error| anyhow::anyhow!("Pin shortcut: {error}"))?;
    let capture = capture
        .parse::<HotKey>()
        .map_err(|error| anyhow::anyhow!("Screenshot shortcut: {error}"))?;
    if panel.id() == capture.id() || panel.id() == pin.id() || pin.id() == capture.id() {
        bail!("The clipboard, pin and screenshot shortcuts must differ");
    }
    Ok(Bindings([
        (Action::Open, panel),
        (Action::Pin, pin),
        (Action::Capture, capture),
    ]))
}

impl Shortcuts {
    pub fn start(
        identity: &PlatformIdentity,
        panel: &str,
        pin: &str,
        capture: &str,
        dispatch: impl Fn(Action) + Send + Sync + 'static,
        report: impl Fn(String) + Send + Sync + 'static,
    ) -> Result<Self> {
        let report: Report = Arc::new(report);
        let mut bindings = Vec::new();
        for (action, config) in [
            (Action::Open, panel),
            (Action::Pin, pin),
            (Action::Capture, capture),
        ] {
            if config.trim().is_empty() {
                continue;
            }
            match config.parse::<HotKey>() {
                Ok(key)
                    if !bindings
                        .iter()
                        .any(|(_, used): &(_, HotKey)| used.id() == key.id()) =>
                {
                    bindings.push((action, key));
                }
                Ok(_) => report(format!(
                    "{action:?} shortcut conflicts with another Clippy action"
                )),
                Err(error) => report(format!("{action:?} shortcut: {error}")),
            }
        }
        let dispatch: Dispatch = Arc::new(dispatch);
        #[cfg(target_os = "linux")]
        if crate::session::is_wayland() {
            if bindings.is_empty() {
                if !crate::session::is_gnome() {
                    bail!("No global shortcut is configured");
                }
                report("No global shortcut is configured".into());
            }
            return Ok(Self {
                native: None,
                wayland: Some(wayland::Worker::start(
                    *identity, bindings, dispatch, report,
                )?),
            });
        }
        let _ = identity;
        if bindings.is_empty() {
            report("No global shortcut is configured".into());
        }
        let manager = GlobalHotKeyManager::new()?;
        bindings.retain(|(action, key)| match manager.register(*key) {
            Ok(()) => true,
            Err(error) => {
                report(format!("{action:?} shortcut: {error}"));
                false
            }
        });
        let registered = bindings.iter().map(|(_, key)| *key).collect();
        let active = Arc::new(Mutex::new(bindings));
        let dispatch_bindings = Arc::clone(&active);
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state() == HotKeyState::Pressed {
                let action = dispatch_bindings
                    .lock()
                    .unwrap()
                    .iter()
                    .find_map(|(action, key)| (key.id() == event.id()).then_some(*action));
                if let Some(action) = action {
                    dispatch(action);
                }
            }
        }));
        Ok(Self {
            native: Some(NativeShortcuts {
                manager,
                registered,
                active,
            }),
            #[cfg(target_os = "linux")]
            wayland: None,
        })
    }

    pub fn rebind_native(
        &mut self,
        next: Bindings,
        persist: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        self.native
            .as_mut()
            .expect("调用方使用原生快捷键后端")
            .rebind(next, persist)
    }

    #[cfg(target_os = "linux")]
    pub fn rebind_gnome(
        &self,
        bindings: Bindings,
        persist: impl FnOnce() -> Result<()> + Send + 'static,
        completed: impl FnOnce(Result<()>) + Send + 'static,
    ) -> Result<()> {
        self.wayland
            .as_ref()
            .expect("调用方使用 GNOME Wayland 快捷键后端")
            .update_gnome(gnome::Update {
                bindings,
                persist: Box::new(persist),
                completed: Box::new(completed),
            })
    }
}

impl NativeShortcuts {
    fn set_registered(&mut self, key: HotKey, registered: bool) -> global_hotkey::Result<()> {
        if registered {
            self.manager.register(key)?;
            self.registered.push(key);
        } else {
            self.manager.unregister(key)?;
            self.registered.retain(|current| *current != key);
        }
        Ok(())
    }

    fn rebind(&mut self, next: Bindings, persist: impl FnOnce() -> Result<()>) -> Result<()> {
        let mut rollback = Vec::new();
        let applied = (|| {
            for (action, key) in next.0 {
                if !self.registered.contains(&key) {
                    self.set_registered(key, true)
                        .map_err(|error| anyhow::anyhow!("{action:?} shortcut: {error}"))?;
                    rollback.push((key, false));
                }
            }
            for key in self.registered.clone() {
                if !next.0.iter().any(|(_, next_key)| *next_key == key) {
                    self.set_registered(key, false)?;
                    rollback.push((key, true));
                }
            }
            persist()
        })();
        if let Err(error) = applied {
            let failures = rollback
                .into_iter()
                .rev()
                .filter_map(|(key, registered)| self.set_registered(key, registered).err())
                .map(|error| error.to_string())
                .collect::<Vec<_>>();
            if failures.is_empty() {
                return Err(error);
            }
            bail!(
                "{error}. Could not restore shortcuts: {}",
                failures.join(", ")
            );
        }
        // 交换动作使用的按键只需换分派表，仍然需要的物理按键始终保留注册。
        *self.active.lock().unwrap() = next.0.into();
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod wayland {
    use super::*;
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;
    use global_hotkey::hotkey::Modifiers;
    use std::time::Duration;
    use std::{sync::mpsc, thread};
    use tokio::sync::oneshot;

    pub(super) struct Worker {
        gnome_updates: Option<mpsc::SyncSender<gnome::Update>>,
        stop: Option<oneshot::Sender<()>>,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl Worker {
        pub(super) fn start(
            identity: PlatformIdentity,
            bindings: Vec<(Action, HotKey)>,
            dispatch: Dispatch,
            report: Report,
        ) -> std::io::Result<Self> {
            let gnome = crate::session::is_gnome();
            let (send_update, updates) = mpsc::sync_channel(1);
            let (stop, stopped) = oneshot::channel();
            let thread = thread::Builder::new()
                .name("clippy-shortcuts".into())
                .spawn(move || {
                    let result = if gnome {
                        gnome::run(identity, &bindings, dispatch, &report, updates)
                    } else {
                        tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .map_err(anyhow::Error::from)
                            .and_then(|runtime| {
                                runtime.block_on(portal_loop(
                                    identity, &bindings, &dispatch, &report, stopped,
                                ))
                            })
                    };
                    if let Err(error) = result {
                        report(format!("Global shortcuts unavailable: {error}"));
                    }
                })?;
            Ok(Self {
                gnome_updates: gnome.then_some(send_update),
                stop: Some(stop),
                thread: Some(thread),
            })
        }

        pub(super) fn update_gnome(&self, update: gnome::Update) -> Result<()> {
            self.gnome_updates
                .as_ref()
                .expect("调用方使用 GNOME 快捷键后端")
                .try_send(update)
                .map_err(|_| anyhow::anyhow!("GNOME global shortcuts are unavailable"))
        }
    }

    impl Drop for Worker {
        fn drop(&mut self) {
            drop(self.gnome_updates.take());
            if let Some(stop) = self.stop.take() {
                let _ = stop.send(());
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    pub(super) fn trigger(hotkey: HotKey, gnome: bool) -> Result<String> {
        let code = hotkey.key.to_string();
        let key = code
            .strip_prefix("Key")
            .or_else(|| code.strip_prefix("Digit"))
            .unwrap_or(&code);
        let key = match key {
            "ArrowDown" => "Down",
            "ArrowUp" => "Up",
            "ArrowLeft" => "Left",
            "ArrowRight" => "Right",
            "Enter" => "Return",
            "Space" => "space",
            "Backquote" => "grave",
            other
                if other.len() == 1
                    || other.starts_with('F') && other[1..].parse::<u8>().is_ok() =>
            {
                other
            }
            _ => bail!("This key is unavailable in the desktop shortcuts backend"),
        };
        let mut result = String::new();
        for (flag, name, portal) in [
            (Modifiers::CONTROL, "Control", "CTRL"),
            (Modifiers::ALT, "Alt", "ALT"),
            (Modifiers::SHIFT, "Shift", "SHIFT"),
            (Modifiers::SUPER, "Super", "LOGO"),
        ] {
            if hotkey.mods.contains(flag) {
                if gnome {
                    result.push_str(&format!("<{name}>"));
                } else {
                    result.push_str(portal);
                    result.push('+');
                }
            }
        }
        if key.len() == 1 {
            result.push_str(&key.to_ascii_lowercase());
        } else {
            result.push_str(key);
        }
        Ok(result)
    }

    async fn portal_loop(
        identity: PlatformIdentity,
        bindings: &[(Action, HotKey)],
        dispatch: &Dispatch,
        report: &Report,
        mut stop: oneshot::Receiver<()>,
    ) -> Result<()> {
        let portal = GlobalShortcuts::new().await?;
        let session = portal.create_session(Default::default()).await?;
        let result = async {
            let session_id = serde_json::to_value(&session)?;
            let mut active = Box::pin(portal.receive_activated().await?);
            let mut closed = Box::pin(session.receive_closed().await?);
            let mut requested = Vec::new();
            for &(action, key) in bindings {
                let (suffix, label) = match action {
                    Action::Open => ("open", "Open Clipboard"),
                    Action::Pin => ("pin", "Pin selected clipboard item"),
                    Action::Capture => ("capture", "Screenshot"),
                };
                let preferred = trigger(key, false)?;
                let id = format!("{}.{}", identity.shortcut_namespace, suffix);
                let shortcut = NewShortcut::new(&id, label).preferred_trigger(Some(preferred.as_str()));
                requested.push((action, id, shortcut));
            }
            let shortcuts = requested.iter().map(|(_, _, key)| key.clone()).collect::<Vec<_>>();
            let request = tokio::select! {
                _ = &mut stop => return Ok(()),
                result = portal.bind_shortcuts(&session, &shortcuts, None, Default::default()) => result?,
            };
            let response = request.response()?;
            let accepted = response.shortcuts();
            if accepted.len() != requested.len() {
                report("Some global shortcuts were not granted by the desktop".to_owned());
            }
            loop {
                tokio::select! {
                    _ = &mut stop => return Ok(()),
                    _ = closed.next() => bail!("the desktop closed the shortcut session"),
                    event = active.next() => {
                        let Some(event) = event else { bail!("portal disconnected") };
                        if session_id.as_str() == Some(event.session_handle().as_str()) {
                            if let Some((action, _, _)) = requested.iter().find(|(_, id, _)| {
                                id == event.shortcut_id()
                                    && accepted.iter().any(|key| key.id() == id)
                            }) {
                                dispatch(*action);
                            }
                        }
                    }
                }
            }
        }.await;
        let _ = tokio::time::timeout(Duration::from_secs(1), session.close()).await;
        result
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn portal_shortcut_syntax_matches_desktop_specification() {
            let key = "Ctrl+Shift+S".parse().unwrap();
            assert_eq!(trigger(key, false).unwrap(), "CTRL+SHIFT+s");
        }
    }
}
