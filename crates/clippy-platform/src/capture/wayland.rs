use super::{
    gnome, pipewire_frame::FrameRegion, pixel_rect, screencast, DesktopRect, FrozenOutput,
    FrozenPixels, Reader, RegionCancellation,
};
use anyhow::{bail, ensure, Context, Result};
use libwayshot_xcap::{output::OutputInfo, WayshotConnection};
use wayland_client::protocol::wl_output::Transform;

pub(super) fn freeze() -> Result<Vec<FrozenOutput>> {
    let connection = WayshotConnection::new().context("Cannot read Wayland outputs")?;
    let outputs = connection.get_all_outputs();
    let areas = outputs
        .iter()
        .map(|output| (output.name.clone(), logical_geometry(output)))
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

fn logical_geometry(output: &OutputInfo) -> DesktopRect {
    let logical = output.logical_region.inner;
    DesktopRect {
        x: logical.position.x,
        y: logical.position.y,
        width: logical.size.width,
        height: logical.size.height,
    }
}

fn selected_output<'a>(
    connection: &'a WayshotConnection,
    name: &str,
    expected: DesktopRect,
) -> Result<&'a OutputInfo> {
    let output = connection
        .get_all_outputs()
        .iter()
        .find(|output| output.name == name)
        .context("Capture output is unavailable")?;
    ensure!(
        logical_geometry(output) == expected,
        "Capture output changed geometry"
    );
    Ok(output)
}

pub(super) fn with_region<T>(
    name: &str,
    geometry: DesktopRect,
    source_size: (u32, u32),
    bounds: [f32; 4],
    cancellation: &RegionCancellation,
    consume: impl FnOnce(&mut Reader<'_>) -> Result<T>,
) -> Result<T> {
    let pixels = pixel_rect(source_size.0, source_size.1, bounds)?;
    let connection = WayshotConnection::new().context("Cannot read Wayland outputs")?;
    let output = selected_output(&connection, name, geometry)?;
    // wlr 流可能把旋转写进视频元数据；Wayshot 已对冻结图应用同一变换。
    if output.transform == Transform::Normal {
        if let Some(mut portal) = screencast::ScreenCast::connect(cancellation)? {
            let stream = portal.authorize(cancellation)?;
            let current = WayshotConnection::new().context("Cannot recheck Wayland outputs")?;
            let live = selected_output(&current, name, geometry)?;
            ensure!(
                live.transform == output.transform && live.physical_size == output.physical_size,
                "Capture output changed display mode"
            );
            let outputs = current.get_all_outputs();
            let matches = outputs
                .iter()
                .filter(|output| logical_geometry(output) == geometry)
                .count();
            screencast::validate_monitor(&stream, geometry, outputs.len(), matches)?;
            drop(current);
            return portal.with_region(
                stream.pipe_wire_node_id(),
                FrameRegion {
                    source_size,
                    bounds: pixels,
                },
                cancellation,
                consume,
            );
        }
    }
    cancellation.check_active()?;
    consume(&mut || {
        let frame = native_frame(&connection, output)?;
        ensure!(
            (frame.width, frame.height) == source_size,
            "Capture output changed size"
        );
        let [left, top, right, bottom] = bounds;
        frame.crop(left, top, right, bottom)
    })
}
