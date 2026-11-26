use anyhow::{bail, Result};
use std::process::Command;

pub(crate) fn run(args: &[&str]) -> Result<String> {
    let result = Command::new("gsettings").args(args).output()?;
    if !result.status.success() {
        bail!("{}", String::from_utf8_lossy(&result.stderr).trim());
    }
    Ok(String::from_utf8(result.stdout)?.trim().to_owned())
}
