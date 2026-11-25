use crate::identity::PlatformIdentity;
use anyhow::{bail, Result};
use global_hotkey::{hotkey::HotKey, GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Open,
    Capture,
}

type Dispatch = Arc<dyn Fn(Action) + Send + Sync>;
type Report = Arc<dyn Fn(String) + Send + Sync>;

pub struct Shortcuts {
    _native: Option<GlobalHotKeyManager>,
    #[cfg(target_os = "linux")]
    _wayland: Option<wayland::Worker>,
}

pub fn can_parse(shortcut: &str) -> bool {
    shortcut.parse::<HotKey>().is_ok()
}

pub fn validate(panel: &str, capture: &str) -> Result<()> {
    let panel = panel
        .parse::<HotKey>()
        .map_err(|error| anyhow::anyhow!("Clipboard shortcut: {error}"))?;
    let capture = capture
        .parse::<HotKey>()
        .map_err(|error| anyhow::anyhow!("Screenshot shortcut: {error}"))?;
    if panel.id() == capture.id() {
        bail!("The clipboard and screenshot shortcuts must differ");
    }
    Ok(())
}

impl Shortcuts {
    pub fn start(
        identity: &PlatformIdentity,
        panel: &str,
        capture: &str,
        dispatch: impl Fn(Action) + Send + Sync + 'static,
        report: impl Fn(String) + Send + Sync + 'static,
    ) -> Result<Self> {
        let report: Report = Arc::new(report);
        let mut bindings = Vec::new();
        for (action, config) in [(Action::Open, panel), (Action::Capture, capture)] {
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
        if bindings.is_empty() {
            bail!("No global shortcut is configured");
        }
        let dispatch: Dispatch = Arc::new(dispatch);
        #[cfg(target_os = "linux")]
        if crate::session::is_wayland() {
            return Ok(Self {
                _native: None,
                _wayland: Some(wayland::Worker::start(
                    *identity, bindings, dispatch, report,
                )?),
            });
        }
        let _ = identity;
        let manager = GlobalHotKeyManager::new()?;
        bindings.retain(|(action, key)| match manager.register(*key) {
            Ok(()) => true,
            Err(error) => {
                report(format!("{action:?} shortcut: {error}"));
                false
            }
        });
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state() == HotKeyState::Pressed {
                if let Some((action, _)) = bindings.iter().find(|(_, key)| key.id() == event.id()) {
                    dispatch(*action);
                }
            }
        }));
        Ok(Self {
            _native: Some(manager),
            #[cfg(target_os = "linux")]
            _wayland: None,
        })
    }
}

#[cfg(target_os = "linux")]
mod wayland {
    use super::*;
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;
    use global_hotkey::hotkey::Modifiers;
    use std::process::Command;
    use std::thread;
    use std::time::Duration;
    use tokio::sync::oneshot;

    const SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
    const LIST: &str = "custom-keybindings";
    const PREFIX: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";

    pub(super) struct Worker {
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
            let (stop, stopped) = oneshot::channel();
            let thread = thread::Builder::new()
                .name("clippy-shortcuts".into())
                .spawn(move || {
                    let result = if gnome {
                        gnome_loop(identity, &bindings, dispatch, &report, stopped)
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
                stop: Some(stop),
                thread: Some(thread),
            })
        }
    }

