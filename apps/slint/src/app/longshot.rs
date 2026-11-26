use super::{capture, clipboard, editor, pins, post, settings, App, Event};
use crate::{LongshotDirection, LongshotWindow};
use anyhow::{anyhow, Context, Result};
use clippy_capture::{editor::Editor, longshot::LongshotSession};
use clippy_core::image_output::SaveTarget;
use clippy_platform::{
    capture::{Region, RegionCancellation},
    scroll,
};
use image::RgbaImage;
use slint::{
    winit_030::{winit::raw_window_handle::HasWindowHandle, WinitWindowAccessor},
    CloseRequestResponse, ComponentHandle, Timer,
};
use std::{sync::Arc, thread, time::Duration};
use tokio::sync::mpsc::{self, Sender};

#[cfg(target_os = "linux")]
use slint::winit_030::winit::window::{Window, WindowId};
#[cfg(target_os = "linux")]
use std::{sync::mpsc as close_signal, time::Instant};

mod automatic;
mod worker;

#[cfg(target_os = "linux")]
type ControlClose = close_signal::Receiver<Instant>;

pub(super) enum Action {
    Append,
    Undo,
    Finish,
    Copy,
    Save,
    Pin,
    Cancel,
    #[cfg(target_os = "linux")]
    Authorize,
    Auto(LongshotDirection),
    Stop,
}

enum Command {
    Append,
    #[cfg(target_os = "linux")]
    AppendPortal(ControlClose),
    Undo,
    Finish,
    Auto(automatic::Request, scroll::ControlWindow),
    #[cfg(target_os = "linux")]
    Authorize(Arc<Window>),
    #[cfg(target_os = "linux")]
    AutoPortal(automatic::Request, ControlClose),
    Export(ExportImage),
    Save(SaveTarget),
}

pub(super) enum ExportImage {
    Copy,
    Pin,
}

pub(super) enum Message {
    Action(Action),
    Preparing(u64),
    Feedback(u64, Feedback),
    Finish(u64, Result<RgbaImage>),
    Export(u64, ExportImage, Result<RgbaImage>),
    Exported(u64, Result<String>),
    Failed(u64, anyhow::Error),
}

pub(super) struct Feedback {
    preview: RgbaImage,
    frames: usize,
    width: u32,
    height: u32,
    status: String,
    auto_scroll: AutoMode,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum AutoMode {
    Unavailable,
    Native,
    #[cfg(target_os = "linux")]
    PermissionRequired,
    #[cfg(target_os = "linux")]
    Portal,
}

pub(super) struct View {
    id: u64,
    commands: Sender<Command>,
    window: LongshotWindow,
    busy: bool,
    cancellation: RegionCancellation,
    auto_scroll: Option<automatic::Permit>,
    auto_mode: AutoMode,
    #[cfg(target_os = "linux")]
    control_close: Option<(WindowId, close_signal::SyncSender<Instant>)>,
}

impl Drop for View {
    fn drop(&mut self) {
        if let Some(permit) = &self.auto_scroll {
            permit.revoke();
        }
        self.cancellation.cancel();
        let _ = self.window.hide();
    }
}

fn send(message: Message) {
    post(Event::Longshot(message));
}

fn action(action: Action) {
    send(Message::Action(action));
}

impl View {
    fn open(id: u64, region: Region, first: RgbaImage) -> Result<Self> {
        let window = LongshotWindow::new()?;
        settings::connect_titlebar(&window, LongshotWindow::invoke_titlebar_contains);
        window.on_append(|| action(Action::Append));
        window.on_undo(|| action(Action::Undo));
        window.on_finish(|| action(Action::Finish));
        window.on_copy(|| action(Action::Copy));
        window.on_save(|| action(Action::Save));
        window.on_pin(|| action(Action::Pin));
        window.on_cancel(|| action(Action::Cancel));
        #[cfg(target_os = "linux")]
        window.on_authorize_scroll(|| action(Action::Authorize));
        window.on_auto_scroll(|direction| action(Action::Auto(direction)));
        window.window().on_close_requested(|| {
            action(Action::Cancel);
            CloseRequestResponse::KeepWindowShown
        });
        let defer_window = scroll::uses_native_target();
        if !defer_window {
            window.show()?;
        }
        let (commands, receiver) = mpsc::channel(1);
        let cancellation = RegionCancellation::default();
        let worker_cancellation = cancellation.clone();
        thread::Builder::new()
            .name("clippy-scrolling-capture".into())
            .spawn(move || {
                if let Err(error) = worker::run(
                    id,
                    region,
                    first,
                    worker_cancellation,
                    receiver,
                    defer_window,
                ) {
                    send(Message::Failed(id, error));
                }
            })?;
        Ok(Self {
            id,
            commands,
            window,
            busy: true,
            cancellation,
            auto_scroll: None,
            auto_mode: AutoMode::Unavailable,
            #[cfg(target_os = "linux")]
            control_close: None,
        })
    }

