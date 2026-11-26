use super::{DesktopRect, DesktopWindow};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::{fs, time::Duration};

#[derive(Deserialize)]
struct ShellWindow {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    #[serde(default)]
    pid: u32,
    #[serde(default)]
    title: String,
}

pub(super) fn windows() -> Result<Vec<DesktopWindow>> {
    let token_path = dirs::data_dir()
        .context("Cannot locate the GNOME extension data directory")?
        .join("gnome-shell/extensions/clippy-windows@clippy.local/token");
    if !token_path.try_exists()? {
        return Ok(Vec::new());
    }
    ensure!(
        crate::private_files::is_private(&token_path),
        "GNOME window token is not private"
    );
    let token = fs::read_to_string(token_path)?;
    let token = token.trim();
    ensure!(token.len() >= 16, "GNOME window token is invalid");

    // 窗口列表是冻结之后独立请求的元数据，超时也不拖住框选或像素采集。
    let connection = zbus::blocking::connection::Builder::session()?
        .method_timeout(Duration::from_millis(300))
        .build()?;
    let reply = connection.call_method(
        Some("org.gnome.Shell"),
        "/org/gnome/Shell/Extensions/ClippyWindows",
        Some("org.gnome.Shell.Extensions.ClippyWindows"),
        "GetWindows",
        &(token,),
    )?;
    let json: String = reply.body().deserialize()?;
    parse(&json)
}

fn parse(json: &str) -> Result<Vec<DesktopWindow>> {
    let entries: Vec<ShellWindow> = serde_json::from_str(json)?;
    Ok(entries
        .into_iter()
        .filter_map(|window| {
            Some(DesktopWindow {
                geometry: DesktopRect {
                    x: window.x,
                    y: window.y,
                    width: window.width.try_into().ok()?,
                    height: window.height.try_into().ok()?,
                },
                pid: window.pid,
                title: window.title,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_the_shell_extension_payload_in_provider_order() {
        let items = super::parse(r#"[
            {"x":-120,"y":250,"width":330,"height":140,"title":"Top","pid":134,"wm_class":"org.example.Window"},
            {"x":20,"y":30,"width":0,"height":-1},
            {"x":-1920,"y":0,"width":1920,"height":1080}
        ]"#).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(
            (items[0].geometry.x, items[0].pid, items[0].title.as_str()),
            (-120, 134, "Top")
        );
        assert_eq!(
            (items[1].geometry.x, items[1].geometry.width),
            (-1920, 1920)
        );
    }
}
