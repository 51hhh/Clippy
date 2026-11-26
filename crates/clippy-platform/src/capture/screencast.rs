use super::{
    screencast_stream::{ScreenCastStream, StreamMode},
    DesktopRect, FrozenPixels, RegionCancellation, ScreenSharingCancelled,
};
use anyhow::{bail, ensure, Context, Result};
use ashpd::desktop::{
    screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType, Stream},
    PersistMode, Session,
};
use std::{future::Future, time::Duration};
use tokio::{runtime::Runtime, sync::oneshot};

const CALL_TIMEOUT: Duration = Duration::from_secs(5);
const CHOOSER_TIMEOUT: Duration = Duration::from_secs(120);
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);

pub(super) struct ScreenCast {
    runtime: Runtime,
    connection: zbus::Connection,
    portal: Screencast,
    session: Option<Session<Screencast>>,
}

impl ScreenCast {
    pub fn connect(cancellation: &RegionCancellation) -> Result<Option<Self>> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (connection, portal) = runtime
            .block_on(cancellable(cancellation, CALL_TIMEOUT, async {
                let connection = zbus::Connection::session().await?;
                let portal = match Screencast::with_connection(connection.clone()).await {
                    Ok(portal) => portal,
                    Err(error) if unavailable(&error) => return Ok((connection, None)),
                    Err(error) => return Err(error.into()),
                };
                match portal.available_source_types().await {
                    Ok(types) if types.contains(SourceType::Monitor) => {
                        Ok((connection, Some(portal)))
                    }
                    Ok(_) => Ok((connection, None)),
                    Err(error) if unavailable(&error) => Ok((connection, None)),
                    Err(error) => Err(error.into()),
                }
            }))
            .context("Checking ScreenCast Portal availability")?;
        Ok(portal.map(|portal| Self {
            runtime,
            connection,
            portal,
            session: None,
        }))
    }

    pub fn authorize(&mut self, cancellation: &RegionCancellation) -> Result<Stream> {
        let (session, options) =
            self.runtime
                .block_on(cancellable(cancellation, CALL_TIMEOUT, async {
                    let mut options = SelectSourcesOptions::default()
                        .set_sources(Some(SourceType::Monitor.into()))
                        .set_multiple(false);
                    if self.portal.version() >= 2 {
                        let modes = self.portal.available_cursor_modes().await?;
                        ensure!(
                            modes.contains(CursorMode::Hidden),
                            "ScreenCast cannot hide the cursor"
                        );
                        options = options.set_cursor_mode(CursorMode::Hidden);
                    }
                    if self.portal.version() >= 4 {
                        options = options.set_persist_mode(PersistMode::DoNot);
                    }
                    let session = self.portal.create_session(Default::default()).await?;
                    Ok((session, options))
                }))?;
        let session = self.session.insert(session);
        // wlr 可能在 SelectSources 就打开系统选择器，两次调用共享用户操作的时限。
        let granted = self
            .runtime
            .block_on(cancellable(cancellation, CHOOSER_TIMEOUT, async {
                self.portal
                    .select_sources(session, options)
                    .await?
                    .response()?;
                Ok(self
                    .portal
                    .start(session, None, Default::default())
                    .await?
                    .response()?)
            }));
        let granted = match granted {
            Err(error)
                if error.downcast_ref::<ashpd::Error>().is_some_and(|error| {
                    matches!(
                        error,
                        ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)
                    )
                }) =>
            {
                return Err(ScreenSharingCancelled.into());
            }
            result => result?,
        };
        let [stream] = granted.streams() else {
            bail!("ScreenCast must return one monitor stream");
        };
        Ok(stream.clone())
    }

    pub fn open_stream(
        &self,
        node: u32,
        source_size: (u32, u32),
        mode: StreamMode,
        cancellation: &RegionCancellation,
    ) -> Result<(ScreenCastStream, FrozenPixels)> {
        let session = self.session.as_ref().expect("屏幕流必须已授权");
        let remote = self
            .runtime
            .block_on(cancellable(cancellation, CALL_TIMEOUT, async {
                Ok(self
                    .portal
                    .open_pipe_wire_remote(session, Default::default())
                    .await?)
            }))
            .context("Opening the authorized ScreenCast PipeWire connection")?;
        let (stream, first) = ScreenCastStream::start(remote, node, source_size, mode)?;
        Ok((stream, self.wait_for_frame(first, cancellation)?))
    }

    fn wait_for_frame(
        &self,
        first: oneshot::Receiver<Result<FrozenPixels>>,
        cancellation: &RegionCancellation,
    ) -> Result<FrozenPixels> {
        self.runtime
            .block_on(cancellable(cancellation, FIRST_FRAME_TIMEOUT, async {
                first
                    .await
                    .context("ScreenCast exited before the first frame")?
            }))
            .context("Waiting for the first ScreenCast frame")
    }

    pub fn resume_region(
        &self,
        stream: &ScreenCastStream,
        bounds: [u32; 4],
        fresh_from: Option<i64>,
        cancellation: &RegionCancellation,
    ) -> Result<()> {
        cancellation.check_active()?;
        self.wait_for_frame(stream.resume(bounds, fresh_from)?, cancellation)?;
        Ok(())
    }
}

