use super::{
    pixel_rect,
    screencast::{
        cancellable, monitor_index, share_error, unavailable, wait_for_frame, CALL_TIMEOUT,
        CHOOSER_TIMEOUT, CLOSE_TIMEOUT,
    },
    screencast_stream::{monotonic_ns, monotonic_ns_at, ScreenCastStream, StreamMode},
    wayland_outputs::Registry,
    DesktopRect, Region, RegionCancellation, ScreenSharingCancelled,
};
use crate::scroll::Direction;
use anyhow::{anyhow, ensure, Context, Result};
use ashpd::{
    desktop::{
        remote_desktop::{Axis, DeviceType, RemoteDesktop, SelectDevicesOptions},
        screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType, Stream},
        PersistMode, Session,
    },
    WindowIdentifier,
};
use futures_util::StreamExt;
use image::RgbaImage;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    runtime::Runtime,
    sync::{oneshot, watch},
};
use wayland_client::protocol::wl_output::Transform;

const INPUT_TIMEOUT: Duration = Duration::from_millis(200);
const FRESH_FRAME_TIMEOUT: Duration = Duration::from_millis(500);

struct Portals {
    runtime: Runtime,
    connection: zbus::Connection,
    remote: RemoteDesktop,
    cast: Screencast,
    session: Option<Arc<Session<RemoteDesktop>>>,
}

impl Portals {
    fn connect(cancellation: &RegionCancellation) -> Result<Option<Self>> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let result = runtime.block_on(cancellable(cancellation, CALL_TIMEOUT, async {
            let connection = zbus::Connection::session().await?;
            let remote = RemoteDesktop::with_connection(connection.clone()).await?;
            let cast = Screencast::with_connection(connection.clone()).await?;
            let supported = remote
                .available_device_types()
                .await?
                .contains(DeviceType::Pointer)
                && cast
                    .available_source_types()
                    .await?
                    .contains(SourceType::Monitor);
            Ok(supported.then_some((connection, remote, cast)))
        }));
        match result {
            Ok(portals) => Ok(portals.map(|(connection, remote, cast)| Self {
                runtime,
                connection,
                remote,
                cast,
                session: None,
            })),
            Err(error) if error.downcast_ref().is_some_and(unavailable) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn select(
        &mut self,
        cancellation: &RegionCancellation,
        deadline: Instant,
    ) -> Result<watch::Receiver<bool>> {
        let (session, closed, devices, sources) =
            self.runtime
                .block_on(cancellable(cancellation, CALL_TIMEOUT, async {
                    let session = Arc::new(self.remote.create_session(Default::default()).await?);
                    let closed = watch_closed(session.clone()).await?;
                    let mut devices = SelectDevicesOptions::default()
                        .set_devices(Some(DeviceType::Pointer.into()));
                    if self.remote.version() >= 2 {
                        devices = devices.set_persist_mode(PersistMode::DoNot);
                    }
                    let mut sources = SelectSourcesOptions::default()
                        .set_sources(Some(SourceType::Monitor.into()))
                        .set_multiple(false);
                    if self.cast.version() >= 2 {
                        ensure!(
                            self.cast
                                .available_cursor_modes()
                                .await?
                                .contains(CursorMode::Hidden),
                            "Screen sharing cannot hide the cursor"
                        );
                        sources = sources.set_cursor_mode(CursorMode::Hidden);
                    }
                    if self.cast.version() >= 4 {
                        sources = sources.set_persist_mode(PersistMode::DoNot);
                    }
                    Ok((session, closed, devices, sources))
                }))?;
        self.session = Some(session);
        self.runtime
            .block_on(cancellable(cancellation, time_left(deadline)?, async {
                self.remote
                    .select_devices(self.session(), devices)
                    .await?
                    .response()?;
                self.cast
                    .select_sources(self.session(), sources)
                    .await?
                    .response()?;
                Ok(closed)
            }))
            .map_err(share_error)
    }

    fn start(
        &self,
        parent: &WindowIdentifier,
        cancellation: &RegionCancellation,
        deadline: Instant,
    ) -> Result<Stream> {
        self.runtime
            .block_on(cancellable(cancellation, time_left(deadline)?, async {
                let granted = self
                    .remote
                    .start(self.session(), Some(parent), Default::default())
                    .await?
                    .response()?;
                ensure!(
                    granted.devices().contains(DeviceType::Pointer),
                    "The desktop did not grant pointer control"
                );
                let [stream] = granted.streams() else {
                    anyhow::bail!("Screen sharing must return one monitor");
                };
                Ok(stream.clone())
            }))
            .map_err(share_error)
    }

    fn session(&self) -> &Session<RemoteDesktop> {
        self.session.as_deref().expect("已创建 Portal 会话")
    }
}

impl Drop for Portals {
    fn drop(&mut self) {
        self.runtime.block_on(async {
            if let Some(session) = self.session.take() {
                let _ = tokio::time::timeout(CLOSE_TIMEOUT, session.close()).await;
            }
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, self.connection.clone().close()).await;
        });
    }
}

