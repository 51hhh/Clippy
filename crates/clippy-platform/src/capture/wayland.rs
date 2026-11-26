use super::{
    gnome, pixel_rect, screencast,
    screencast_stream::{ScreenCastStream, StreamMode},
    validate_outputs,
    wayland_outputs::Registry,
    CaptureSource, DesktopRect, FrozenCapture, FrozenOutput, FrozenPixels, Reader,
    RegionCancellation, RegionFrame,
};
use anyhow::{bail, ensure, Context, Result};
use libwayshot_xcap::{output::OutputInfo, WayshotConnection};
use std::{sync::mpsc, time::Instant};
use wayland_client::protocol::wl_output::Transform;

pub(super) fn freeze() -> Result<Vec<FrozenOutput>> {
    let connection = if crate::session::is_gnome() {
        WayshotConnection::new().context("Cannot read Wayland outputs")?
    } else {
        let registry = Registry::connect()?;
        ensure!(
            supports_direct_capture(&registry),
            "This Wayland desktop requires choosing a screen for capture"
        );
        WayshotConnection::from_connection(registry.connection)
            .context("Cannot read Wayland outputs")?
    };
    freeze_outputs(&connection)
}

fn supports_direct_capture(registry: &Registry) -> bool {
    registry.has("zwlr_screencopy_manager_v1", 3) && registry.has("zxdg_output_manager_v1", 3)
}