    fn start_automatic(&mut self, direction: LongshotDirection) -> Result<()> {
        let permit = automatic::Permit::new();
        let request = automatic::Request {
            direction,
            permit: permit.clone(),
        };
        let command = match self.auto_mode {
            AutoMode::Native => {
                let control = self
                    .window
                    .window()
                    .with_winit_window(|native| {
                        scroll::ControlWindow::from_handle(native.window_handle()?)
                    })
                    .context("Could not identify the native scrolling controls")??;
                self.window.hide()?;
                Command::Auto(request, control)
            }
            #[cfg(target_os = "linux")]
            AutoMode::Portal => Command::AutoPortal(request, self.hide_portal_controls()?),
            _ => return Err(anyhow!("Automatic scrolling is unavailable")),
        };
        self.commands
            .try_send(command)
            .map_err(|_| anyhow!("Capture session closed"))?;
        self.auto_scroll = Some(permit);
        self.busy = true;
        self.window.set_busy(true);
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn hide_portal_controls(&mut self) -> Result<ControlClose> {
        let window_id = self
            .window
            .window()
            .with_winit_window(|window| window.id())
            .context("Could not identify the Wayland scrolling controls")?;
        let (sender, closed) = close_signal::sync_channel(1);
        self.control_close = Some((window_id, sender));
        self.window.hide()?;
        Ok(closed)
    }

    #[cfg(target_os = "linux")]
    fn request_permission(&mut self) -> Result<()> {
        let id = self.id;
        let controls = self.window.as_weak();
        let commands = self.commands.clone();
        slint::spawn_local(async move {
            let result = async {
                let controls = controls.upgrade().context("Scrolling capture closed")?;
                let native = controls.window().winit_window().await?;
                commands
                    .try_send(Command::Authorize(native))
                    .map_err(|_| anyhow!("Scrolling capture closed"))
            }
            .await;
            if let Err(error) = result {
                send(Message::Exported(id, Err(error)));
            }
        })?;
        self.busy = true;
        self.window.set_busy(true);
        self.window
            .set_status("Waiting for screen and pointer permission…".into());
        Ok(())
    }
}

fn feedback(session: &LongshotSession, note: String, auto_scroll: AutoMode) -> Result<Feedback> {
    let state = session.snapshot();
    let preview = session.preview_pixels()?;
    Ok(Feedback {
        preview,
        frames: state.frame_count,
        width: state.width,
        height: state.total_height,
        status: note,
        auto_scroll,
    })
}

impl App {
    pub(super) fn start_longshot(&mut self, result: Result<(Region, RgbaImage)>) {
        if let Some(editor) = self.editor.as_ref() {
            let _ = editor.show();
            self.set_busy(false);
            return;
        }
        self.capture_task_id += 1;
        match result.and_then(|(region, first)| View::open(self.capture_task_id, region, first)) {
            Ok(view) => self.longshot = Some(view),
            Err(error) => self.capture_error(error),
        }
    }

