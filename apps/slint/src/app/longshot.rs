use super::{editor, post, settings, App, Event};
use crate::{ClippyTheme, LongshotWindow};
use anyhow::{anyhow, Result};
use clippy_capture::{longshot::LongshotSession, CaptureError};
use clippy_platform::capture::{with_region_reader, Region};
use image::RgbaImage;
use slint::{CloseRequestResponse, ComponentHandle, Timer};
use std::{
    sync::mpsc::{self, SyncSender},
    thread,
    time::Duration,
};

#[derive(Clone, Copy)]
pub(super) enum Command {
    Append,
    Undo,
    Finish,
    Cancel,
}

pub(super) enum Message {
    Action(Command),
    Feedback(u64, Feedback),
    Finish(u64, Result<RgbaImage>),
    Failed(u64, anyhow::Error),
}

pub(super) struct Feedback {
    preview: RgbaImage,
    frames: usize,
    status: String,
}

pub(super) struct View {
    id: u64,
    commands: SyncSender<Command>,
    window: LongshotWindow,
    busy: bool,
}

impl Drop for View {
    fn drop(&mut self) {
        let _ = self.window.hide();
    }
}

fn send(message: Message) {
    post(Event::Longshot(message));
}

fn action(command: Command) {
    send(Message::Action(command));
}

impl View {
    fn open(id: u64, region: Region, first: RgbaImage, theme: i32) -> Result<Self> {
        let window = LongshotWindow::new()?;
        let appearance = window.global::<ClippyTheme>();
        appearance.set_index(theme);
        settings::connect_window_frame(&window, appearance);
        window.on_append(|| action(Command::Append));
        window.on_undo(|| action(Command::Undo));
        window.on_finish(|| action(Command::Finish));
        window.on_cancel(|| action(Command::Cancel));
        window.window().on_close_requested(|| {
            action(Command::Cancel);
            CloseRequestResponse::KeepWindowShown
        });
        window.show()?;
        let (commands, receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("clippy-scrolling-capture".into())
            .spawn(move || {
                let result = with_region_reader(region, |read| {
                    let mut session = LongshotSession::start(first)?;
                    send(Message::Feedback(
                        id,
                        feedback(&session, "Ready for the next frame".into())?,
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
                            Command::Cancel => break,
                        };
                        send(Message::Feedback(id, feedback(&session, status)?));
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
        })
    }
}

fn feedback(session: &LongshotSession, note: String) -> Result<Feedback> {
    let state = session.snapshot();
    let preview = session.preview_pixels()?;
    Ok(Feedback {
        preview,
        frames: state.frame_count,
        status: format!(
            "{} × {} · {} frames\n{note}",
            state.width, state.total_height, state.frame_count
        ),
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
        let theme = settings::theme_index(&self.config.theme);
        match result
            .and_then(|(region, first)| View::open(self.capture_task_id, region, first, theme))
        {
            Ok(view) => self.longshot = Some(view),
            Err(error) => self.capture_error(error),
        }
    }

    pub(super) fn longshot_message(&mut self, message: Message) {
        let Some(view) = self.longshot.as_mut() else {
            return;
        };
        match message {
            Message::Action(Command::Cancel) => {
                self.longshot.take();
                self.set_busy(false);
            }
            Message::Action(command) if !view.busy => {
                view.busy = true;
                view.window.set_busy(true);
                let id = view.id;
                let sender = view.commands.clone();
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
            Message::Feedback(id, feedback) if id == view.id => {
                view.window.set_preview(editor::to_slint(&feedback.preview));
                view.busy = false;
                view.window.set_busy(false);
                view.window.set_can_undo(feedback.frames > 1);
                view.window.set_status(feedback.status.into());
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
            Message::Failed(id, error) if id == view.id => {
                self.longshot.take();
                self.capture_error(error);
            }
            _ => {}
        }
    }
}