    impl Drop for Worker {
        fn drop(&mut self) {
            if let Some(stop) = self.stop.take() {
                let _ = stop.send(());
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    struct ShortcutEndpoint(Dispatch);

    #[zbus::interface(name = "com.clippy.app")]
    impl ShortcutEndpoint {
        fn toggle(&self) {
            (self.0)(Action::Open);
        }

        fn capture(&self) -> bool {
            (self.0)(Action::Capture);
            true
        }
    }

    fn gnome_loop(
        identity: PlatformIdentity,
        bindings: &[(Action, HotKey)],
        dispatch: Dispatch,
        report: &Report,
        stopped: oneshot::Receiver<()>,
    ) -> Result<()> {
        let name = format!("{}.Shortcuts", identity.application_id);
        let _bus = zbus::blocking::connection::Builder::session()?
            .name(name.as_str())?
            .serve_at("/com/clippy/app", ShortcutEndpoint(dispatch))?
            .build()?;
        for &(action, key) in bindings {
            let (method, label) = match action {
                Action::Open => ("Toggle", "Open Clipboard"),
                Action::Capture => ("Capture", "Screenshot"),
            };
            let command =
                format!("dbus-send --session --type=method_call --dest={name} /com/clippy/app com.clippy.app.{method}");
            if let Err(error) = bind_gnome(identity, label, method, key, &command) {
                report(format!("{label} shortcut: {error}"));
            }
        }
        let _ = stopped.blocking_recv();
        Ok(())
    }

    fn bind_gnome(
        identity: PlatformIdentity,
        label: &str,
        method: &str,
        key: HotKey,
        command: &str,
    ) -> Result<()> {
        let accelerator = trigger(key, true)?;
        let raw = gsettings(&["get", SCHEMA, LIST])?;
        let mut entries = parse_entries(&raw)?;
        let expected_command = format!("'{command}'");
        let mut owned = None;
        for entry in &entries {
            let schema = format!("{SCHEMA}.custom-keybinding:{entry}");
            let current_command = gsettings(&["get", &schema, "command"])?;
            if current_command == expected_command {
                owned = Some(entry.clone());
            } else if gsettings(&["get", &schema, "binding"])? == format!("'{accelerator}'") {
                bail!("the shortcut is already assigned in GNOME Settings");
            }
        }
        let slot = format!(
            "{PREFIX}{}-{}/",
            identity.shortcut_namespace.replace('.', "-"),
            method.to_ascii_lowercase()
        );
        let new_slot = owned.is_none();
        let entry = owned.unwrap_or(slot);
        if new_slot && entries.contains(&entry) {
            bail!("the Clippy shortcut slot is already in use");
        }
        let schema = format!("{SCHEMA}.custom-keybinding:{entry}");
        if new_slot {
            gsettings(&["set", &schema, "name", &format!("'Clippy {label}'")])?;
            gsettings(&["set", &schema, "command", &expected_command])?;
        }
        let quoted = format!("'{accelerator}'");
        if gsettings(&["get", &schema, "binding"])? != quoted {
            gsettings(&["set", &schema, "binding", &quoted])?;
        }
        if new_slot {
            // 保留设置页刚写入的其它快捷键，已有条目也不会随应用重启增长。
            let current = gsettings(&["get", SCHEMA, LIST])?;
            if current != raw {
                entries = parse_entries(&current)?;
            }
            if !entries.contains(&entry) {
                entries.push(entry);
            }
            let value = format!("['{}']", entries.join("', '"));
            gsettings(&["set", SCHEMA, LIST, &value])?;
        }
        Ok(())
    }

    fn parse_entries(value: &str) -> Result<Vec<String>> {
        let list = value.strip_prefix("@as ").unwrap_or(value);
        let raw = list
            .strip_prefix('[')
            .and_then(|list| list.strip_suffix(']'))
            .ok_or_else(|| anyhow::anyhow!("invalid GNOME shortcut list"))?;
        if raw.is_empty() {
            return Ok(Vec::new());
        }
        raw.split(',')
            .map(|part| {
                let entry = part
                    .trim()
                    .strip_prefix('\'')
                    .and_then(|part| part.strip_suffix('\''))
                    .filter(|entry| {
                        entry.starts_with('/')
                            && entry.ends_with('/')
                            && entry
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || b"/_-.".contains(&byte))
                    })
                    .ok_or_else(|| anyhow::anyhow!("invalid GNOME shortcut path"))?;
                Ok(entry.to_owned())
            })
            .collect()
    }

    fn gsettings(args: &[&str]) -> Result<String> {
        let result = Command::new("gsettings").args(args).output()?;
        if !result.status.success() {
            bail!("{}", String::from_utf8_lossy(&result.stderr).trim());
        }
        Ok(String::from_utf8(result.stdout)?.trim().to_owned())
    }

    fn trigger(hotkey: HotKey, gnome: bool) -> Result<String> {
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
        fn custom_list_and_shortcut_syntax_preserve_existing_gnome_entries() {
            assert!(parse_entries("@as []").unwrap().is_empty());
            assert_eq!(
                parse_entries("['/org/user/custom0/']").unwrap(),
                ["/org/user/custom0/"]
            );
            assert!(parse_entries("['invalid']").is_err());
            let key = "Ctrl+Shift+S".parse().unwrap();
            assert_eq!(trigger(key, true).unwrap(), "<Control><Shift>s");
            assert_eq!(trigger(key, false).unwrap(), "CTRL+SHIFT+s");
        }
    }
}
