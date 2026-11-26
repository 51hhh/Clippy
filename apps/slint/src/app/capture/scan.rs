use super::{capture_bounds, render_marks, Area, DesktopSnapshot, OutputMark, Selection, View};
use crate::{
    app::{clipboard, post, vision, App, Event},
    locale::tr,
    CaptureWindow, CodeMarker,
};
use anyhow::Result;
use clippy_platform::{capture, identity::DistributionChannel};
use clippy_vision::code_detection::{self, CodeScanResponse};
use slint::{ComponentHandle, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel, Weak};
use std::{cell::Cell, rc::Rc, thread, time::Instant};

pub(in crate::app) enum Message {
    Start(Ticket, Area),
    Finished(Ticket, Result<Decoded>),
    Copy(Ticket, String),
    Copied(Ticket, Result<()>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct Ticket {
    started: Instant,
    serial: u64,
    anchor: usize,
}

pub(super) struct Overlay {
    started: Instant,
    windows: Vec<Weak<CaptureWindow>>,
    serial: Cell<u64>,
    active: Cell<Option<Ticket>>,
}

impl Overlay {
    fn begin(&self, anchor: usize) -> Ticket {
        self.invalidate();
        let serial = self.serial.get() + 1;
        self.serial.set(serial);
        let ticket = Ticket {
            started: self.started,
            serial,
            anchor,
        };
        self.active.set(Some(ticket));
        ticket
    }

    pub(super) fn request(&self, anchor: usize, area: Area) {
        let ticket = self.begin(anchor);
        let window = self.windows[anchor].unwrap();
        window.set_scan_status(tr("Reading image…").into());
        window.set_scan_visible(true);
        window.set_scan_busy(true);
        send(Message::Start(ticket, area));
    }

    fn accepts(&self, ticket: Ticket) -> bool {
        self.active.get() == Some(ticket)
    }

    fn invalidate(&self) {
        if self.active.take().is_some() {
            for window in self.windows.iter().filter_map(Weak::upgrade) {
                window.set_scan_visible(false);
                window.set_scan_busy(false);
                window.set_scan_selected(-1);
                window.set_scan_matches(Default::default());
                window.set_scan_markers(Default::default());
                window.set_scan_limited(false);
            }
        }
    }

    fn select(&self, code: i32) {
        for window in self.windows.iter().filter_map(Weak::upgrade) {
            window.set_scan_selected(code);
        }
    }

    fn copy(&self, code: Option<i32>) {
        let ticket = self.active.get().expect("复制按钮属于当前扫码结果");
        let matches = self.windows[ticket.anchor].unwrap().get_scan_matches();
        let text = match code {
            Some(id) => {
                self.select(id);
                matches
                    .row_data(id as usize)
                    .expect("扫码项来自当前列表")
                    .text
                    .to_string()
            }
            None => matches
                .iter()
                .map(|item| item.text.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        };
        send(Message::Copy(ticket, text));
    }

    fn finish(&self, views: &[View], ticket: Ticket, result: Result<Decoded>) {
        let anchor = &views[ticket.anchor].window;
        anchor.set_scan_busy(false);
        let Decoded { codes, coordinates } = match result {
            Ok(decoded) => decoded,
            Err(error) => {
                anchor.set_scan_status(error.to_string().into());
                return;
            }
        };

        let mut markers = vec![Vec::new(); views.len()];
        let mut matches = Vec::with_capacity(codes.results.len());
        for (index, code) in codes.results.into_iter().enumerate() {
            for point in &code.points {
                if let Some((view, x, y)) = coordinates.locate(views, point.x, point.y) {
                    markers[view].push(CodeMarker {
                        id: index as i32,
                        x,
                        y,
                    });
                }
            }
            matches.push(vision::code_match(index, code));
        }
        for (view, markers) in views.iter().zip(markers) {
            view.window
                .set_scan_markers(ModelRc::new(VecModel::from(markers)));
        }
        anchor.set_scan_status(if matches.is_empty() {
            tr("No barcode found").into()
        } else {
            Default::default()
        });
        anchor.set_scan_limited(codes.limited);
        anchor.set_scan_matches(ModelRc::new(VecModel::from(matches)));
    }
}

pub(super) fn connect(views: &[View], started: Instant) -> Rc<Overlay> {
    let overlay = Rc::new(Overlay {
        started,
        windows: views.iter().map(|view| view.window.as_weak()).collect(),
        serial: Cell::new(0),
        active: Cell::new(None),
    });
    for (index, view) in views.iter().enumerate() {
        view.window.on_scan_selection({
            let overlay = overlay.clone();
            let source_size = (view.frame.pixels.width(), view.frame.pixels.height());
            let window = view.window.as_weak();
            move |left, top, right, bottom| {
                let bounds =
                    capture_bounds(&window.unwrap(), source_size, [left, top, right, bottom]);
                overlay.request(index, Area::Display { index, bounds });
            }
        });
        view.window.on_invalidate_scan({
            let overlay = overlay.clone();
            move || overlay.invalidate()
        });
        view.window.on_select_scan_code({
            let overlay = overlay.clone();
            move |id| overlay.select(id)
        });
        view.window.on_copy_scan_code({
            let overlay = overlay.clone();
            move |id| overlay.copy(Some(id))
        });
        view.window.on_copy_all_scan_codes({
            let overlay = overlay.clone();
            move || overlay.copy(None)
        });
    }
    overlay
}

enum Snapshot {
    Display {
        index: usize,
        bounds: [f32; 4],
        pixels: SharedPixelBuffer<Rgba8Pixel>,
        marks: Vec<OutputMark>,
    },
    Desktop(DesktopSnapshot),
}

impl Snapshot {
    fn new(selection: &Selection, area: Area) -> Self {
        match area {
            Area::Display { index, bounds } => {
                let view = &selection.views[index];
                Self::Display {
                    index,
                    bounds,
                    pixels: view.frame.pixels.clone(),
                    marks: view
                        .window
                        .get_marks()
                        .iter()
                        .map(OutputMark::from)
                        .collect(),
                }
            }
            Area::Desktop(composition) => {
                Self::Desktop(DesktopSnapshot::new(&selection.views, composition))
            }
        }
    }

    fn decode(self) -> Result<Decoded> {
        let (image, coordinates) = match self {
            Self::Display {
                index,
                bounds,
                pixels,
                marks,
            } => {
                let (width, height) = (pixels.width(), pixels.height());
                let [left, top, ..] = capture::pixel_rect(width, height, bounds)?;
                let selected = capture::crop_rgba(pixels.as_bytes(), width, height, bounds)?;
                drop(pixels);
                (
                    render_marks(selected, &marks)?,
                    Coordinates::Display {
                        index,
                        left,
                        top,
                        width,
                        height,
                    },
                )
            }
            Self::Desktop(snapshot) => {
                let composition = snapshot.composition;
                (snapshot.render(), Coordinates::Desktop(composition))
            }
        };
        let (width, height) = image.dimensions();
        let codes = code_detection::scan_rgba(image.into_raw(), width, height)?;
        Ok(Decoded { codes, coordinates })
    }
}

pub(in crate::app) struct Decoded {
    codes: CodeScanResponse,
    coordinates: Coordinates,
}

enum Coordinates {
    Display {
        index: usize,
        left: u32,
        top: u32,
        width: u32,
        height: u32,
    },
    Desktop(capture::desktop::Composition),
}

impl Coordinates {
    fn locate(&self, views: &[View], x: f32, y: f32) -> Option<(usize, f32, f32)> {
        match *self {
            Self::Display {
                index,
                left,
                top,
                width,
                height,
            } => Some((
                index,
                (left as f32 + x + 0.5) / width as f32,
                (top as f32 + y + 0.5) / height as f32,
            )),
            Self::Desktop(composition) => {
                let [x, y] = composition.desktop_point([x, y]);
                views.iter().enumerate().find_map(|(index, view)| {
                    let rect = view.frame.geometry;
                    let x = (x - f64::from(rect.x)) / f64::from(rect.width);
                    let y = (y - f64::from(rect.y)) / f64::from(rect.height);
                    ((0.0..1.0).contains(&x) && (0.0..1.0).contains(&y))
                        .then_some((index, x as f32, y as f32))
                })
            }
        }
    }
}

fn send(message: Message) {
    post(Event::CaptureScan(message));
}

impl App {
    pub(in crate::app) fn capture_scan_message(&mut self, message: Message) {
        let ticket = match &message {
            Message::Start(ticket, _)
            | Message::Finished(ticket, _)
            | Message::Copy(ticket, _)
            | Message::Copied(ticket, _) => *ticket,
        };
        let current = self
            .capture_selection
            .as_ref()
            .filter(|selection| selection.scan.accepts(ticket));
        match message {
            Message::Start(ticket, area) => {
                if let Some(selection) = current {
                    let result = self.vision.start_recognition(|_| {
                        let snapshot = Snapshot::new(selection, area);
                        thread::Builder::new()
                            .name("clippy-capture-scan".into())
                            .spawn(move || {
                                send(Message::Finished(ticket, snapshot.decode()));
                            })?;
                        Ok(())
                    });
                    if let Err(error) = result {
                        selection.scan.finish(&selection.views, ticket, Err(error));
                    }
                }
            }
            Message::Finished(ticket, result) => {
                self.vision.recognition_finished();
                if self.identity.channel == DistributionChannel::Qa {
                    eprintln!(
                        "capture_scan {} codes={:?}",
                        if current.is_some() {
                            "finished"
                        } else {
                            "discarded"
                        },
                        result
                            .as_ref()
                            .ok()
                            .map(|decoded| decoded.codes.results.len()),
                    );
                }
                if let Some(selection) = current {
                    selection.scan.finish(&selection.views, ticket, result);
                }
            }
            Message::Copy(ticket, text) => {
                // 用户已点击复制，关闭选区只撤销进度提示。
                if let Err(error) = self.clipboard.submit(clipboard::Request::WriteText {
                    text,
                    finish: Box::new(move |result| send(Message::Copied(ticket, result))),
                }) {
                    send(Message::Copied(ticket, Err(error)));
                }
            }
            Message::Copied(ticket, result) => {
                if let Some(selection) = current {
                    selection.views[ticket.anchor].window.set_scan_status(
                        result
                            .map(|()| tr("Copied to clipboard").to_owned())
                            .unwrap_or_else(|error| error.to_string())
                            .into(),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changing_the_selection_and_reopening_reject_late_decodes() {
        let started = Instant::now();
        let overlay = |started| Overlay {
            started,
            windows: Vec::new(),
            serial: Cell::new(0),
            active: Cell::new(None),
        };
        let first = overlay(started);
        let old = first.begin(0);
        assert!(first.accepts(old));
        first.invalidate();
        assert!(!first.accepts(old));
        let current = first.begin(0);
        assert!(first.accepts(current));
        assert!(!first.accepts(old));

        let reopened = overlay(started + std::time::Duration::from_millis(1));
        reopened.begin(0);
        assert!(!reopened.accepts(old));
        assert!(!reopened.accepts(current));
    }
}
