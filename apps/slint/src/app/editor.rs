use super::{capture, clipboard, post, settings, use_app, vision, App, Event};
use crate::{locale::tr, ClippyTheme, EditorWindow};
use anyhow::{anyhow, Result};
use clippy_capture::editor::{Editor, ImageProject, Tool};
use clippy_core::image_output::SaveTarget;
use image::RgbaImage;
use slint::{
    CloseRequestResponse, ComponentHandle, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel,
};
use std::{sync::Arc, thread};

const COLORS: [&str; 6] = [
    "#ff3b30", "#ff9500", "#34c759", "#0a84ff", "#ffffff", "#171717",
];
pub(super) const MAX_STROKE_POINTS: usize = 4_096;

fn tool_labels() -> ModelRc<slint::SharedString> {
    ModelRc::new(VecModel::from(
        Tool::ALL
            .into_iter()
            .map(|tool| tr(tool.label()).into())
            .chain(std::iter::once(tr("Pick color").into()))
            .collect::<Vec<_>>(),
    ))
}

pub(super) enum Action {
    Undo,
    Redo,
    Adjust(bool, f32),
    Copy,
    Save,
    Close,
    Pin,
    Analyze(vision::Mode),
    Sample([f32; 2]),
}

pub(super) struct View {
    id: u64,
    window: EditorWindow,
    document: Editor,
    current: Arc<RgbaImage>,
    stroke: Vec<[f32; 2]>,
    busy: bool,
}

impl View {
    pub(super) fn refresh_language(&self) {
        self.window.set_tools(tool_labels());
    }

    fn open(
        id: u64,
        image: Arc<RgbaImage>,
        document: Editor,
        theme: i32,
        browse: bool,
    ) -> Result<Self> {
        let window = EditorWindow::new()?;
        window.set_browse(browse);
        let appearance = window.global::<ClippyTheme>();
        appearance.set_index(theme);
        settings::connect_window_frame(&window, appearance);
        let (grayscale, corners) = document.grayscale_and_corners();
        window.set_grayscale_value(grayscale);
        window.set_corner_value(corners);
        window.set_can_undo(document.has_annotations());
        window.set_can_redo(document.has_redo());
        window.set_tools(tool_labels());
        window.set_text_tool_index(
            Tool::ALL
                .iter()
                .position(|tool| *tool == Tool::Text)
                .expect("文字工具可用") as i32,
        );
        window.set_aspect(image.width() as f32 / image.height() as f32);
        window.set_picture(to_slint(&image));
        window.on_start_stroke(|x, y| {
            edit_sync(|editor| {
                editor.stroke.clear();
                editor.stroke.push([x, y]);
            })
        });
        window.on_move_stroke(|x, y| {
            edit_sync(|editor| {
                if !editor.stroke.is_empty() && editor.stroke.len() < MAX_STROKE_POINTS {
                    editor.stroke.push([x, y]);
                }
            })
        });
        window.on_finish_stroke(|x, y, tool, color, size, text| {
            edit_sync(|editor| {
                if editor.stroke.is_empty() {
                    return;
                }
                let mut points = std::mem::take(&mut editor.stroke);
                points.push([x, y]);
                if tool as usize == Tool::ALL.len() {
                    action(Action::Sample([x, y]));
                    return;
                }
                let Some(tool) = Tool::ALL.get(tool as usize) else {
                    return;
                };
                let Some(color) = COLORS.get(color as usize) else {
                    return;
                };
                match editor
                    .document
                    .annotate(*tool, &points, color, size, text.as_str())
                {
                    Ok(true) => editor.render(),
                    Ok(false) => {}
                    Err(error) => editor.set_status(error),
                }
            })
        });
        window.on_undo(|| action(Action::Undo));
        window.on_redo(|| action(Action::Redo));
        window.on_adjustments(|grayscale, corner_radius| {
            action(Action::Adjust(grayscale, corner_radius))
        });
        window.on_copy(|| action(Action::Copy));
        window.on_save(|| action(Action::Save));
        window.on_pin(|| action(Action::Pin));
        window.on_recognize(|| action(Action::Analyze(vision::Mode::Recognize)));
        window.on_scan_code(|| action(Action::Analyze(vision::Mode::Scan)));
        window.on_close_editor(|| action(Action::Close));
        window.window().on_close_requested(|| {
            action(Action::Close);
            CloseRequestResponse::KeepWindowShown
        });
        window.show()?;
        Ok(Self {
            id,
            window,
            document,
            current: image,
            stroke: Vec::new(),
            busy: false,
        })
    }