fn freeze_outputs(connection: &WayshotConnection) -> Result<Vec<FrozenOutput>> {
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
                None => native_frame(connection, output)?,
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

pub(super) fn freeze_selection(
    cancellation: &RegionCancellation,
    publish: impl FnOnce(Result<FrozenCapture>),
) {
    let result = prepare_selection(cancellation);
    let (mut frozen, portal) = match result {
        Ok(selection) => selection,
        Err(error) => return publish(Err(error)),
    };
    if let Some(portal) = portal {
        let (claim, requested) = mpsc::sync_channel(1);
        frozen.source = CaptureSource(Some(PortalHandoff {
            claim,
            fresh_from: None,
        }));
        publish(Ok(frozen));
        // 回调只交付图片和一次性句柄，D-Bus 与 PipeWire 的关闭不会阻塞界面。
        if let Ok(receiver) = requested.recv() {
            let _ = receiver.send(portal);
        }
    } else {
        publish(Ok(frozen));
    }
}

fn prepare_selection(
    cancellation: &RegionCancellation,
) -> Result<(FrozenCapture, Option<PortalCapture>)> {
    if crate::session::is_gnome() {
        let outputs = freeze()?;
        validate_outputs(&outputs)?;
        return Ok((FrozenCapture::direct(outputs), None));
    }

    let registry = Registry::connect()?;
    if supports_direct_capture(&registry) {
        let connection = WayshotConnection::from_connection(registry.connection)
            .context("Cannot read Wayland outputs")?;
        let outputs = freeze_outputs(&connection)?;
        validate_outputs(&outputs)?;
        return Ok((FrozenCapture::direct(outputs), None));
    }
    drop(registry);

    let mut portal = screencast::ScreenCast::connect(cancellation)?
        .context("This Wayland desktop has no monitor capture provider")?;
    let granted = portal.authorize(cancellation)?;
    let authorized_at = super::screencast_stream::monotonic_ns()?;
    let outputs = Registry::connect()?.displays()?;
    let index = screencast::monitor_index(&granted, outputs.iter().map(|output| output.geometry))?;
    let output = &outputs[index];
    let geometry = output.geometry;
    let name = output.name.clone();
    let size = output.size;
    ensure!(
        output.transform == Transform::Normal,
        "ScreenCast capture on a rotated display is unavailable"
    );
    ensure!(
        size.0 >= geometry.width && size.1 >= geometry.height,
        "ScreenCast output is smaller than its logical viewport"
    );
    let output_count = outputs.len();
    let (stream, pixels) = portal.open_stream(
        granted.pipe_wire_node_id(),
        size,
        StreamMode::Freeze(authorized_at),
        cancellation,
    )?;
    let outputs = vec![FrozenOutput {
        name,
        geometry,
        pixels,
    }];
    validate_outputs(&outputs)?;
    Ok((
        FrozenCapture {
            outputs,
            output_count,
            source: CaptureSource::default(),
        },
        Some(PortalCapture { stream, portal }),
    ))
}

struct PortalCapture {
    stream: ScreenCastStream,
    portal: screencast::ScreenCast,
}

pub(super) struct PortalHandoff {
    claim: mpsc::SyncSender<mpsc::SyncSender<PortalCapture>>,
    fresh_from: Option<i64>,
}

impl PortalHandoff {
    pub fn overlay_closed(&mut self, closed: std::time::Instant) -> Result<()> {
        self.fresh_from = Some(super::screencast_stream::monotonic_ns_at(closed)?);
        Ok(())
    }

    pub fn with_region<T>(
        self,
        name: &str,
        geometry: DesktopRect,
        source_size: (u32, u32),
        bounds: [f32; 4],
        cancellation: &RegionCancellation,
        consume: impl FnOnce(&mut Reader<'_>) -> Result<T>,
    ) -> Result<T> {
        cancellation.check_active()?;
        let pixels = pixel_rect(source_size.0, source_size.1, bounds)?;
        let outputs = Registry::connect()?.displays()?;
        let current = outputs
            .iter()
            .find(|output| output.name == name)
            .context("Capture output is unavailable")?;
        ensure!(
            current.geometry == geometry
                && current.transform == Transform::Normal
                && current.size == source_size,
            "Capture output changed display mode"
        );
        let (reply, authorized) = mpsc::sync_channel(1);
        self.claim
            .send(reply)
            .context("The shared screen is no longer available")?;
        let mut capture = authorized.recv()?;
        capture
            .portal
            .resume_region(&capture.stream, pixels, self.fresh_from, cancellation)?;
        consume(&mut |demand| capture.stream.read_region(demand))
    }
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
    let registry = Registry::connect()?;
    let direct = supports_direct_capture(&registry);
    let outputs = registry.displays()?;
    let output = outputs
        .iter()
        .find(|output| output.name == name && output.geometry == geometry)
        .context("Capture output changed geometry")?;
    // wlr 流可能把旋转写进视频元数据；Wayshot 已对冻结图应用同一变换。
    if output.transform == Transform::Normal {
        if let Some(mut portal) = screencast::ScreenCast::connect(cancellation)? {
            let stream = portal.authorize(cancellation)?;
            let current = Registry::connect()?.displays()?;
            let index =
                screencast::monitor_index(&stream, current.iter().map(|output| output.geometry))?;
            let live = &current[index];
            ensure!(
                live.name == name,
                "Share the same display that was used for the screenshot"
            );
            ensure!(
                live.geometry == geometry
                    && live.transform == output.transform
                    && live.size == output.size,
                "Capture output changed display mode"
            );
            let (mut stream, _) = portal.open_stream(
                stream.pipe_wire_node_id(),
                source_size,
                StreamMode::Region(pixels),
                cancellation,
            )?;
            return consume(&mut |demand| stream.read_region(demand));
        }
    }
    cancellation.check_active()?;
    ensure!(direct, "This desktop requires a screen sharing session");
    let connection = WayshotConnection::new().context("Cannot read Wayland outputs")?;
    let output = selected_output(&connection, name, geometry)?;
    consume(&mut |_| {
        let frame = native_frame(&connection, output)?;
        let observed = Instant::now();
        ensure!(
            (frame.width, frame.height) == source_size,
            "Capture output changed size"
        );
        let [left, top, right, bottom] = bounds;
        let pixels = frame.crop(left, top, right, bottom)?;
        Ok(Some(RegionFrame::observed(pixels, observed)))
    })
}
