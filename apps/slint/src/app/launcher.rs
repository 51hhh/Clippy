use super::{capture, pins, post, settings, tools, vision, App, Event};
use crate::{
    locale::tr, AppWindow, ClipRow, ClippyTheme, LauncherAction, LauncherRow, LauncherWindow,
};
use anyhow::{bail, ensure, Context, Result};
use clippy_capture::{editor::Editor, project};
use clippy_core::image_output::SaveTarget;
use clippy_storage::{BoundedImageData, BoundedImageSnapshot};
use image::RgbaImage;
use slint::{
    winit_030::WinitWindowAccessor, CloseRequestResponse, ComponentHandle, Model, ModelRc, VecModel,
};
use std::{sync::Arc, thread};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImageSource {
    Latest,
    Selected(i64),
    Unavailable,
}

#[derive(Clone, Copy)]
struct Selection {
    image: ImageSource,
    text: Option<i64>,
}

impl Selection {
    const TRAY: Self = Self {
        image: ImageSource::Latest,
        text: None,
    };

    fn history(row: Option<ClipRow>) -> Self {
        let Some(row) = row.filter(|row| !row.sensitive) else {
            return Self {
                image: ImageSource::Unavailable,
                text: None,
            };
        };
        let id = row
            .id
            .parse()
            .expect("The clipboard model carries database IDs");
        if row.kind == "Image" {
            Self {
                image: ImageSource::Selected(id),
                text: None,
            }
        } else {
            Self {
                image: ImageSource::Unavailable,
                text: Some(id),
            }
        }
    }

    fn source_title(self) -> Option<&'static str> {
        match self.image {
            ImageSource::Latest => Some("Latest clipboard image"),
            ImageSource::Selected(_) => Some("Selected clipboard image"),
            ImageSource::Unavailable if self.text.is_some() => Some("Selected clipboard text"),
            ImageSource::Unavailable => None,
        }
    }
}

pub(super) enum Message {
    OpenFromTray,
    OpenFromHistory,
    Close,
    Search(String),
    Activate(LauncherAction),
    Source(u64, &'static str, Option<String>),
    Image(u64, LauncherAction, Result<RgbaImage>),
    Saved(u64, Result<String>),
}

struct View {
    window: LauncherWindow,
    selection: Selection,
    system_capture: bool,
}

#[derive(Default)]
pub(super) struct Launcher {
    view: Option<View>,
    generation: u64,
    image_pending: bool,
}

impl Launcher {
    pub(super) fn is_open(&self) -> bool {
        self.view.is_some()
    }

    pub(super) fn close(&mut self) {
        self.generation += 1;
        if let Some(view) = self.view.take() {
            let _ = view.window.hide();
        }
    }

    pub(super) fn refresh_language(&self) {
        if let Some(view) = &self.view {
            view.window
                .set_actions(ModelRc::new(VecModel::from(filtered_actions(
                    view.window.get_query().as_str(),
                    view.selection,
                    view.system_capture,
                ))));
            if !view.window.get_source_title().is_empty() {
                let title = view.selection.source_title().expect("已显示的来源有标题");
                view.window.set_source_title(tr(title).into());
            }
        }
    }

    fn current(&self, generation: u64) -> Option<&View> {
        self.view.as_ref().filter(|_| self.generation == generation)
    }

    fn open(&mut self, selection: Selection, theme: i32) -> Result<()> {
        if let Some(view) = &self.view {
            view.window.window().with_winit_window(|window| {
                window.set_minimized(false);
                window.focus_window();
            });
            return Ok(());
        }
        let window = LauncherWindow::new()?;
        window.global::<ClippyTheme>().set_index(theme);
        let system_capture = cfg!(target_os = "linux") && clippy_platform::session::is_wayland();
        window.set_actions(ModelRc::new(VecModel::from(filtered_actions(
            "",
            selection,
            system_capture,
        ))));
        window
            .on_search(|value| super::handle(Event::Launcher(Message::Search(value.to_string()))));
        window.on_launch(|action| send(Message::Activate(action)));
        window.on_dismiss(|| send(Message::Close));
        settings::connect_titlebar(&window, LauncherWindow::invoke_titlebar_contains);
        window.window().on_close_requested(|| {
            send(Message::Close);
            CloseRequestResponse::KeepWindowShown
        });
        window.show()?;
        self.view = Some(View {
            window,
            selection,
            system_capture,
        });
        Ok(())
    }

