use super::{DesktopRect, FrozenOutput, FrozenPixels};
use anyhow::{ensure, Context, Result};
use x11rb::{
    connection::Connection,
    image::{Image, PixelLayout},
    protocol::{randr::ConnectionExt as _, xproto::ConnectionExt as _},
};

pub(super) fn freeze() -> Result<Vec<FrozenOutput>> {
    let (connection, screen) = x11rb::connect(None)?;
    let root = &connection.setup().roots[screen];
    let monitors = connection
        .randr_get_monitors(root.root, true)?
        .reply()?
        .monitors;
    let visual = root
        .allowed_depths
        .iter()
        .flat_map(|depth| &depth.visuals)
        .find(|visual| visual.visual_id == root.root_visual)
        .context("Missing X11 root visual")?;
    let layout = PixelLayout::from_visual_type(*visual)?;

    monitors
        .into_iter()
        .map(|monitor| {
            let width = monitor.width;
            let height = monitor.height;
            ensure!(width > 0 && height > 0, "Empty X11 monitor");
            let (image, _) =
                Image::get(&connection, root.root, monitor.x, monitor.y, width, height)?;
            let mut rgba = Vec::with_capacity(usize::from(width) * usize::from(height) * 4);
            for y in 0..height {
                for x in 0..width {
                    let (red, green, blue) = layout.decode(image.get_pixel(x, y));
                    rgba.extend_from_slice(&[
                        (red >> 8) as u8,
                        (green >> 8) as u8,
                        (blue >> 8) as u8,
                        255,
                    ]);
                }
            }
            let name =
                String::from_utf8_lossy(&connection.get_atom_name(monitor.name)?.reply()?.name)
                    .into_owned();
            let (width, height) = (u32::from(width), u32::from(height));
            Ok(FrozenOutput {
                name,
                geometry: DesktopRect {
                    x: i32::from(monitor.x),
                    y: i32::from(monitor.y),
                    width,
                    height,
                },
                pixels: FrozenPixels {
                    width,
                    height,
                    rgba: rgba.into(),
                },
            })
        })
        .collect()
}