    pub(super) fn show(&self) -> Result<()> {
        Ok(self.window.show()?)
    }

    fn set_status(&self, status: String) {
        self.window.set_status(status.into());
    }

    fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
        self.window.set_busy(busy);
        self.window.set_can_undo(self.document.has_annotations());
        self.window.set_can_redo(self.document.has_redo());
    }

    fn render(&mut self) {
        let project = self.document.snapshot();
        let id = self.id;
        self.set_busy(true);
        self.window.set_output_ready(false);
        self.set_status("Rendering…".to_owned());
        if let Err(error) = thread::Builder::new()
            .name("clippy-render".into())
            .spawn(move || {
                let result = project.render().map_err(anyhow::Error::msg);
                post(Event::EditorRendered(id, result));
            })
        {
            self.rendered(Err(error.into()));
        }
    }

    fn rendered(&mut self, result: Result<RgbaImage>) {
        match result {
            Ok(image) => {
                self.window.set_picture(to_slint(&image));
                self.current = Arc::new(image);
                self.set_status(String::new());
                self.window.set_output_ready(true);
            }
            Err(error) => self.set_status(format!("Could not render image: {error}")),
        }
        self.set_busy(false);
    }
}

impl Drop for View {
    fn drop(&mut self) {
        let _ = self.window.hide();
    }
}

pub(super) fn to_slint(image: &RgbaImage) -> slint::Image {
    let pixels = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
        image.as_raw(),
        image.width(),
        image.height(),
    );
    slint::Image::from_rgba8(pixels)
}

fn action(action: Action) {
    post(Event::EditorAction(action));
}

fn edit_sync(edit: impl FnOnce(&mut View)) {
    use_app(|app| {
        if let Some(editor) = app.editor.as_mut().filter(|editor| !editor.busy) {
            edit(editor);
        }
    });
}

impl App {
    pub(super) fn open_editor(&mut self, image: RgbaImage) -> Result<()> {
        let image = Arc::new(image);
        self.show_editor(image.clone(), Editor::new(image), false)
    }

    pub(super) fn open_editor_project(
        &mut self,
        image: Arc<RgbaImage>,
        project: ImageProject,
    ) -> Result<()> {
        self.show_editor(
            image,
            Editor::from_project(project).map_err(anyhow::Error::msg)?,
            false,
        )
    }

    pub(super) fn open_viewer_project(
        &mut self,
        image: Arc<RgbaImage>,
        project: ImageProject,
    ) -> Result<()> {
        anyhow::ensure!(
            self.editor.is_none(),
            "Close the current image before opening another one"
        );
        self.show_editor(
            image,
            Editor::from_project(project).map_err(anyhow::Error::msg)?,
            true,
        )
    }

    fn show_editor(&mut self, image: Arc<RgbaImage>, document: Editor, browse: bool) -> Result<()> {
        if let Some(view) = self.editor.as_ref() {
            return view.show();
        }
        self.editor_id += 1;
        self.editor = Some(View::open(
            self.editor_id,
            image,
            document,
            settings::theme_index(&self.config.theme),
            browse,
        )?);
        Ok(())
    }

    pub(super) fn editor_rendered(&mut self, id: u64, result: Result<RgbaImage>) {
        if let Some(editor) = self.editor.as_mut().filter(|editor| editor.id == id) {
            editor.rendered(result);
        }
    }