    fn status(&self, text: impl Into<slint::SharedString>, busy: bool) {
        if let Some(view) = &self.view {
            view.window.set_status(tr(text.into().as_str()).into());
            view.window.set_busy(busy);
        }
    }
}

struct ActionEntry {
    action: LauncherAction,
    label: &'static str,
    detail: &'static str,
    keywords: &'static str,
}

use LauncherAction::*;

const ACTIONS: &[ActionEntry] = &[
    ActionEntry {
        action: Capture,
        label: "Take screenshot",
        detail: "Freeze all displays and select an area",
        keywords: "capture region crop 截图",
    },
    ActionEntry {
        action: Recognize,
        label: "Recognize image text",
        detail: "Read text and its position in an image",
        keywords: "ocr recognize image text 识字",
    },
    ActionEntry {
        action: PinImage,
        label: "Pin image",
        detail: "Keep a clipboard image on the desktop",
        keywords: "pin image window 贴图",
    },
    ActionEntry {
        action: SaveImage,
        label: "Save PNG",
        detail: "Save a clipboard image to the screenshot folder",
        keywords: "save image png export 保存",
    },
    ActionEntry {
        action: Scan,
        label: "Scan QR code or barcode",
        detail: "Decode codes in an image",
        keywords: "scan qr barcode image 扫码",
    },
    ActionEntry {
        action: EditImage,
        label: "Edit image",
        detail: "Open the clipboard image in the editor",
        keywords: "edit image annotate 编辑",
    },
    ActionEntry {
        action: SystemCapture,
        label: "System screenshot",
        detail: "Open the desktop screenshot dialog",
        keywords: "portal wayland 系统截图",
    },
    ActionEntry {
        action: TextTools,
        label: "Text tools",
        detail: "Copy, translate, format JSON or convert encodings",
        keywords: "text copy translation translate markdown json codec base64 翻译 文本",
    },
    ActionEntry {
        action: Workspace,
        label: "Pinned workspace",
        detail: "Find or restore saved pins",
        keywords: "pin workspace group 贴图工作区",
    },
    ActionEntry {
        action: Recordings,
        label: "Recordings",
        detail: "Play or export saved screen recordings",
        keywords: "recordings video library 录屏",
    },
    ActionEntry {
        action: Settings,
        label: "Settings",
        detail: "Configure shortcuts, themes and translation",
        keywords: "settings preferences 配置 设置",
    },
];

fn image_action(action: LauncherAction) -> bool {
    matches!(action, EditImage | Recognize | Scan | PinImage | SaveImage)
}

fn filtered_actions(query: &str, selection: Selection, system_capture: bool) -> Vec<LauncherRow> {
    let query = query.to_lowercase();
    ACTIONS
        .iter()
        .filter(|entry| {
            (entry.action != SystemCapture || system_capture)
                && (!image_action(entry.action) || selection.image != ImageSource::Unavailable)
                && query.split_whitespace().all(|word| {
                    entry.keywords.contains(word)
                        || entry.label.to_ascii_lowercase().contains(word)
                        || tr(entry.label).to_ascii_lowercase().contains(word)
                })
        })
        .map(|entry| LauncherRow {
            action: entry.action,
            label: tr(entry.label).into(),
            detail: tr(entry.detail).into(),
        })
        .collect()
}

fn send(message: Message) {
    post(Event::Launcher(message));
}

impl App {
    fn describe_launcher_source(&self, selection: Selection) {
        let Some(title) = selection.source_title() else {
            return;
        };
        let generation = self.launcher.generation;
        if let Err(error) = self.storage.client().execute(
            move |db| {
                let id = match selection.image {
                    ImageSource::Latest => db.get_latest_image_id()?,
                    ImageSource::Selected(id) => Some(id),
                    ImageSource::Unavailable => selection.text,
                };
                id.map(|id| db.get_clip_detail_by_id(id)).transpose()
            },
            move |result| {
                let detail = result
                    .ok()
                    .flatten()
                    .filter(|clip| !clip.is_sensitive)
                    .map(|clip| crate::clip_metadata(clip.byte_size, clip.created_at));
                send(Message::Source(generation, title, detail));
            },
        ) {
            eprintln!("Unable to describe the launcher selection: {error}");
        }
    }

    pub(super) fn launcher_message(&mut self, message: Message) {
        if let Err(error) = self.handle_launcher(message) {
            if self.launcher.is_open() {
                self.launcher.status(error.to_string(), false);
            } else {
                self.status(format!("Action unavailable: {error}"));
            }
        }
    }

