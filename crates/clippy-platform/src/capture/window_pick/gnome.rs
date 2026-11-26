use super::{DesktopRect, DesktopWindow};
use crate::gnome_extension::{self, ShellWindow};
use anyhow::Result;

pub(super) fn windows() -> Result<Vec<DesktopWindow>> {
    Ok(targets(gnome_extension::windows()?))
}

fn targets(entries: Vec<ShellWindow>) -> Vec<DesktopWindow> {
    entries
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
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_the_shell_extension_payload_in_provider_order() {
        let entries = serde_json::from_str(r#"[
            {"x":-120,"y":250,"width":330,"height":140,"title":"Top","pid":134,"wm_class":"org.example.Window"},
            {"x":20,"y":30,"width":0,"height":-1},
            {"x":-1920,"y":0,"width":1920,"height":1080}
        ]"#).unwrap();
        let items = super::targets(entries);
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
