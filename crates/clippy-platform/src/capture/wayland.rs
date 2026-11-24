use super::{gnome, DesktopRect, FrozenOutput, FrozenPixels};
use anyhow::{bail, Context, Result};
use libwayshot_xcap::{output::OutputInfo, WayshotConnection};

pub(super) fn freeze() -> Result<Vec<FrozenOutput>> {
    let connection = WayshotConnection::new().context("Cannot read Wayland outputs")?;
    let outputs = connection.get_all_outputs();
    let areas = outputs
        .iter()
        .map(|output| {
            let logical = output.logical_region.inner;
            (
                output.name.clone(),
                DesktopRect {
                    x: logical.position.x,
                    y: logical.position.y,
                    width: logical.size.width,
                    height: logical.size.height,
                },
            )
        })
        .collect::<Vec<_>>();
    let gnome = if crate::session::is_gnome() {
        Some(gnome::capture_areas(&areas))
    } else {
        None
    };
    let mut frames = match gnome {
        Some(Ok(frames)) => Some(frames.into_iter()),
        Some(Err(error)) => return Err(error.context("Mutter did not deliver a frozen frame")),
        None => None,
    };
    areas
        .into_iter()
        .zip(outputs)
        .map(|((name, geometry), output)| {
            let pixels = match frames.as_mut() {
                Some(frames) => frames.next().expect("RecordArea preserves output order")?,
                None => native_frame(&connection, output)?,
            };
            if pixels.width < geometry.width || pixels.height < geometry.height {
                bail!("Captured output {name} is smaller than its logical viewport");
            }
            Ok(FrozenOutput {
                name,
                geometry,
                pixels,
            })
        })
        .collect()
}

fn native_frame(connection: &WayshotConnection, output: &OutputInfo) -> Result<FrozenPixels> {
    let image = connection
        .screenshot_single_output(output, false)
        .context("This Wayland compositor did not provide output capture")?;
    Ok(image.into_rgba8().into())
}

pub(super) fn with_output<T>(
    name: &str,
    use_stream: impl FnOnce(&mut dyn FnMut() -> Result<FrozenPixels>) -> Result<T>,
) -> Result<T> {
    let connection = WayshotConnection::new().context("Cannot read Wayland outputs")?;
    let output = connection
        .get_all_outputs()
        .iter()
        .find(|output| output.name == name)
        .context("Capture output is unavailable")?;
    use_stream(&mut || native_frame(&connection, output))
}
