use crate::identity::{DeploymentIdentity, DistributionChannel};
use anyhow::{Context, Result};

fn name(identity: &DeploymentIdentity) -> &str {
    match identity.channel {
        DistributionChannel::Production => "Clippy",
        DistributionChannel::Qa => "Clippy Slint QA",
    }
}

pub fn enabled(identity: &DeploymentIdentity) -> Result<bool> {
    #[cfg(target_os = "linux")]
    return Ok(desktop_entry(identity)?.try_exists()?);
    #[cfg(not(target_os = "linux"))]
    Ok(entry(identity)?.is_enabled()?)
}

pub fn set_enabled(identity: &DeploymentIdentity, enabled: bool) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use crate::private_files::{replace_private_file, write_private};
        use std::path::PathBuf;
        let destination = desktop_entry(identity)?;
        if enabled {
            let executable = std::env::var_os("APPIMAGE")
                .map(PathBuf::from)
                .map_or_else(std::env::current_exe, Ok)?;
            let executable = executable
                .to_str()
                .context("Executable path must be UTF-8")?;
            if executable.chars().any(char::is_control) {
                anyhow::bail!("Executable path contains a control character");
            }
            let escaped = executable
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('$', "\\$")
                .replace('`', "\\`")
                .replace('%', "%%")
                .replace('\\', "\\\\");
            let content = format!(
                "[Desktop Entry]\nType=Application\nName={}\nExec=\"{escaped}\"\nTerminal=false\n",
                name(identity),
            );
            std::fs::create_dir_all(destination.parent().expect("自启动文件位于 XDG 目录"))?;
            let temp = destination.with_extension("desktop.tmp");
            write_private(&temp, content.as_bytes())?;
            replace_private_file(&temp, &destination)?;
        } else if destination.try_exists()? {
            std::fs::remove_file(destination)?;
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let entry = entry(identity)?;
        if enabled {
            entry.enable()?;
        } else {
            entry.disable()?;
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn desktop_entry(identity: &DeploymentIdentity) -> Result<std::path::PathBuf> {
    Ok(dirs::config_dir()
        .context("The desktop configuration directory is unavailable")?
        .join("autostart")
        .join(format!("{}.desktop", name(identity))))
}

#[cfg(not(target_os = "linux"))]
fn entry(identity: &DeploymentIdentity) -> Result<auto_launch::AutoLaunch> {
    let executable = std::env::current_exe()?;
    let path = executable
        .to_str()
        .context("Executable path must be UTF-8")?;
    let mut builder = auto_launch::AutoLaunchBuilder::new();
    builder.set_app_path(path).set_app_name(name(identity));
    #[cfg(target_os = "macos")]
    builder.set_use_launch_agent(true);
    Ok(builder.build()?)
}