    pub(super) fn longshot_message(&mut self, message: Message) {
        let Some(view) = self.longshot.as_mut() else {
            return;
        };
        match message {
            Message::Action(Action::Cancel) => {
                self.longshot.take();
                self.tray.set_scrolling(false);
                self.set_busy(false);
            }
            Message::Action(Action::Stop) => {
                if let Some(permit) = &view.auto_scroll {
                    permit.revoke();
                }
            }
            #[cfg(target_os = "linux")]
            Message::Action(Action::Authorize) => {
                if !view.busy && view.auto_mode == AutoMode::PermissionRequired {
                    if let Err(error) = view.request_permission() {
                        send(Message::Exported(view.id, Err(error)));
                    }
                }
            }
            Message::Action(Action::Auto(direction)) if !view.busy => {
                match view.start_automatic(direction) {
                    Ok(()) => self.tray.set_scrolling(true),
                    Err(error) => send(Message::Failed(view.id, error)),
                }
            }
            Message::Action(action) if !view.busy => {
                view.busy = true;
                view.window.set_busy(true);
                let id = view.id;
                let sender = view.commands.clone();
                #[cfg(target_os = "linux")]
                if matches!(action, Action::Append) && view.auto_mode == AutoMode::Portal {
                    let result = view.hide_portal_controls().and_then(|closed| {
                        sender
                            .try_send(Command::AppendPortal(closed))
                            .map_err(|_| anyhow!("Capture session closed"))
                    });
                    if let Err(error) = result {
                        send(Message::Failed(id, error));
                    }
                    return;
                }
                let command = match action {
                    Action::Append => Command::Append,
                    Action::Undo => Command::Undo,
                    Action::Finish => Command::Finish,
                    Action::Copy => Command::Export(ExportImage::Copy),
                    Action::Pin => Command::Export(ExportImage::Pin),
                    Action::Save => Command::Save(SaveTarget::from_config(
                        &self.config,
                        &self.paths.screenshot_default(self.identity),
                    )),
                    _ => unreachable!("其他长截图动作已处理"),
                };
                if matches!(command, Command::Append) {
                    let _ = view.window.hide();
                    Timer::single_shot(Duration::from_millis(32), move || {
                        if sender.try_send(command).is_err() {
                            send(Message::Failed(id, anyhow!("Capture session closed")));
                        }
                    });
                } else if sender.try_send(command).is_err() {
                    send(Message::Failed(id, anyhow!("Capture session closed")));
                }
            }
            Message::Preparing(id) if id == view.id => {
                if let Err(error) = view.window.show() {
                    send(Message::Failed(id, error.into()));
                }
            }
            Message::Feedback(id, feedback) if id == view.id => {
                view.auto_scroll.take();
                #[cfg(target_os = "linux")]
                {
                    view.control_close.take();
                    view.window.set_scroll_permission_required(
                        feedback.auto_scroll == AutoMode::PermissionRequired,
                    );
                }
                self.tray.set_scrolling(false);
                view.auto_mode = feedback.auto_scroll;
                view.window.set_preview(editor::to_slint(&feedback.preview));
                view.busy = false;
                view.window.set_busy(false);
                view.window.set_can_undo(feedback.frames > 1);
                view.window.set_frames(feedback.frames as i32);
                view.window.set_capture_width(feedback.width as i32);
                view.window.set_capture_height(feedback.height as i32);
                view.window.set_status(feedback.status.into());
                view.window
                    .set_auto_scroll_available(match feedback.auto_scroll {
                        AutoMode::Native => true,
                        #[cfg(target_os = "linux")]
                        AutoMode::Portal => true,
                        _ => false,
                    });
                if let Err(error) = view.window.show() {
                    send(Message::Failed(id, error.into()));
                }
            }
            Message::Finish(id, result) if id == view.id => {
                self.longshot.take();
                self.set_busy(false);
                if let Err(error) = result.and_then(|image| self.open_editor(image)) {
                    self.capture_error(error);
                }
            }
            Message::Export(id, destination, result) if id == view.id => {
                let result = result.and_then(|image| {
                    let image = Arc::new(image);
                    match destination {
                        ExportImage::Copy => {
                            self.clipboard.submit(clipboard::Request::WriteImage {
                                image,
                                finish: Box::new(move |result| {
                                    send(Message::Exported(
                                        id,
                                        result.map(|()| "Image copied".to_owned()),
                                    ));
                                }),
                            })
                        }
                        ExportImage::Pin => self.pins.create(
                            pins::PinRequest::Longshot(id),
                            Editor::new(image.clone()).snapshot(),
                            image,
                            self.storage.client(),
                        ),
                    }
                });
                if let Err(error) = result {
                    send(Message::Exported(id, Err(error)));
                }
            }
            Message::Exported(id, result) if id == view.id => {
                view.busy = false;
                view.window.set_busy(false);
                view.window
                    .set_status(result.unwrap_or_else(|error| format!("{error:#}")).into());
            }
            Message::Failed(id, error) if id == view.id => {
                self.longshot.take();
                self.tray.set_scrolling(false);
                self.capture_error(error);
            }
            _ => {}
        }
    }

    #[cfg(target_os = "linux")]
    pub(super) fn longshot_window_destroyed(&mut self, id: WindowId, closed: Instant) {
        if let Some(view) = self.longshot.as_mut() {
            if view
                .control_close
                .as_ref()
                .is_some_and(|(pending, _)| *pending == id)
            {
                let (_, notify) = view.control_close.take().expect("控制窗正在关闭");
                let _ = notify.try_send(closed);
            }
        }
    }
}
