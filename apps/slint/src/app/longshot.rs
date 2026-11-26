use super::{capture, clipboard, editor, pins, post, settings, App, Event};
use crate::{LongshotDirection, LongshotWindow};
use anyhow::{anyhow, Context, Result};
use clippy_capture::{editor::Editor, longshot::LongshotSession, CaptureError};
use clippy_core::image_output::SaveTarget;
use clippy_platform::{
    capture::{with_region_reader, Region, RegionCancellation},
    scroll,
};
use image::RgbaImage;
use slint::{
    winit_030::{winit::raw_window_handle::HasWindowHandle, WinitWindowAccessor},
    CloseRequestResponse, ComponentHandle, Timer,
};
use std::{
    sync::{
        mpsc::{self, SyncSender},
        Arc,
    },
    thread,
    time::Duration,
};

mod automatic;

pub(super) enum Action {
    Append,
    Undo,
    Finish,
    Copy,
    Save,
    Pin,
    Cancel,
    Auto(LongshotDirection),
    Stop,
}

enum Command {
    Append,
    Undo,
    Finish,
    Auto(automatic::Request),
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
    auto_scroll: bool,
}

pub(super) struct View {
    id: u64,
    commands: SyncSender<Command>,
    window: LongshotWindow,
    busy: bool,
    cancellation: RegionCancellation,
    auto_scroll: Option<automatic::Permit>,
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
        window.on_auto_scroll(|direction| action(Action::Auto(direction)));
        window.window().on_close_requested(|| {
            action(Action::Cancel);
            CloseRequestResponse::KeepWindowShown
        });
        let defer_window = scroll::uses_native_target();
        if !defer_window {
            window.show()?;
        }
        let (commands, receiver) = mpsc::sync_channel(1);
        let cancellation = RegionCancellation::default();
        let worker_cancellation = cancellation.clone();
        thread::Builder::new()
            .name("clippy-scrolling-capture".into())
            .spawn(move || {
                let (mut target, ready) = match scroll::for_region(&region) {
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
                let auto_scroll = target.is_some();
                let result = with_region_reader(region, &worker_cancellation, |read| {
                    let mut session = LongshotSession::start(first)?;
                    send(Message::Feedback(
                        id,
                        feedback(&session, ready, auto_scroll)?,
                    ));
                    while let Ok(command) = receiver.recv() {
                        let status = match command {
                            Command::Append => match read()
                                .and_then(|image| session.append(image).map_err(Into::into))
                            {
                                Ok(result) if result.committed => "Frame added".into(),
                                Ok(_) => "This part is already captured".into(),
                                Err(error)
                                    if error.downcast_ref::<CaptureError>().is_some_and(
                                        |cause| {
                                            matches!(
                                                cause,
                                                CaptureError::LongshotEstimateNoExtension
                                            )
                                        },
                                    ) =>
                                {
                                    "No movement detected. Scroll farther and try again.".into()
                                }
                                Err(error) => format!("Frame not added: {error}"),
                            },
                            Command::Undo => {
                                session.undo()?;
                                "Last frame removed".into()
                            }
                            Command::Finish => {
                                let result = session.finish_pixels().map_err(anyhow::Error::from);
                                send(Message::Finish(id, result));
                                break;
                            }
                            Command::Auto(request) => {
                                let target = target
                                    .as_deref_mut()
                                    .expect("自动滚动入口要求已锁定目标窗口");
                                match request.run(target, &mut session, read) {
                                    Ok(status) => status.to_owned(),
                                    Err(error) => format!("{error:#}"),
                                }
                            }
                            Command::Export(destination) => {
                                send(Message::Export(
                                    id,
                                    destination,
                                    session.finish_pixels().map_err(Into::into),
                                ));
                                continue;
                            }
                            Command::Save(target) => {
                                let result = session
                                    .finish_pixels()
                                    .map_err(anyhow::Error::from)
                                    .and_then(|image| capture::save_png(&image, &target))
                                    .map(|path| format!("Saved to {}", path.display()));
                                send(Message::Exported(id, result));
                                continue;
                            }
                        };
                        send(Message::Feedback(
                            id,
                            feedback(&session, status, auto_scroll)?,
                        ));
                    }
                    Ok(())
                });
                if let Err(error) = result {
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
        })
    }

    fn start_automatic(&mut self, direction: LongshotDirection) -> Result<()> {
        let control_window = self
            .window
            .window()
            .with_winit_window(|native| scroll::ControlWindow::from_handle(native.window_handle()?))
            .context("Could not identify the native scrolling controls")??;
        self.window.hide()?;
        let permit = automatic::Permit::new();
        self.commands
            .try_send(Command::Auto(automatic::Request {
                direction,
                control_window,
                permit: permit.clone(),
            }))
            .map_err(|_| anyhow!("Capture session closed"))?;
        self.auto_scroll = Some(permit);
        self.busy = true;
        self.window.set_busy(true);
        Ok(())
    }
}

fn feedback(session: &LongshotSession, note: String, auto_scroll: bool) -> Result<Feedback> {
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
                self.tray.set_scrolling(false);
                view.window.set_preview(editor::to_slint(&feedback.preview));
                view.busy = false;
                view.window.set_busy(false);
                view.window.set_can_undo(feedback.frames > 1);
                view.window.set_frames(feedback.frames as i32);
                view.window.set_capture_width(feedback.width as i32);
                view.window.set_capture_height(feedback.height as i32);
                view.window.set_status(feedback.status.into());
                view.window.set_auto_scroll_available(feedback.auto_scroll);
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
}
