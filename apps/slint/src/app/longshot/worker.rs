use super::{capture, feedback, send, AutoMode, Command, Message};
use anyhow::Result;
use clippy_capture::{
    longshot::{LongshotAppendOutcome, LongshotSession},
    CaptureError,
};
use clippy_platform::{
    capture::{Region, RegionCancellation, RegionReader},
    scroll::{self, Target},
};
use image::RgbaImage;
use tokio::sync::mpsc::Receiver;

#[cfg(target_os = "linux")]
use {
    super::ControlClose,
    anyhow::{ensure, Context},
    clippy_platform::{capture::wayland_scroll::AuthorizedScroll, session},
    std::time::{Duration, Instant},
};

struct Worker {
    id: u64,
    canvas: LongshotSession,
    manual: Option<RegionReader>,
    target: Option<Box<dyn Target>>,
    #[cfg(target_os = "linux")]
    portal: Option<PortalCapture>,
}

#[cfg(target_os = "linux")]
struct PortalCapture {
    region: Region,
    cancellation: RegionCancellation,
    authorized: Option<AuthorizedScroll>,
}

pub(super) fn run(
    id: u64,
    region: Region,
    first: RgbaImage,
    cancellation: RegionCancellation,
    mut commands: Receiver<Command>,
    defer_window: bool,
) -> Result<()> {
    let (target, ready) = match scroll::for_region(&region) {
        Ok(Some(target)) => (
            Some(target),
            "Move the pointer or use the tray to stop automatic scrolling".to_owned(),
        ),
        Ok(None) => (None, "Ready for the next frame".to_owned()),
        Err(error) => (None, format!("{error:#}")),
    };
    if defer_window {
        send(Message::Preparing(id));
    }
    #[cfg(target_os = "linux")]
    let portal = if session::is_wayland() {
        match AuthorizedScroll::available(&cancellation) {
            Ok(true) => Some(PortalCapture {
                region: without_source(&region),
                cancellation: cancellation.clone(),
                authorized: None,
            }),
            Ok(false) => None,
            Err(error) => {
                eprintln!("Could not check scrolling permission: {error:#}");
                None
            }
        }
    } else {
        None
    };
    let mut worker = Worker {
        id,
        canvas: LongshotSession::start(first)?,
        manual: Some(RegionReader::open(region, cancellation)?),
        target,
        #[cfg(target_os = "linux")]
        portal,
    };
    let ready = match worker.auto_mode() {
        #[cfg(target_os = "linux")]
        AutoMode::PermissionRequired => "Authorize scrolling or add a frame manually".into(),
        _ => ready,
    };
    worker.publish(ready)?;

    while let Some(command) = worker.next_command(&mut commands)? {
        let status = match command {
            Command::Append => append_status(
                worker
                    .read_manual()
                    .and_then(|image| worker.canvas.append(image).map_err(Into::into)),
            ),
            #[cfg(target_os = "linux")]
            Command::AppendPortal(close) => append_status(
                worker
                    .read_portal(close)
                    .and_then(|image| worker.canvas.append(image).map_err(Into::into)),
            ),
            Command::Undo => {
                worker.canvas.undo()?;
                "Last frame removed".into()
            }
            Command::Finish => {
                send(Message::Finish(
                    id,
                    worker.canvas.finish_pixels().map_err(Into::into),
                ));
                break;
            }
            Command::Auto(request, control) => {
                let target = worker
                    .target
                    .as_deref_mut()
                    .expect("原生自动滚动已锁定窗口");
                let source = worker.manual.as_mut().expect("原生自动滚动已有读取器");
                status(
                    request.run_native(control, target, &mut worker.canvas, &mut || source.read()),
                )
            }
            #[cfg(target_os = "linux")]
            Command::Authorize(window) => status(worker.authorize(&window)),
            #[cfg(target_os = "linux")]
            Command::AutoPortal(request, close) => status(worker.run_portal(request, close)),
            Command::Export(destination) => {
                send(Message::Export(
                    id,
                    destination,
                    worker.canvas.finish_pixels().map_err(Into::into),
                ));
                continue;
            }
            Command::Save(target) => {
                let result = worker
                    .canvas
                    .finish_pixels()
                    .map_err(anyhow::Error::from)
                    .and_then(|image| capture::save_png(&image, &target))
                    .map(|path| format!("Saved to {}", path.display()));
                send(Message::Exported(id, result));
                continue;
            }
        };
        worker.publish(status)?;
    }
    Ok(())
}