    pub(super) fn editor_output(&mut self, id: u64, result: Result<String>) {
        if let Some(editor) = self.editor.as_mut().filter(|editor| editor.id == id) {
            editor
                .set_status(result.unwrap_or_else(|error| format!("Image output failed: {error}")));
            editor.set_busy(false);
        }
    }

    pub(super) fn edit(&mut self, action: Action) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        // 保持每个编辑窗口至多一个工作任务，输出未提交时窗口继续承载它的反馈。
        if editor.busy {
            return;
        }
        match action {
            Action::Close => {
                self.editor.take();
            }
            Action::Undo => {
                if editor.document.undo() {
                    editor.render();
                }
            }
            Action::Redo => {
                if editor.document.redo() {
                    editor.render();
                }
            }
            Action::Adjust(grayscale, corners) => {
                editor
                    .document
                    .change_grayscale_and_corners(grayscale, corners);
                editor.render();
            }
            Action::Copy => {
                if !editor.window.get_output_ready() {
                    return;
                }
                let id = editor.id;
                let result = self.clipboard.submit(clipboard::Request::WriteImage {
                    image: editor.current.clone(),
                    finish: Box::new(move |result| {
                        post(Event::EditorOutput(
                            id,
                            result.map(|()| "Image copied".to_owned()),
                        ))
                    }),
                });
                match result {
                    Ok(()) => editor.set_busy(true),
                    Err(error) => editor.set_status(error.to_string()),
                }
            }
            Action::Save => {
                if !editor.window.get_output_ready() {
                    return;
                }
                let target = SaveTarget::from_config(
                    &self.config,
                    &self.paths.screenshot_default(self.identity),
                );
                let id = editor.id;
                let image = editor.current.clone();
                editor.set_busy(true);
                editor.set_status("Saving…".to_owned());
                if let Err(error) = thread::Builder::new()
                    .name("clippy-image-save".into())
                    .spawn(move || {
                        let result = capture::save_png(&image, &target)
                            .map(|path| format!("Saved to {}", path.display()));
                        post(Event::EditorOutput(id, result));
                    })
                {
                    self.editor_output(id, Err(anyhow!(error)));
                }
            }
            Action::Pin => {
                if editor.window.get_output_ready() {
                    let snapshot = editor.document.snapshot();
                    let image = editor.current.clone();
                    let result = self.pins.create(
                        super::pins::PinRequest::Editor(editor.id),
                        snapshot,
                        image,
                        self.storage.client(),
                    );
                    editor.set_status(match result {
                        Ok(()) => "Adding to the pinned workspace…".into(),
                        Err(error) => error.to_string(),
                    });
                }
            }
            Action::Analyze(mode) => {
                if editor.window.get_output_ready() {
                    let image = editor.current.clone();
                    let id = editor.id;
                    if let Err(error) = self.analyze_image(image, mode) {
                        self.editor_output(id, Err(error));
                    }
                }
            }
            Action::Sample([x, y]) => {
                if !editor.window.get_output_ready() {
                    return;
                }
                let (width, height) = editor.current.dimensions();
                let pixel = editor.current.get_pixel(
                    ((x * width as f32) as u32).min(width - 1),
                    ((y * height as f32) as u32).min(height - 1),
                );
                let color = format!("#{:02X}{:02X}{:02X}", pixel[0], pixel[1], pixel[2]);
                let id = editor.id;
                let result = self.clipboard.submit(clipboard::Request::WriteText {
                    text: color.clone(),
                    finish: Box::new(move |result| {
                        post(Event::EditorOutput(
                            id,
                            result.map(|()| format!("Copied {color}")),
                        ));
                    }),
                });
                match result {
                    Ok(()) => editor.set_busy(true),
                    Err(error) => editor.set_status(error.to_string()),
                }
            }
        }
    }
}
