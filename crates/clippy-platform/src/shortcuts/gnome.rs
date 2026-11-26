use super::{wayland::trigger, Action, Bindings, Dispatch, PlatformIdentity, Report};
use crate::gnome_settings::run as gsettings;
use anyhow::{bail, ensure, Result};
use global_hotkey::hotkey::HotKey;
use std::sync::mpsc::Receiver;

const SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
const LIST: &str = "custom-keybindings";
const PREFIX: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";
const MEDIA_KEYS: &str = "org.gnome.SettingsDaemon.MediaKeys";
const SYSTEM_SCHEMAS: [&str; 5] = [
    SCHEMA,
    "org.gnome.desktop.wm.keybindings",
    "org.gnome.mutter.keybindings",
    "org.gnome.mutter.wayland.keybindings",
    "org.gnome.shell.keybindings",
];

pub(super) struct Update {
    pub bindings: Bindings,
    pub persist: Box<dyn FnOnce() -> Result<()> + Send>,
    pub completed: Box<dyn FnOnce(Result<()>) + Send>,
}

struct Endpoint(Dispatch);

#[zbus::interface(name = "com.clippy.app")]
impl Endpoint {
    fn toggle(&self) {
        (self.0)(Action::Open);
    }

    fn pin(&self) {
        (self.0)(Action::Pin);
    }

    fn capture(&self) -> bool {
        (self.0)(Action::Capture);
        true
    }
}

pub(super) fn run(
    identity: PlatformIdentity,
    bindings: &[(Action, HotKey)],
    dispatch: Dispatch,
    report: &Report,
    updates: Receiver<Update>,
) -> Result<()> {
    let service = format!("{}.Shortcuts", identity.application_id);
    let bus = zbus::blocking::connection::Builder::session()?
        .name(service.as_str())?
        .serve_at("/com/clippy/app", Endpoint(dispatch))?
        .build()?;
    let namespace = identity.shortcut_namespace.replace('.', "-");
    let requested = |binding: (Action, HotKey)| Requested::new(binding, &service, &namespace);
    for &binding in bindings {
        let result = requested(binding).and_then(|target| replace(&[target], false, || Ok(())));
        if let Err(error) = result {
            report(format!("{} shortcut: {error}", method(binding.0).1));
        }
    }
    for update in updates {
        let result = (|| {
            let proxy = zbus::blocking::fdo::DBusProxy::new(&bus)?;
            ensure!(
                proxy.name_has_owner(MEDIA_KEYS.try_into()?)?,
                "GNOME media keys service is unavailable"
            );
            let targets = update
                .bindings
                .0
                .map(requested)
                .into_iter()
                .collect::<Result<Vec<_>>>()?;
            replace(&targets, true, update.persist)
        })();
        (update.completed)(result);
    }
    Ok(())
}

fn method(action: Action) -> (&'static str, &'static str) {
    match action {
        Action::Open => ("Toggle", "Open Clipboard"),
        Action::Pin => ("Pin", "Pin selected clipboard item"),
        Action::Capture => ("Capture", "Screenshot"),
    }
}

struct Requested {
    label: &'static str,
    command: String,
    binding: String,
    path: String,
}

impl Requested {
    fn new((action, key): (Action, HotKey), service: &str, namespace: &str) -> Result<Self> {
        let (method, label) = method(action);
        Ok(Self {
            label,
            command: format!(
                "'dbus-send --session --type=method_call --dest={service} /com/clippy/app com.clippy.app.{method}'"
            ),
            binding: format!("'{}'", trigger(key, true)?),
            path: format!("{PREFIX}{namespace}-{}/", method.to_ascii_lowercase()),
        })
    }
}

struct Slot {
    path: String,
    command: String,
    binding: String,
}

impl Slot {
    fn read(path: String) -> Result<Self> {
        let schema = slot_schema(&path);
        Ok(Self {
            path,
            command: gsettings(&["get", &schema, "command"])?,
            binding: gsettings(&["get", &schema, "binding"])?,
        })
    }
}

fn slot_schema(path: &str) -> String {
    format!("{SCHEMA}.custom-keybinding:{path}")
}

struct Change {
    schema: String,
    key: &'static str,
    before: String,
    after: String,
}

impl Change {
    fn stage(
        changes: &mut Vec<Self>,
        schema: &str,
        key: &'static str,
        before: String,
        after: String,
    ) {
        if before != after {
            changes.push(Self {
                schema: schema.into(),
                key,
                before,
                after,
            });
        }
    }

    fn replace(&self, from: &str, to: &str) -> Result<()> {
        let actual = gsettings(&["get", &self.schema, self.key])?;
        ensure!(
            actual == from,
            "GNOME shortcuts changed while saving. Try again."
        );
        gsettings(&["set", &self.schema, self.key, to])?;
        Ok(())
    }
}

