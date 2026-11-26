use crate::{gnome_settings, private_files};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use zbus::{
    blocking::{connection::Builder, Connection, Proxy},
    zvariant::OwnedValue,
};

const UUID: &str = "clippy-windows@clippy.local";
const VERSION: u32 = 5;
const METADATA: &str =
    include_str!("../../../gnome-extension/clippy-windows@clippy.local/metadata.json");
const SCRIPT: &str =
    include_str!("../../../gnome-extension/clippy-windows@clippy.local/extension.js");
const METADATA_FILE: &str = "metadata.json";
const SCRIPT_FILE: &str = "extension.js";
const TOKEN_FILE: &str = "token";
const SHELL_BUS: &str = "org.gnome.Shell";
const SHELL_PATH: &str = "/org/gnome/Shell";
const EXTENSIONS_INTERFACE: &str = "org.gnome.Shell.Extensions";
const HELPER_PATH: &str = "/org/gnome/Shell/Extensions/ClippyWindows";
const HELPER_INTERFACE: &str = "org.gnome.Shell.Extensions.ClippyWindows";
const SCHEMA: &str = "org.gnome.shell";
const ENABLED: &str = "enabled-extensions";
const DISABLED: &str = "disabled-extensions";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Refresh,
    Install,
    Enable,
    Disable,
    Remove,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    NotInstalled,
    Disabled,
    AwaitingLogin,
    Ready,
    UpdateAvailable,
    NeedsRepair,
    UserExtensionsOff,
    Unavailable,
    ManagedElsewhere,
    UnsupportedShell,
}

pub struct Status {
    pub state: State,
    pub version: Option<u32>,
    pub removable: bool,
}

#[derive(Deserialize)]
struct Manifest {
    uuid: String,
    version: u32,
    #[serde(rename = "shell-version")]
    shell_versions: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Bundle {
    Missing,
    Local { version: u32, complete: bool },
    Foreign,
}

impl Bundle {
    fn version(self) -> Option<u32> {
        match self {
            Self::Local { version, .. } => Some(version),
            _ => None,
        }
    }
}

struct Registration {
    path: PathBuf,
    version: Option<u32>,
}

struct Shell {
    bus: Connection,
    directory: PathBuf,
}

struct Snapshot {
    bundle: Bundle,
    registration: Option<Registration>,
    running: Option<u32>,
    compatible: bool,
    user_extensions: bool,
    enabled: bool,
}

pub fn manage(action: Action) -> Result<Status> {
    ensure!(
        crate::session::is_gnome() && crate::session::is_wayland(),
        "The helper is used on GNOME Wayland"
    );
    let shell = Shell {
        bus: connect(Duration::from_millis(750))?,
        directory: directory()?,
    };
    let current = shell.snapshot()?;
    if action == Action::Refresh {
        return shell.status(current);
    }
    ensure!(
        !shell.protected(&current),
        "This extension is managed outside this copy of Clippy"
    );
    ensure!(
        current.compatible,
        "This GNOME Shell version is outside the bundled helper's supported range"
    );
    let registered = current.registration.is_some();
    match action {
        Action::Install => {
            install_files(&shell.directory, current.bundle)?;
            shell.set_enabled(true, registered)?;
        }
        Action::Enable => {
            ensure!(
                matches!(current.bundle, Bundle::Local { complete: true, .. }),
                "Install the helper before enabling it"
            );
            shell.set_enabled(true, registered)?;
        }
        Action::Disable | Action::Remove => {
            ensure!(
                matches!(current.bundle, Bundle::Local { .. }),
                "The helper is no longer installed"
            );
            shell.set_enabled(false, registered)?;
            ensure!(
                shell.wait_for_running(false)?,
                "GNOME has not finished disabling the helper"
            );
            if action == Action::Remove {
                set_membership(DISABLED, false)?;
                remove_files(&shell.directory)?;
            }
        }
        Action::Refresh => unreachable!(),
    }
    if matches!(action, Action::Install | Action::Enable) && registered && current.user_extensions {
        shell.wait_for_running(true)?;
    }
    shell.status(shell.snapshot()?)
}

impl Shell {
    fn extensions(&self) -> Result<Proxy<'_>> {
        Ok(Proxy::new(
            &self.bus,
            SHELL_BUS,
            SHELL_PATH,
            EXTENSIONS_INTERFACE,
        )?)
    }

