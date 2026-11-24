use super::{DesktopRect, FrozenOutput};
use anyhow::{ensure, Context, Result};
use x11rb::{
    connection::Connection,
    image::{Image, PixelLayout},
    protocol::{
        randr::ConnectionExt as _,
        xproto::{ConnectionExt as _, Window},
    },
    rust_connection::RustConnection,
};

pub(super) struct Source {
    connection: RustConnection,
    screen: usize,
    root: Window,
    layout: PixelLayout,
}

impl Source {
    pub(super) fn open() -> Result<Self> {
        let (connection, screen) = x11rb::connect(None)?;
        let root = &connection.setup().roots[screen];
        let visual = root
            .allowed_depths
            .iter()
            .flat_map(|depth| &depth.visuals)
            .find(|visual| visual.visual_id == root.root_visual)
            .context("Missing X11 root visual")?;
        let layout = PixelLayout::from_visual_type(*visual)?;
        let root = root.root;
        Ok(Self {
            connection,
            screen,
            root,
            layout,
        })
    }

    pub(super) fn read(&self, geometry: DesktopRect) -> Result<image::RgbaImage> {
        let width = u16::try_from(geometry.width)?;
        let height = u16::try_from(geometry.height)?;
        let bytes = usize::from(width) * usize::from(height) * 4;
        clippy_core::clipboard::validate_image_layout(
            usize::from(width),
            usize::from(height),
            bytes,
        )
        .map_err(anyhow::Error::msg)?;
        let (image, _) = Image::get(
            &self.connection,
            self.root,
            geometry.x.try_into()?,
            geometry.y.try_into()?,
            width,
            height,
        )?;
        let mut rgba = Vec::with_capacity(bytes);
        for y in 0..height {
            for x in 0..width {
                let (red, green, blue) = self.layout.decode(image.get_pixel(x, y));
                rgba.extend_from_slice(&[
                    (red >> 8) as u8,
                    (green >> 8) as u8,
                    (blue >> 8) as u8,
                    255,
                ]);
            }
        }
        Ok(
            image::RgbaImage::from_raw(geometry.width, geometry.height, rgba)
                .expect("按行输出 RGBA 像素"),
        )
    }
}

pub(super) fn freeze() -> Result<Vec<FrozenOutput>> {
    let source = Source::open()?;
    let connection = &source.connection;
    let root = &connection.setup().roots[source.screen];
    let monitors = connection
        .randr_get_monitors(root.root, true)?
        .reply()?
        .monitors;

    monitors
        .into_iter()
        .map(|monitor| {
            let width = monitor.width;
            let height = monitor.height;
            ensure!(width > 0 && height > 0, "Empty X11 monitor");
            let name =
                String::from_utf8_lossy(&connection.get_atom_name(monitor.name)?.reply()?.name)
                    .into_owned();
            let (width, height) = (u32::from(width), u32::from(height));
            let geometry = DesktopRect {
                x: i32::from(monitor.x),
                y: i32::from(monitor.y),
                width,
                height,
            };
            Ok(FrozenOutput {
                name,
                geometry,
                pixels: source.read(geometry)?.into(),
            })
        })
        .collect()
}