async fn watch_closed(session: Arc<Session<RemoteDesktop>>) -> Result<watch::Receiver<bool>> {
    let (listening, ready) = oneshot::channel();
    let (closed, state) = watch::channel(false);
    tokio::spawn(async move {
        let mut signals = match session.receive_closed().await {
            Ok(signals) => {
                let _ = listening.send(Ok(()));
                signals
            }
            Err(error) => {
                let _ = listening.send(Err(error));
                return;
            }
        };
        let _ = signals.next().await;
        closed.send_replace(true);
    });
    ready.await??;
    Ok(state)
}

fn time_left(deadline: Instant) -> Result<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    ensure!(
        !remaining.is_zero(),
        "Screen sharing authorization timed out"
    );
    Ok(remaining)
}

pub struct AuthorizedScroll {
    stream: ScreenCastStream,
    portals: Portals,
    closed: watch::Receiver<bool>,
    cancellation: RegionCancellation,
    node: u32,
    pixels: [u32; 4],
    anchor: (f64, f64),
}

impl AuthorizedScroll {
    pub fn available(cancellation: &RegionCancellation) -> Result<bool> {
        Ok(Portals::connect(cancellation)?.is_some())
    }

    pub fn authorize(
        region: &Region,
        window: &(impl HasWindowHandle + HasDisplayHandle),
        cancellation: &RegionCancellation,
        release_previous: impl FnOnce(),
    ) -> Result<Self> {
        let mut portals =
            Portals::connect(cancellation)?.context("This desktop cannot authorize scrolling")?;
        let raw_window = window.window_handle()?.as_raw();
        let raw_display = window.display_handle()?.as_raw();
        ensure!(
            matches!(raw_window, RawWindowHandle::Wayland(_))
                && matches!(raw_display, RawDisplayHandle::Wayland(_)),
            "Scrolling authorization requires a native Wayland window"
        );
        // 调用方保持 Winit 窗口的所有权，直到 xdg-foreign parent 完成注销。
        let parent = portals
            .runtime
            .block_on(WindowIdentifier::from_raw_handle(
                &raw_window,
                Some(&raw_display),
            ))
            .filter(|parent| parent.to_string() != "wayland:")
            .context("Could not export the scrolling controls for the system dialog")?;
        let deadline = Instant::now() + CHOOSER_TIMEOUT;
        let closed = portals.select(cancellation, deadline)?;
        cancellation.check_active()?;
        release_previous();
        let granted = portals.start(&parent, cancellation, deadline)?;
        drop(parent);
        let authorized_at = monotonic_ns()?;
        validate_monitor(&granted, region)?;
        let pixels = pixel_rect(region.source_size.0, region.source_size.1, region.bounds)?;
        let node = granted.pipe_wire_node_id();
        let remote = portals
            .runtime
            .block_on(cancellable(cancellation, CALL_TIMEOUT, async {
                Ok(portals
                    .cast
                    .open_pipe_wire_remote(portals.session(), Default::default())
                    .await?)
            }))?;
        let (stream, first) = ScreenCastStream::start(
            remote,
            node,
            region.source_size,
            StreamMode::RegionAfter(pixels, authorized_at),
        )?;
        wait_for_frame(&portals.runtime, first, cancellation)?;
        let active = Self {
            stream,
            portals,
            closed,
            cancellation: cancellation.clone(),
            node,
            pixels,
            anchor: pointer_anchor(region.geometry, region.source_size, pixels),
        };
        active.check()?;
        Ok(active)
    }