    fn snapshot(&self) -> Result<Snapshot> {
        let extensions = self.extensions()?;
        let info: HashMap<String, OwnedValue> = extensions.call("GetExtensionInfo", &(UUID,))?;
        let registration = if info.is_empty() {
            None
        } else {
            let path: &str = info
                .get("path")
                .context("GNOME did not return the extension path")?
                .try_into()?;
            let version = info
                .get("version")
                .map(f64::try_from)
                .transpose()?
                .map(|version| version as u32);
            Some(Registration {
                path: path.into(),
                version,
            })
        };
        let shell_version: String = extensions.get_property("ShellVersion")?;
        let major = shell_version
            .split('.')
            .next()
            .expect("split has at least one entry");
        let bundle = bundle_at(&self.directory)?;
        Ok(Snapshot {
            bundle,
            registration,
            running: self.running_version()?,
            compatible: embedded()
                .shell_versions
                .iter()
                .any(|version| version == major),
            user_extensions: extensions.get_property("UserExtensionsEnabled")?,
            enabled: matches!(bundle, Bundle::Local { .. })
                && listed(ENABLED)?
                && !listed(DISABLED)?,
        })
    }

    fn protected(&self, snapshot: &Snapshot) -> bool {
        snapshot.bundle == Bundle::Foreign
            || installed_version(snapshot).is_some_and(|version| version > VERSION)
            || snapshot
                .registration
                .as_ref()
                .is_some_and(|entry| entry.path != self.directory)
    }

    fn status(&self, snapshot: Snapshot) -> Result<Status> {
        let protected = self.protected(&snapshot);
        let state = if protected {
            State::ManagedElsewhere
        } else if !snapshot.compatible {
            State::UnsupportedShell
        } else {
            match snapshot.bundle {
                Bundle::Missing => State::NotInstalled,
                Bundle::Foreign => unreachable!("外部扩展已单独处理"),
                Bundle::Local { version, .. } if version < VERSION => State::UpdateAvailable,
                Bundle::Local {
                    complete: false, ..
                } => State::NeedsRepair,
                Bundle::Local { .. } if !snapshot.enabled && snapshot.running.is_none() => {
                    State::Disabled
                }
                Bundle::Local { .. } if !snapshot.user_extensions => State::UserExtensionsOff,
                Bundle::Local { version, .. }
                    if snapshot
                        .registration
                        .as_ref()
                        .and_then(|entry| entry.version)
                        != Some(version)
                        || snapshot.running.is_some_and(|running| running != version) =>
                {
                    State::AwaitingLogin
                }
                Bundle::Local { .. } if snapshot.running.is_none() => State::Unavailable,
                Bundle::Local { .. } => {
                    let token = read_token(&self.directory)?
                        .context("The helper's private token is unavailable")?;
                    window_reply(&self.bus, &token)?;
                    State::Ready
                }
            }
        };
        Ok(Status {
            state,
            version: if state == State::NotInstalled {
                None
            } else {
                installed_version(&snapshot)
            },
            removable: !protected
                && snapshot.compatible
                && matches!(snapshot.bundle, Bundle::Local { .. }),
        })
    }