    fn handle_launcher(&mut self, message: Message) -> Result<()> {
        match message {
            Message::OpenFromTray | Message::OpenFromHistory => {
                let already_open = self.launcher.is_open();
                let selection = if matches!(message, Message::OpenFromTray) {
                    Selection::TRAY
                } else {
                    let Some(window) = self
                        .window
                        .as_ref()
                        .filter(|window| !window.get_show_settings())
                    else {
                        return Ok(());
                    };
                    Selection::history(selected_row(window))
                };
                self.launcher
                    .open(selection, settings::theme_index(&self.config.theme))?;
                if !already_open {
                    self.describe_launcher_source(selection);
                }
            }
            Message::Source(generation, title, Some(detail)) => {
                if let Some(view) = self.launcher.current(generation) {
                    view.window.set_source_title(tr(title).into());
                    view.window.set_source_detail(detail.into());
                }
            }
            Message::Source(_, _, None) => {}
            Message::Close => {
                self.launcher.close();
                if let Some(window) = &self.window {
                    window
                        .window()
                        .with_winit_window(|window| window.focus_window());
                    window.set_history_focus_request(window.get_history_focus_request() + 1);
                }
            }
            Message::Search(query) => {
                if let Some(view) = &self.launcher.view {
                    view.window
                        .set_actions(ModelRc::new(VecModel::from(filtered_actions(
                            &query,
                            view.selection,
                            view.system_capture,
                        ))));
                }
            }
            Message::Activate(action) => self.launch(action)?,
            Message::Image(generation, action, result) => {
                self.launcher.image_pending = false;
                if self.launcher.current(generation).is_none() {
                    return Ok(());
                }
                let image = result?;
                match action {
                    EditImage => {
                        self.open_editor(image)?;
                        self.close_launcher_and_panel();
                    }
                    Recognize | Scan => {
                        let mode = if action == Recognize {
                            vision::Mode::Recognize
                        } else {
                            vision::Mode::Scan
                        };
                        self.analyze_image(Arc::new(image), mode)?;
                        self.close_launcher_and_panel();
                    }
                    PinImage => {
                        let image = Arc::new(image);
                        self.launcher
                            .status("Adding to the pinned workspace…", true);
                        self.pins.create(
                            pins::PinRequest::Launcher(generation),
                            Editor::new(image.clone()).snapshot(),
                            image,
                            self.storage.client(),
                        )?;
                    }
                    SaveImage => self.save_launcher_image(generation, image)?,
                    _ => unreachable!("Only image commands request pixels"),
                }
            }
            Message::Saved(generation, result) if self.launcher.current(generation).is_some() => {
                self.launcher.status(result?, false);
            }
            Message::Saved(..) => {}
        }
        Ok(())
    }

    fn launch(&mut self, action: LauncherAction) -> Result<()> {
        let Some(view) = &self.launcher.view else {
            return Ok(());
        };
        if view.window.get_busy() {
            return Ok(());
        }
        let selection = view.selection;
        match action {
            Capture => {
                ensure!(!self.busy, "Wait for the current action");
                self.start_capture();
            }
            SystemCapture =>
            {
                #[cfg(target_os = "linux")]
                if view.system_capture {
                    self.close_launcher_and_panel();
                    self.tools_message(tools::Message::Show);
                    self.tools_message(tools::Message::SystemScreenshot);
                }
            }
            EditImage | Recognize | Scan | PinImage | SaveImage => {
                self.load_launcher_image(action, selection.image)?;
            }
            TextTools => {
                self.close_launcher_and_panel();
                self.tools_message(
                    selection
                        .text
                        .map_or(tools::Message::Show, tools::Message::Preview),
                );
            }
            Workspace => {
                self.close_launcher_and_panel();
                self.pin_message(pins::Message::Workspace);
            }
            Recordings => {
                self.close_launcher_and_panel();
                self.recording_message(super::recording::Message::Show);
            }
            Settings => {
                self.close_launcher_and_panel();
                self.open(true)?;
            }
        }
        Ok(())
    }

