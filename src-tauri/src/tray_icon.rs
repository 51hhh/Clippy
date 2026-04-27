//! 托盘图标：构建时预渲染各主题 RGBA，运行时零开销加载。

use tauri::image::Image;

const SIZE: u32 = 64;

const THEME_ICONS: &[(&str, &[u8])] = &[
    (
        "light",
        include_bytes!(concat!(env!("OUT_DIR"), "/tray-light.rgba")),
    ),
    (
        "dark",
        include_bytes!(concat!(env!("OUT_DIR"), "/tray-dark.rgba")),
    ),
    (
        "nord",
        include_bytes!(concat!(env!("OUT_DIR"), "/tray-nord.rgba")),
    ),
    (
        "solarized-light",
        include_bytes!(concat!(env!("OUT_DIR"), "/tray-solarized-light.rgba")),
    ),
    (
        "rose",
        include_bytes!(concat!(env!("OUT_DIR"), "/tray-rose.rgba")),
    ),
    (
        "midnight",
        include_bytes!(concat!(env!("OUT_DIR"), "/tray-midnight.rgba")),
    ),
];

/// 返回对应主题的托盘图标；未知主题回退到 light。
pub fn render_themed_tray_icon(theme: &str) -> Option<Image<'static>> {
    let rgba = THEME_ICONS
        .iter()
        .find(|(name, _)| *name == theme)
        .or_else(|| THEME_ICONS.iter().find(|(name, _)| *name == "light"))
        .map(|(_, data)| *data)?;
    Image::new_owned(rgba.to_vec(), SIZE, SIZE).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_returns_64x64() {
        let img = render_themed_tray_icon("light").expect("应返回图像");
        assert_eq!(img.width(), 64);
        assert_eq!(img.height(), 64);
        assert!(img.rgba().iter().any(|b| *b != 0));
    }

    #[test]
    fn render_handles_all_known_themes() {
        for (name, _) in THEME_ICONS {
            assert!(render_themed_tray_icon(name).is_some(), "{name} 渲染失败");
        }
    }

    #[test]
    fn unknown_theme_falls_back_to_light() {
        assert!(render_themed_tray_icon("nonexistent").is_some());
    }
}