    pub fn while_active<T>(&self, operation: impl Future<Output = T>) -> Result<T> {
        let mut closed = self.closed.clone();
        self.portals.runtime.block_on(async {
            tokio::select! {
                biased;
                _ = self.cancellation.cancelled() => Err(ScreenSharingCancelled.into()),
                _ = closed.wait_for(|closed| *closed) => Err(anyhow!("Scrolling permission ended")),
                result = operation => Ok(result),
            }
        })
    }

    pub fn refresh_after(&mut self, hidden: Instant) -> Result<()> {
        let frame = self
            .stream
            .resume(self.pixels, Some(monotonic_ns_at(hidden)?))?;
        self.while_active(cancellable(
            &self.cancellation,
            FRESH_FRAME_TIMEOUT,
            async {
                frame
                    .await
                    .context("Screen sharing closed before a fresh frame")?
            },
        ))??;
        self.check()
    }

    pub fn check(&self) -> Result<()> {
        self.cancellation.check_active()?;
        ensure!(!*self.closed.borrow(), "Scrolling permission ended");
        self.stream.check_active()
    }

    pub fn read(&mut self) -> Result<RgbaImage> {
        self.check()?;
        self.stream.read()
    }

    pub fn matches_view(&mut self, expected: &RgbaImage) -> Result<bool> {
        self.check()?;
        self.stream.matches_view(expected)
    }

    pub fn position_pointer(&self) -> Result<()> {
        self.check()?;
        let (x, y) = self.anchor;
        self.input(self.portals.remote.notify_pointer_motion_absolute(
            self.portals.session(),
            self.node,
            x,
            y,
            Default::default(),
        ))
    }

    pub fn step(&self, direction: Direction) -> Result<()> {
        self.check()?;
        let (axis, steps) = scroll_input(direction);
        self.input(self.portals.remote.notify_pointer_axis_discrete(
            self.portals.session(),
            axis,
            steps,
            Default::default(),
        ))
    }

    fn input(&self, operation: impl Future<Output = Result<(), ashpd::Error>>) -> Result<()> {
        self.while_active(cancellable(&self.cancellation, INPUT_TIMEOUT, async {
            Ok(operation.await?)
        }))?
    }
}

fn validate_monitor(stream: &Stream, region: &Region) -> Result<()> {
    let displays = Registry::connect()?.displays()?;
    let index = monitor_index(stream, displays.iter().map(|display| display.geometry))?;
    let display = &displays[index];
    ensure!(
        display.name == region.output && display.geometry == region.geometry,
        "Share the same display that was used for the screenshot"
    );
    ensure!(
        display.transform == Transform::Normal && display.size == region.source_size,
        "The shared display has a different pixel mode than the screenshot"
    );
    Ok(())
}

fn pointer_anchor(
    geometry: DesktopRect,
    source_size: (u32, u32),
    [left, top, right, bottom]: [u32; 4],
) -> (f64, f64) {
    let center_x = (f64::from(left) + f64::from(right)) / 2.0;
    let center_y = (f64::from(top) + f64::from(bottom)) / 2.0;
    (
        center_x * f64::from(geometry.width) / f64::from(source_size.0),
        center_y * f64::from(geometry.height) / f64::from(source_size.1),
    )
}

fn scroll_input(direction: Direction) -> (Axis, i32) {
    match direction {
        Direction::Down => (Axis::Vertical, 1),
        Direction::Up => (Axis::Vertical, -1),
        Direction::Left => (Axis::Horizontal, -1),
        Direction::Right => (Axis::Horizontal, 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_is_relative_to_the_monitor_in_logical_units() {
        let geometry = DesktopRect {
            x: -1600,
            y: 120,
            width: 1600,
            height: 900,
        };
        assert_eq!(
            pointer_anchor(geometry, (3200, 1800), [100, 200, 500, 700]),
            (150.0, 225.0)
        );
    }

    #[test]
    fn portal_receives_one_detent_with_the_axis_and_sign_for_each_direction() {
        assert_eq!(scroll_input(Direction::Down), (Axis::Vertical, 1));
        assert_eq!(scroll_input(Direction::Up), (Axis::Vertical, -1));
        assert_eq!(scroll_input(Direction::Left), (Axis::Horizontal, -1));
        assert_eq!(scroll_input(Direction::Right), (Axis::Horizontal, 1));
    }
}