    fn load_launcher_image(
        &mut self,
        action: LauncherAction,
        selection: ImageSource,
    ) -> Result<()> {
        ensure!(
            !self.launcher.image_pending,
            "Wait for the previous image to finish loading"
        );
        let generation = self.launcher.generation;
        self.storage.client().execute(
            move |db| {
                let id = match selection {
                    ImageSource::Latest => db.get_latest_image_id()?,
                    ImageSource::Selected(id) => Some(id),
                    ImageSource::Unavailable => None,
                };
                match id {
                    Some(id) => db.get_bounded_image_snapshot(id, project::MAX_SOURCE_PNG_BYTES),
                    None => Ok(None),
                }
            },
            move |result| {
                let input = result.map_err(anyhow::Error::from).and_then(image_bytes);
                match input {
                    Ok(bytes) => {
                        if let Err(error) = thread::Builder::new()
                            .name("clippy-launcher-image".into())
                            .spawn(move || {
                                let image = project::decode_png(
                                    &bytes,
                                    project::MAX_SOURCE_PNG_BYTES,
                                    "Clipboard image",
                                )
                                .map_err(anyhow::Error::msg);
                                send(Message::Image(generation, action, image));
                            })
                        {
                            send(Message::Image(generation, action, Err(error.into())));
                        }
                    }
                    Err(error) => send(Message::Image(generation, action, Err(error))),
                }
            },
        )?;
        self.launcher.image_pending = true;
        self.launcher.status("Loading clipboard image…", true);
        Ok(())
    }

    fn save_launcher_image(&self, generation: u64, image: RgbaImage) -> Result<()> {
        let target =
            SaveTarget::from_config(&self.config, &self.paths.screenshot_default(self.identity));
        self.launcher.status("Saving…", true);
        thread::Builder::new()
            .name("clippy-image-save".into())
            .spawn(move || {
                let result = capture::save_png(&image, &target)
                    .map(|path| format!("Saved to {}", path.display()));
                send(Message::Saved(generation, result));
            })?;
        Ok(())
    }

    pub(super) fn launcher_pin_created(&mut self, generation: u64, result: Result<()>) {
        if self.launcher.current(generation).is_some() {
            match result {
                Ok(()) => self.close_launcher_and_panel(),
                Err(error) => self.launcher.status(error.to_string(), false),
            }
        }
    }

    fn close_launcher_and_panel(&mut self) {
        self.launcher.close();
        self.hide();
    }
}

fn selected_row(window: &AppWindow) -> Option<ClipRow> {
    if window.get_loading_history() {
        return None;
    }
    let index = usize::try_from(window.get_selected_index()).ok()?;
    window
        .get_clips()
        .row_data(index)
        .filter(|row| row.id == window.get_selected_id())
}

fn image_bytes(snapshot: Option<BoundedImageSnapshot>) -> Result<Vec<u8>> {
    let snapshot = snapshot.context("No image in clipboard history")?;
    ensure!(
        !snapshot.is_sensitive,
        "Sensitive clipboard image is hidden"
    );
    match snapshot.image {
        BoundedImageData::Bytes(bytes) => Ok(bytes),
        BoundedImageData::TooLarge => bail!("Clipboard image is too large"),
        _ => bail!("Clipboard image is unavailable"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_search_by_keywords_and_respect_the_input_source() {
        let tray = filtered_actions("qR image", Selection::TRAY, false);
        assert_eq!(tray.len(), 1);
        assert_eq!(tray[0].action, Scan);
        assert!(filtered_actions("system screenshot", Selection::TRAY, false).is_empty());
        assert_eq!(
            filtered_actions("system screenshot", Selection::TRAY, true)[0].action,
            SystemCapture
        );
        let text = Selection::history(Some(ClipRow {
            id: "12".into(),
            kind: "Text".into(),
            ..Default::default()
        }));
        assert_eq!(text.text, Some(12));
        assert!(filtered_actions("ocr", text, false).is_empty());
        let image = Selection::history(Some(ClipRow {
            id: "24".into(),
            kind: "Image".into(),
            ..Default::default()
        }));
        assert!(image.image == ImageSource::Selected(24));
        assert_eq!(filtered_actions("ocr", image, false)[0].action, Recognize);
    }

    #[test]
    fn protected_selection_never_enters_an_action() {
        let selection = Selection::history(Some(ClipRow {
            id: "3".into(),
            kind: "Text".into(),
            sensitive: true,
            ..Default::default()
        }));
        assert!(selection.text.is_none());
        assert!(selection.image == ImageSource::Unavailable);
        let stored = BoundedImageSnapshot {
            content_hash: "fixture".into(),
            is_sensitive: true,
            image: BoundedImageData::Bytes(vec![1, 2]),
        };
        assert!(image_bytes(Some(stored))
            .unwrap_err()
            .to_string()
            .contains("Sensitive"));
    }
}