fn replace(
    targets: &[Requested],
    check_system: bool,
    persist: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let raw = gsettings(&["get", SCHEMA, LIST])?;
    let mut paths = parse_entries(&raw)?;
    let slots = paths
        .iter()
        .cloned()
        .map(Slot::read)
        .collect::<Result<Vec<_>>>()?;
    let owners = targets
        .iter()
        .map(|target| {
            slots
                .iter()
                .rposition(|slot| slot.command == target.command)
        })
        .collect::<Vec<_>>();
    let owned = owners.iter().flatten().copied().collect::<Vec<_>>();
    let triggers = targets
        .iter()
        .map(|target| normalize(&target.binding))
        .collect::<Vec<_>>();
    for (target, chord) in targets.iter().zip(&triggers) {
        if slots
            .iter()
            .enumerate()
            .any(|(i, slot)| !owned.contains(&i) && normalize(&slot.binding) == *chord)
        {
            bail!(
                "{} shortcut is already assigned in GNOME Settings",
                target.label
            );
        }
    }
    if check_system {
        check_system_shortcuts(targets, &triggers)?;
    }

    // 保留仍在使用的键，互换动作时只改媒体键服务分派的命令。
    let mut assigned = triggers
        .iter()
        .map(|chord| {
            owned
                .iter()
                .copied()
                .find(|&i| normalize(&slots[i].binding) == *chord)
        })
        .collect::<Vec<_>>();
    for i in 0..assigned.len() {
        if assigned[i].is_none() {
            assigned[i] = owners[i]
                .filter(|owner| !assigned.contains(&Some(*owner)))
                .or_else(|| {
                    owned
                        .iter()
                        .copied()
                        .find(|&owner| !assigned.contains(&Some(owner)))
                });
        }
    }
    let previous_count = paths.len();
    let mut changes = Vec::new();
    for ((target, assigned), chord) in targets.iter().zip(assigned).zip(triggers) {
        let slot = match assigned {
            Some(i) => &slots[i],
            None => {
                let path = std::iter::once(&target.path)
                    .chain(targets.iter().map(|other| &other.path))
                    .find(|path| !paths.contains(*path))
                    .ok_or_else(|| {
                        anyhow::anyhow!("The Clippy shortcut slots are already in use")
                    })?;
                let slot = Slot::read(path.clone())?;
                paths.push(path.clone());
                stage_slot(&mut changes, &slot, target, chord)?;
                continue;
            }
        };
        stage_slot(&mut changes, slot, target, chord)?;
    }
    if paths.len() != previous_count {
        Change::stage(
            &mut changes,
            SCHEMA,
            LIST,
            raw,
            format!("['{}']", paths.join("', '")),
        );
    }

    let mut applied = Vec::new();
    let result = (|| {
        for change in &changes {
            change.replace(&change.before, &change.after)?;
            applied.push(change);
        }
        persist()
    })();
    if let Err(error) = result {
        let failures = applied
            .into_iter()
            .rev()
            .filter_map(|change| {
                change
                    .replace(&change.after, &change.before)
                    .err()
                    .map(|error| error.to_string())
            })
            .collect::<Vec<_>>();
        if !failures.is_empty() {
            bail!(
                "{error}. Could not restore GNOME shortcuts: {}",
                failures.join(", ")
            );
        }
        return Err(error);
    }
    Ok(())
}

fn stage_slot(
    changes: &mut Vec<Change>,
    slot: &Slot,
    target: &Requested,
    chord: Option<(u8, String)>,
) -> Result<()> {
    let schema = slot_schema(&slot.path);
    if slot.command != target.command {
        Change::stage(
            changes,
            &schema,
            "name",
            gsettings(&["get", &schema, "name"])?,
            format!("'Clippy {}'", target.label),
        );
        Change::stage(
            changes,
            &schema,
            "command",
            slot.command.clone(),
            target.command.clone(),
        );
    }
    if normalize(&slot.binding) != chord {
        Change::stage(
            changes,
            &schema,
            "binding",
            slot.binding.clone(),
            target.binding.clone(),
        );
    }
    Ok(())
}

fn check_system_shortcuts(targets: &[Requested], triggers: &[Option<(u8, String)>]) -> Result<()> {
    let schemas = gsettings(&["list-schemas"])?;
    for schema in SYSTEM_SCHEMAS
        .into_iter()
        .filter(|schema| schemas.lines().any(|line| line == *schema))
    {
        for line in gsettings(&["list-recursively", schema])?.lines() {
            let mut fields = line.splitn(3, ' ').skip(1);
            let (Some(key), Some(value)) = (fields.next(), fields.next()) else {
                continue;
            };
            for candidate in value.split('\'').skip(1).step_by(2).filter_map(normalize) {
                if let Some(index) = triggers
                    .iter()
                    .position(|trigger| trigger.as_ref() == Some(&candidate))
                {
                    bail!(
                        "{} shortcut is already assigned in GNOME ({key})",
                        targets[index].label
                    );
                }
            }
        }
    }
    Ok(())
}

fn normalize(raw: &str) -> Option<(u8, String)> {
    let mut key = raw.trim_matches('\'');
    let mut modifiers = 0;
    while let Some(prefix) = key.strip_prefix('<') {
        let (modifier, rest) = prefix.split_once('>')?;
        modifiers |= match modifier.to_ascii_lowercase().as_str() {
            "primary" | "control" | "ctrl" => 1,
            "alt" | "mod1" => 2,
            "shift" => 4,
            "super" | "mod4" => 8,
            _ => return None,
        };
        key = rest;
    }
    (!key.is_empty()).then(|| (modifiers, key.to_ascii_lowercase()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gnome_paths_and_accelerator_aliases() {
        assert!(parse_entries("@as []").unwrap().is_empty());
        assert_eq!(
            parse_entries("['/org/user/custom0/']").unwrap(),
            ["/org/user/custom0/"]
        );
        assert!(parse_entries("['invalid']").is_err());
        assert_eq!(
            normalize("'<Shift><Primary>F8'"),
            normalize("<Control><Shift>f8")
        );
        assert_ne!(normalize("<Super>F8"), normalize("<Ctrl>F8"));
        assert!(normalize("<Primary>").is_none());
        assert_eq!(
            trigger("Ctrl+Shift+S".parse().unwrap(), true).unwrap(),
            "<Control><Shift>s"
        );
    }
}