impl Drop for ScreenCast {
    fn drop(&mut self) {
        self.runtime.block_on(async {
            if let Some(session) = self.session.take() {
                let _ = tokio::time::timeout(CLOSE_TIMEOUT, session.close()).await;
            }
            // 请求还在等待用户选择时，断开任务自己的连接也会回收关联的系统对话框。
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, self.connection.clone().close()).await;
        });
    }
}

async fn cancellable<T>(
    cancellation: &RegionCancellation,
    timeout: Duration,
    operation: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(ScreenSharingCancelled.into()),
        result = tokio::time::timeout(timeout, operation) =>
            result.context("ScreenCast Portal request timed out")?,
    }
}

fn unavailable(error: &ashpd::Error) -> bool {
    let missing = |name: &str| {
        matches!(
            name,
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
                | "org.freedesktop.DBus.Error.UnknownInterface"
                | "org.freedesktop.DBus.Error.UnknownProperty"
                | "org.freedesktop.DBus.Error.UnknownMethod"
        )
    };
    match error {
        ashpd::Error::PortalNotFound(_) => true,
        ashpd::Error::Zbus(zbus::Error::MethodError(name, _, _)) => missing(name.as_str()),
        ashpd::Error::Zbus(zbus::Error::FDO(error)) => {
            missing(zbus::DBusError::name(error.as_ref()).as_str())
        }
        _ => false,
    }
}

pub(super) fn monitor_index(
    stream: &Stream,
    outputs: impl ExactSizeIterator<Item = DesktopRect>,
) -> Result<usize> {
    ensure!(
        stream
            .source_type()
            .is_none_or(|kind| kind == SourceType::Monitor),
        "ScreenCast did not return a monitor"
    );
    let position = stream.position();
    let size = stream
        .size()
        .map(|(width, height)| {
            Ok::<_, anyhow::Error>((u32::try_from(width)?, u32::try_from(height)?))
        })
        .transpose()?;
    ensure!(
        outputs.len() == 1 || (position.is_some() && size.is_some()),
        "ScreenCast did not identify a display in this multi-monitor setup"
    );
    let mut matching = outputs.enumerate().filter(|(_, geometry)| {
        position.is_none_or(|value| value == (geometry.x, geometry.y))
            && size.is_none_or(|value| value == (geometry.width, geometry.height))
    });
    let (index, _) = matching
        .next()
        .context("The shared display is unavailable")?;
    ensure!(
        matching.next().is_none(),
        "ScreenCast cannot distinguish mirrored displays"
    );
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use zbus::zvariant::{serialized::Context, to_bytes, Value, LE};

    fn reply(monitor: HashMap<&str, Value<'_>>) -> Stream {
        let data = to_bytes(Context::new_dbus(LE, 0), &(55_u32, monitor)).unwrap();
        data.deserialize::<Stream>().unwrap().0
    }

    #[test]
    fn screen_geometry_uses_logical_size_and_rejects_ambiguous_or_different_monitors() {
        let geometry = DesktopRect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let stream = reply(HashMap::from([
            ("source_type", Value::U32(SourceType::Monitor as u32)),
            ("position", Value::from((-1920, 0))),
            ("size", Value::from((1920, 1080))),
        ]));
        let other = DesktopRect { x: 0, ..geometry };
        assert_eq!(
            monitor_index(&stream, [other, geometry].into_iter()).unwrap(),
            1
        );
        assert!(monitor_index(&stream, [geometry, geometry].into_iter()).is_err());
        assert!(monitor_index(&stream, [other].into_iter()).is_err());
        let opaque = reply(HashMap::new());
        assert_eq!(monitor_index(&opaque, [geometry].into_iter()).unwrap(), 0);
        assert!(monitor_index(&opaque, [geometry, other].into_iter()).is_err());
        let wrong_kind = reply(HashMap::from([(
            "source_type",
            Value::U32(SourceType::Window as u32),
        )]));
        assert!(monitor_index(&wrong_kind, [geometry].into_iter()).is_err());
    }

    #[test]
    fn portal_cancellation_wins_before_a_ready_reply() {
        let cancellation = RegionCancellation::default();
        cancellation.cancel();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let result = runtime.block_on(cancellable(&cancellation, CALL_TIMEOUT, async { Ok(1) }));
        assert_eq!(result.unwrap_err().to_string(), "Screen sharing cancelled");
    }
}