    fn running_version(&self) -> Result<Option<u32>> {
        match self.bus.call_method(
            Some(SHELL_BUS),
            HELPER_PATH,
            Some(HELPER_INTERFACE),
            "GetVersion",
            &(),
        ) {
            Ok(reply) => Ok(Some(reply.body().deserialize()?)),
            Err(zbus::Error::MethodError(name, _, _))
                if matches!(
                    name.as_str(),
                    "org.freedesktop.DBus.Error.UnknownMethod"
                        | "org.freedesktop.DBus.Error.UnknownObject"
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn set_enabled(&self, enabled: bool, registered: bool) -> Result<()> {
        if registered {
            let method = if enabled {
                "EnableExtension"
            } else {
                "DisableExtension"
            };
            let accepted: bool = self.extensions()?.call(method, &(UUID,))?;
            ensure!(accepted, "GNOME rejected the helper's requested state");
        } else {
            // 新目录要等下次登录才会进入 GNOME 的注册表，先保存用户选择。
            if enabled {
                set_membership(DISABLED, false)?;
            }
            set_membership(ENABLED, enabled)?;
        }
        Ok(())
    }

    fn wait_for_running(&self, expected: bool) -> Result<bool> {
        let deadline = Instant::now() + Duration::from_millis(400);
        loop {
            if self.running_version()?.is_some() == expected {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            thread::sleep(Duration::from_millis(40));
        }
    }
}

fn installed_version(snapshot: &Snapshot) -> Option<u32> {
    snapshot.bundle.version().max(snapshot.running).max(
        snapshot
            .registration
            .as_ref()
            .and_then(|entry| entry.version),
    )
}

fn embedded() -> Manifest {
    let manifest: Manifest = serde_json::from_str(METADATA).expect("内嵌的扩展清单经过门禁校验");
    assert_eq!((manifest.uuid.as_str(), manifest.version), (UUID, VERSION));
    manifest
}

fn directory() -> Result<PathBuf> {
    Ok(dirs::data_dir()
        .context("Cannot locate the GNOME extension data directory")?
        .join("gnome-shell/extensions")
        .join(UUID))
}

fn entry(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn bundle_at(path: &Path) -> Result<Bundle> {
    let Some(directory) = entry(path)? else {
        return Ok(Bundle::Missing);
    };
    if !directory.is_dir() {
        return Ok(Bundle::Foreign);
    }
    for name in [METADATA_FILE, SCRIPT_FILE, TOKEN_FILE] {
        if entry(&path.join(name))?.is_some_and(|item| !item.is_file()) {
            return Ok(Bundle::Foreign);
        }
    }
    let Some(metadata) = read_optional(&path.join(METADATA_FILE))? else {
        return Ok(Bundle::Foreign);
    };
    let Ok(manifest) = serde_json::from_slice::<Manifest>(&metadata) else {
        return Ok(Bundle::Foreign);
    };
    if manifest.uuid != UUID {
        return Ok(Bundle::Foreign);
    }
    let complete = metadata == METADATA.as_bytes()
        && read_optional(&path.join(SCRIPT_FILE))?.as_deref() == Some(SCRIPT.as_bytes())
        && read_token(path)?.is_some();
    Ok(Bundle::Local {
        version: manifest.version,
        complete,
    })
}

fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_token(directory: &Path) -> Result<Option<String>> {
    let path = directory.join(TOKEN_FILE);
    if !entry(&path)?.is_some_and(|item| item.is_file()) || !private_files::is_private(&path) {
        return Ok(None);
    }
    let token = read_optional(&path)?.and_then(|bytes| String::from_utf8(bytes).ok());
    Ok(token
        .map(|value| value.trim().to_owned())
        .filter(|value| value.len() >= 16))
}

fn install_files(path: &Path, bundle: Bundle) -> Result<()> {
    match bundle {
        Bundle::Missing => {
            let parent = path.parent().expect("扩展目录位于用户 extensions 目录");
            fs::create_dir_all(parent)?;
            let staging = tempfile::tempdir_in(parent)?;
            write_bundle(staging.path())?;
            fs::rename(staging.path(), path)?;
        }
        Bundle::Local { version, .. } if version <= VERSION => write_bundle(path)?,
        _ => anyhow::bail!("The installed extension must be managed externally"),
    }
    Ok(())
}

fn write_bundle(path: &Path) -> Result<()> {
    if read_token(path)?.is_none() {
        let mut bytes = [0u8; 32];
        fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        let token = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        write_asset(path, TOKEN_FILE, token.as_bytes())?;
    }
    // 最后提交清单，避免 GNOME 在安装过程中读到新版本和旧脚本。
    write_asset(path, SCRIPT_FILE, SCRIPT.as_bytes())?;
    write_asset(path, METADATA_FILE, METADATA.as_bytes())?;
    Ok(())
}

fn write_asset(directory: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let destination = directory.join(name);
    if read_optional(&destination)?.as_deref() != Some(bytes) {
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        file.write_all(bytes)?;
        file.as_file().sync_all()?;
        file.persist(destination)?;
    }
    Ok(())
}

fn remove_files(path: &Path) -> Result<()> {
    for name in [SCRIPT_FILE, TOKEN_FILE, METADATA_FILE] {
        match fs::remove_file(path.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    if fs::read_dir(path)?.next().is_none() {
        fs::remove_dir(path)?;
    }
    Ok(())
}

fn listed(key: &str) -> Result<bool> {
    let raw = gnome_settings::run(&["get", SCHEMA, key])?;
    Ok(parse_entries(&raw)?.into_iter().any(is_our_entry))
}

fn is_our_entry(entry: &str) -> bool {
    &entry[1..entry.len() - 1] == UUID
}

fn set_membership(key: &str, enabled: bool) -> Result<()> {
    let raw = gnome_settings::run(&["get", SCHEMA, key])?;
    let Some(value) = change_entries(&raw, enabled)? else {
        return Ok(());
    };
    gnome_settings::run(&["set", SCHEMA, key, &value])?;
    Ok(())
}

fn change_entries(raw: &str, enabled: bool) -> Result<Option<String>> {
    let mut entries = parse_entries(raw)?;
    if entries.iter().copied().any(is_our_entry) == enabled {
        return Ok(None);
    }
    let entry = format!("'{UUID}'");
    if enabled {
        entries.push(&entry);
    } else {
        entries.retain(|entry| !is_our_entry(entry));
    }
    Ok(Some(format!("@as [{}]", entries.join(", "))))
}

fn parse_entries(value: &str) -> Result<Vec<&str>> {
    let array = value.strip_prefix("@as ").unwrap_or(value);
    let mut remaining = array
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .context("Cannot read GNOME's extension list")?
        .trim();
    let mut entries = Vec::new();
    while !remaining.is_empty() {
        let quote = remaining.chars().next().expect("已排除空条目");
        ensure!(matches!(quote, '\'' | '"'), "Invalid GNOME extension entry");
        let mut escaped = false;
        let end = remaining
            .char_indices()
            .skip(1)
            .find_map(|(index, character)| {
                if std::mem::take(&mut escaped) {
                    return None;
                }
                escaped = character == '\\';
                (character == quote).then_some(index + 1)
            })
            .context("Unterminated GNOME extension entry")?;
        let (entry, tail) = remaining.split_at(end);
        entries.push(entry);
        if tail.trim().is_empty() {
            break;
        }
        remaining = tail
            .trim_start()
            .strip_prefix(',')
            .context("Invalid GNOME extension list")?
            .trim_start();
        ensure!(
            !remaining.is_empty(),
            "Invalid trailing GNOME extension entry"
        );
    }
    Ok(entries)
}

#[derive(Deserialize)]
pub(crate) struct ShellWindow {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    #[serde(default)]
    pub pid: u32,
    #[serde(default)]
    pub title: String,
}

pub(crate) fn windows() -> Result<Vec<ShellWindow>> {
    let Some(token) = read_token(&directory()?)? else {
        return Ok(Vec::new());
    };
    // 窗口列表在冻结图交付后单独读取，Shell 应答慢也不阻塞框选。
    window_reply(&connect(Duration::from_millis(300))?, &token)
}

fn connect(timeout: Duration) -> Result<Connection> {
    Ok(Builder::session()?.method_timeout(timeout).build()?)
}

fn window_reply(connection: &Connection, token: &str) -> Result<Vec<ShellWindow>> {
    let reply = connection.call_method(
        Some(SHELL_BUS),
        HELPER_PATH,
        Some(HELPER_INTERFACE),
        "GetWindows",
        &(token,),
    )?;
    let json: String = reply.body().deserialize()?;
    Ok(serde_json::from_str(&json)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::{fs::symlink, fs::PermissionsExt};

    #[test]
    fn embedded_helper_has_the_expected_protocol_and_private_token() {
        assert!(embedded().shell_versions.contains(&"50".to_owned()));
        assert!(SCRIPT.contains(&format!("const PROTOCOL_VERSION = {VERSION};")));
        assert!(SCRIPT.contains(HELPER_PATH));
        assert!(SCRIPT.contains(&format!("const TOKEN_FILE_NAME = '{TOKEN_FILE}';")));

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(UUID);
        install_files(&path, Bundle::Missing).unwrap();
        assert!(matches!(
            bundle_at(&path).unwrap(),
            Bundle::Local {
                version: VERSION,
                complete: true
            }
        ));
        let token = path.join(TOKEN_FILE);
        assert_eq!(
            fs::metadata(&token).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(read_token(&path).unwrap().unwrap().len(), 64);
    }

    #[test]
    fn upgrade_keeps_a_valid_token_and_removal_keeps_other_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(UUID);
        install_files(&path, Bundle::Missing).unwrap();
        let old_token = fs::read(path.join(TOKEN_FILE)).unwrap();
        let mut manifest = serde_json::from_str::<serde_json::Value>(METADATA).unwrap();
        manifest["version"] = (VERSION - 1).into();
        fs::write(
            path.join(METADATA_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(path.join("local-notes.txt"), b"personal notes").unwrap();

        install_files(&path, bundle_at(&path).unwrap()).unwrap();
        assert!(fs::read(path.join(TOKEN_FILE)).unwrap() == old_token);
        remove_files(&path).unwrap();
        assert!(!path.join(TOKEN_FILE).exists());
        assert_eq!(
            fs::read(path.join("local-notes.txt")).unwrap(),
            b"personal notes"
        );
    }

    #[test]
    fn later_versions_and_linked_files_cannot_be_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(UUID);
        fs::create_dir(&path).unwrap();
        let mut manifest = serde_json::from_str::<serde_json::Value>(METADATA).unwrap();
        manifest["version"] = (VERSION + 1).into();
        fs::write(
            path.join(METADATA_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(install_files(&path, bundle_at(&path).unwrap()).is_err());
        assert_eq!(bundle_at(&path).unwrap().version(), Some(VERSION + 1));

        let other = temp.path().join("other.js");
        fs::write(&other, b"owned by someone else").unwrap();
        fs::write(path.join(METADATA_FILE), METADATA).unwrap();
        symlink(&other, path.join(SCRIPT_FILE)).unwrap();
        assert!(install_files(&path, bundle_at(&path).unwrap()).is_err());
        assert_eq!(fs::read(&other).unwrap(), b"owned by someone else");
    }

    #[test]
    fn extension_lists_preserve_other_entries_with_commas_and_escapes() {
        let raw = r#"['other@local', "another'@local", 'comma,inside@local', 'slash\\name@local']"#;
        let enabled = change_entries(raw, true).unwrap().unwrap();
        assert!(parse_entries(&enabled)
            .unwrap()
            .into_iter()
            .any(is_our_entry));
        assert!(change_entries(&enabled, true).unwrap().is_none());
        let disabled = change_entries(&enabled, false).unwrap().unwrap();
        assert_eq!(
            parse_entries(&disabled).unwrap(),
            parse_entries(raw).unwrap()
        );
        assert!(change_entries("@as []", false).unwrap().is_none());
        assert!(change_entries("['other@local', invalid]", true).is_err());
    }
}