impl Worker {
    fn auto_mode(&self) -> AutoMode {
        #[cfg(target_os = "linux")]
        if let Some(portal) = &self.portal {
            return if portal.authorized.is_some() {
                AutoMode::Portal
            } else {
                AutoMode::PermissionRequired
            };
        }
        if self.target.is_some() {
            AutoMode::Native
        } else {
            AutoMode::Unavailable
        }
    }

    fn publish(&self, status: String) -> Result<()> {
        send(Message::Feedback(
            self.id,
            feedback(&self.canvas, status, self.auto_mode())?,
        ));
        Ok(())
    }

    fn next_command(&mut self, commands: &mut Receiver<Command>) -> Result<Option<Command>> {
        #[cfg(target_os = "linux")]
        if let Some(portal) = &mut self.portal {
            if let Some(authorized) = &portal.authorized {
                match authorized.while_active(commands.recv()) {
                    Ok(command) => return Ok(command),
                    Err(error) => {
                        portal.authorized.take();
                        self.publish(format!("{error:#}. You can still add frames manually."))?;
                    }
                }
            }
        }
        Ok(commands.blocking_recv())
    }

    fn read_manual(&mut self) -> Result<RgbaImage> {
        #[cfg(target_os = "linux")]
        if self.manual.is_none() {
            let portal = self.portal.as_ref().expect("只有 Wayland 授权会切换采集源");
            self.manual = Some(RegionReader::open(
                without_source(&portal.region),
                portal.cancellation.clone(),
            )?);
        }
        self.manual.as_mut().expect("已打开手动采集源").read()
    }

    #[cfg(target_os = "linux")]
    fn authorize(&mut self, window: &super::Window) -> Result<&'static str> {
        let portal = self.portal.as_mut().expect("授权入口要求 Portal 可用");
        ensure!(
            portal.authorized.is_none(),
            "Scrolling is already authorized"
        );
        portal.authorized = Some(AuthorizedScroll::authorize(
            &portal.region,
            window,
            &portal.cancellation,
            || drop(self.manual.take()),
        )?);
        Ok("Choose a direction or use the tray to stop automatic scrolling")
    }

    #[cfg(target_os = "linux")]
    fn read_portal(&mut self, close: ControlClose) -> Result<RgbaImage> {
        let hidden = hidden_at(close)?;
        let portal = self
            .portal
            .as_mut()
            .expect("Wayland 控制窗要求 Portal 可用");
        if let Some(authorized) = &mut portal.authorized {
            let result = authorized
                .refresh_after(hidden)
                .and_then(|()| authorized.read());
            if result.is_err() {
                portal.authorized.take();
            }
            return result;
        }
        self.read_manual()
    }

    #[cfg(target_os = "linux")]
    fn run_portal(
        &mut self,
        request: super::automatic::Request,
        close: ControlClose,
    ) -> Result<&'static str> {
        let hidden = hidden_at(close)?;
        let portal = self.portal.as_mut().expect("自动滚动要求 Portal 可用");
        let authorized = portal
            .authorized
            .as_mut()
            .context("Scrolling permission ended")?;
        let result = request.run_portal(hidden, authorized, &mut self.canvas);
        if result.is_err() {
            portal.authorized.take();
        }
        result
    }
}

fn status(result: Result<&'static str>) -> String {
    result
        .map(str::to_owned)
        .unwrap_or_else(|error| format!("{error:#}"))
}

fn append_status(result: Result<LongshotAppendOutcome>) -> String {
    match result {
        Ok(result) if result.committed => "Frame added".into(),
        Ok(_) => "This part is already captured".into(),
        Err(error)
            if matches!(
                error.downcast_ref(),
                Some(CaptureError::LongshotEstimateNoExtension)
            ) =>
        {
            "No movement detected. Scroll farther and try again.".into()
        }
        Err(error) => format!("Frame not added: {error}"),
    }
}

#[cfg(target_os = "linux")]
fn hidden_at(close: ControlClose) -> Result<Instant> {
    close
        .recv_timeout(Duration::from_millis(500))
        .context("Scrolling controls did not hide in time")
}

#[cfg(target_os = "linux")]
fn without_source(region: &Region) -> Region {
    Region {
        output: region.output.clone(),
        geometry: region.geometry,
        source_size: region.source_size,
        bounds: region.bounds,
        source: Default::default(),
    }
}
