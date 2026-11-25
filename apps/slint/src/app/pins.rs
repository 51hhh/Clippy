use super::{capture, clipboard, editor, post, settings, App, Event};
use crate::{ClippyTheme, PinnedWindow};
use anyhow::{anyhow, Context, Result};
use clippy_capture::{
    editor::{Editor, ImageProject},
    png_io::encode_png,
    project::{self, ProjectDocument},
    render_v2::RENDERER_VERSION,
};
use clippy_core::{
    clipboard::{compute_hash, strip_html_tags},
    image_output::SaveTarget,
    models::ContentType,
};
use clippy_storage::{
    BoundedImageData, ImageRevisionWrite, PinWorkspaceGroup, PinWorkspaceItemWrite,
    PinWorkspacePresentation, StorageClient, StorageEngine, StorageError, StoredImageRevision,
    StoredPinPlacement, StoredPinWorkspaceContent, StoredPinWorkspaceItem,
    StoredPinWorkspaceSummary,
};
use image::RgbaImage;
use slint::winit_030::{winit::window::WindowLevel, WinitWindowAccessor};
use slint::{CloseRequestResponse, ComponentHandle, LogicalSize};
use std::{collections::HashMap, ops::Range, sync::Arc, thread};

mod workspace;
use workspace::Workspace;

type Library = (Vec<PinWorkspaceGroup>, Vec<StoredPinWorkspaceSummary>);

pub(super) enum Message {
    Workspace,
    CloseWorkspace,
    Library(Result<Library, StorageError>),
    Filter(String),
    CreateGroup(String),
    RenameGroup(i64, String),
    DeleteGroup(i64),
    Viewport(Range<usize>),
    Thumbnail(i64, Result<Option<RgbaImage>>),
    Move(i64, Option<i64>),
    Remove(i64),
    Updated(Result<(), StorageError>),
    Open(i64),
    Ready(Result<Prepared>),
    Created(PinRequest, Result<Prepared>),
    PinClip(i64),
    ClipReady(i64, Result<PinInput, StorageError>),
    History(i64, ImageDestination),
    HistoryReady(ImageDestination, Result<(Arc<RgbaImage>, ImageProject)>),
    View(i64, Action),
    Status(i64, Result<String>),
}

#[derive(Clone, Copy)]
pub(super) enum PinRequest {
    Editor(u64),
    Launcher(u64),
    History,
}

#[derive(Clone, Copy)]
pub(super) enum ImageDestination {
    Editor,
    Viewer,
    Pin,
}

pub(super) enum PinInput {
    Image,
    Text(Box<Prepared>),
}

pub(super) enum Action {
    Edit,
    Copy,
    Save,
    Update,
    Resize(f32),
    Close,
}

enum Content {
    Image {
        project: ImageProject,
        rendered: Arc<RgbaImage>,
    },
    Text(String),
}

pub(super) struct Prepared {
    id: i64,
    content: Content,
    presentation: PinWorkspacePresentation,
    viewport: (f64, f64),
}

struct View {
    window: PinnedWindow,
    content: Content,
    viewport: (f64, f64),
}

impl Drop for View {
    fn drop(&mut self) {
        let _ = self.window.hide();
    }
}

#[derive(Default)]
pub(super) struct Pins {
    workspace: Option<Workspace>,
    views: HashMap<i64, View>,
    writing: bool,
    thumbnail_pending: bool,
}

fn send(message: Message) {
    post(Event::Pins(message));
}

fn parse_id(id: &slint::SharedString) -> i64 {
    id.parse().expect("工作区模型持有数据库 ID")
}

impl Pins {
    pub(super) fn create(
        &mut self,
        request: PinRequest,
        project: ImageProject,
        rendered: Arc<RgbaImage>,
        storage: StorageClient,
    ) -> Result<()> {
        anyhow::ensure!(!self.writing, "A pin is being saved");
        thread::Builder::new()
            .name("clippy-pin-output".into())
            .spawn(move || {
                let encoded = encode_pin(&project, &rendered);
                match encoded {
                    Ok(encoded) => {
                        let presentation = new_presentation();
                        let viewport = (
                            f64::from(project.source.width()),
                            f64::from(project.source.height()),
                        );
                        let result = storage.execute(
                            move |db| store_pin(db, &encoded, &presentation, viewport),
                            move |result| {
                                send(Message::Created(
                                    request,
                                    result
                                        .map(|id| Prepared {
                                            id,
                                            content: Content::Image { project, rendered },
                                            presentation: new_presentation(),
                                            viewport,
                                        })
                                        .map_err(Into::into),
                                ))
                            },
                        );
                        if let Err(error) = result {
                            send(Message::Created(request, Err(error.into())));
                        }
                    }
                    Err(error) => send(Message::Created(request, Err(error))),
                }
            })?;
        self.writing = true;
        Ok(())
    }

    fn workspace(&mut self, theme: i32) -> Result<()> {
        if let Some(workspace) = self.workspace.as_ref() {
            workspace.window.global::<ClippyTheme>().set_index(theme);
            return Ok(workspace.window.show()?);
        }
        self.workspace = Some(Workspace::open(theme)?);
        Ok(())
    }

    fn report(&self, error: impl std::fmt::Display) {
        if let Some(workspace) = self.workspace.as_ref() {
            workspace.window.set_status(error.to_string().into());
        } else {
            eprintln!("{error}");
        }
    }

    fn show(&mut self, prepared: Prepared, theme: i32) -> Result<()> {
        let id = prepared.id;
        if let Some(view) = self.views.get(&id) {
            return Ok(view.window.show()?);
        }
        let window = PinnedWindow::new()?;
        window.global::<ClippyTheme>().set_index(theme);
        match &prepared.content {
            Content::Image { rendered, .. } => {
                window.set_picture(editor::to_slint(rendered));
                window.set_has_image(true);
            }
            Content::Text(text) => window.set_body(text.clone().into()),
        }
        let state = prepared.presentation;
        window.set_zoom(state.scale as f32);
        window.set_opacity_value(state.opacity as f32);
        window.set_locked(state.locked);
        window.set_above(state.above);
        resize(&window, prepared.viewport, state.scale);
        window.on_update_pin(move || send(Message::View(id, Action::Update)));
        window.on_resize_pin(move |scale| send(Message::View(id, Action::Resize(scale))));
        window.on_edit_pin(move || send(Message::View(id, Action::Edit)));
        window.on_copy_pin(move || send(Message::View(id, Action::Copy)));
        window.on_save_pin(move || send(Message::View(id, Action::Save)));
        window.on_workspace(|| send(Message::Workspace));
        window.on_close_pin(move || send(Message::View(id, Action::Close)));
        settings::connect_titlebar(&window, PinnedWindow::invoke_drag_contains);
        window.window().on_close_requested(move || {
            send(Message::View(id, Action::Close));
            CloseRequestResponse::KeepWindowShown
        });
        window.show()?;
        window.window().with_winit_window(|native| {
            if let Some(saved) = state.placement.as_ref() {
                if let Some(monitor) = native
                    .available_monitors()
                    .find(|item| item.name() == saved.display_name)
                {
                    let factor = monitor.scale_factor();
                    native.set_outer_position(slint::winit_030::winit::dpi::PhysicalPosition::new(
                        (saved.x * factor).round() as i32,
                        (saved.y * factor).round() as i32,
                    ));
                }
            }
        });
        apply_presentation(&window);
        self.views.insert(
            id,
            View {
                window,
                content: prepared.content,
                viewport: prepared.viewport,
            },
        );
        Ok(())
    }
}

struct EncodedPin {
    source_png: Vec<u8>,
    source_hash: String,
    rendered_hash: String,
    document_hash: String,
    dimensions: (u32, u32),
    annotations_json: String,
    adjustments_json: String,
}

fn encode_pin(project: &ImageProject, rendered: &RgbaImage) -> Result<EncodedPin> {
    let dimensions = project.source.dimensions();
    let source_png = encode_png(project.source.as_raw(), dimensions.0, dimensions.1)?;
    let rendered_bytes = encode_png(rendered.as_raw(), rendered.width(), rendered.height())?;
    let annotations_json = project.document.annotations.to_string();
    let adjustments_json = project.document.adjustments.to_string();
    Ok(EncodedPin {
        source_hash: compute_hash(&source_png),
        source_png,
        rendered_hash: compute_hash(&rendered_bytes),
        document_hash: project::revision_document_hash(
            RENDERER_VERSION,
            dimensions,
            &annotations_json,
            &adjustments_json,
        ),
        dimensions,
        annotations_json,
        adjustments_json,
    })
}

fn new_presentation() -> PinWorkspacePresentation {
    PinWorkspacePresentation {
        scale: 1.0,
        opacity: 1.0,
        locked: false,
        above: true,
        placement: None,
    }
}

fn store_clip_pin(db: &StorageEngine, id: i64) -> Result<PinInput, StorageError> {
    let clip = db.get_clip_detail_by_id(id)?;
    if clip.is_sensitive {
        return Err(StorageError::PinWorkspaceInvariant(
            "Sensitive clipboard items cannot be pinned".into(),
        ));
    }
    if clip.content_type == ContentType::Image {
        return Ok(PinInput::Image);
    }
    let presentation = new_presentation();
    let viewport = (370.0, 240.0);
    let pin = db.upsert_pin_workspace_item(PinWorkspaceItemWrite {
        id: None,
        group_id: None,
        revision_id: None,
        content_type: clip.content_type,
        text_content: clip.text_content.as_deref(),
        html_content: clip.html_content.as_deref(),
        content_hash: &clip.content_hash,
        content_width: viewport.0,
        content_height: viewport.1,
        scale: presentation.scale,
        opacity: presentation.opacity,
        locked: presentation.locked,
        above: presentation.above,
        placement: None,
    })?;
    let text = clip
        .text_content
        .unwrap_or_else(|| strip_html_tags(clip.html_content.as_deref().unwrap_or_default()));
    Ok(PinInput::Text(Box::new(Prepared {
        id: pin,
        content: Content::Text(text),
        presentation,
        viewport,
    })))
}

fn store_pin(
    db: &StorageEngine,
    data: &EncodedPin,
    state: &PinWorkspacePresentation,
    size: (f64, f64),
) -> Result<i64, StorageError> {
    let revision = db.register_image_revision(ImageRevisionWrite {
        source_png: &data.source_png,
        source_hash: &data.source_hash,
        source_width: data.dimensions.0,
        source_height: data.dimensions.1,
        renderer_version: RENDERER_VERSION,
        annotations_json: &data.annotations_json,
        adjustments_json: &data.adjustments_json,
        document_hash: &data.document_hash,
        rendered_hash: &data.rendered_hash,
        root_clip_id: None,
    })?;
    db.upsert_pin_workspace_item(PinWorkspaceItemWrite {
        id: None,
        group_id: None,
        revision_id: Some(revision),
        content_type: ContentType::Image,
        text_content: None,
        html_content: None,
        content_hash: &data.rendered_hash,
        content_width: size.0,
        content_height: size.1,
        scale: state.scale,
        opacity: state.opacity,
        locked: state.locked,
        above: state.above,
        placement: None,
    })
}

fn decode_revision(revision: StoredImageRevision) -> Result<(Arc<RgbaImage>, ImageProject)> {
    anyhow::ensure!(
        revision.renderer_version == RENDERER_VERSION,
        "Stored renderer version is unsupported"
    );
    let source = project::decode_png(
        &revision.source_png,
        project::MAX_SOURCE_PNG_BYTES,
        "Pin source",
    )
    .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        source.dimensions() == (revision.source_width, revision.source_height),
        "Pin source dimensions changed"
    );
    let project = ImageProject {
        source: Arc::new(source),
        document: ProjectDocument {
            annotations: revision.annotations,
            adjustments: revision.adjustments,
        },
    };
    let image = Arc::new(project.render().map_err(anyhow::Error::msg)?);
    Ok((image, project))
}

fn prepare(item: StoredPinWorkspaceItem) -> Result<Prepared> {
    let content = match item.content {
        StoredPinWorkspaceContent::Image { revision, .. } => {
            let (rendered, project) = decode_revision(revision)?;
            Content::Image { project, rendered }
        }
        StoredPinWorkspaceContent::Snapshot {
            text_content,
            html_content,
            ..
        } => Content::Text(
            text_content
                .unwrap_or_else(|| strip_html_tags(html_content.as_deref().unwrap_or_default())),
        ),
    };
    Ok(Prepared {
        id: item.id,
        content,
        viewport: (item.content_width, item.content_height),
        presentation: PinWorkspacePresentation {
            scale: item.scale,
            opacity: item.opacity,
            locked: item.locked,
            above: item.above,
            placement: item.placement,
        },
    })
}

fn resize(window: &PinnedWindow, viewport: (f64, f64), scale: f64) {
    let base = (760.0 / viewport.0).min(520.0 / viewport.1).min(1.0);
    let width = (viewport.0 * base * scale + 68.0).clamp(330.0, 1600.0);
    let (padding, min_height) = if window.get_has_image() {
        (72.0, 341.0)
    } else {
        (24.0, 281.0)
    };
    let height = (viewport.1 * base * scale + padding).clamp(min_height, 1200.0);
    window
        .window()
        .set_size(LogicalSize::new(width as f32, height as f32));
}

fn apply_presentation(window: &PinnedWindow) {
    window.window().with_winit_window(|native| {
        native.set_window_level(if window.get_above() {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        });
        native.set_resizable(!window.get_locked());
    });
}

fn presentation(window: &PinnedWindow) -> PinWorkspacePresentation {
    let placement = window
        .window()
        .with_winit_window(|native| {
            let monitor = native.current_monitor()?;
            let scale = monitor.scale_factor();
            let position = native.outer_position().ok()?.to_logical::<f64>(scale);
            let origin = monitor.position().to_logical::<f64>(scale);
            let size = monitor.size().to_logical::<f64>(scale);
            Some(StoredPinPlacement {
                x: position.x,
                y: position.y,
                display_name: monitor.name(),
                display_x: origin.x,
                display_y: origin.y,
                display_width: size.width,
                display_height: size.height,
                display_scale: scale,
            })
        })
        .flatten();
    PinWorkspacePresentation {
        scale: f64::from(window.get_zoom()),
        opacity: f64::from(window.get_opacity_value()).clamp(0.15, 1.0),
        above: window.get_above(),
        locked: window.get_locked(),
        placement,
    }
}

impl App {
    fn refresh_pins(&self) {
        if self.pins.workspace.is_some() {
            if let Err(error) = self.storage.client().execute(
                |db| Ok((db.list_pin_groups()?, db.list_pin_workspace_summaries()?)),
                |result| send(Message::Library(result)),
            ) {
                self.pins.report(error);
            }
        }
    }

    pub(super) fn pin_message(&mut self, message: Message) {
        let result = self.handle_pins(message);
        if let Err(error) = result {
            self.status(error.to_string());
            self.pins.report(error);
        }
    }

    fn handle_pins(&mut self, message: Message) -> Result<()> {
        let theme = settings::theme_index(&self.config.theme);
        match message {
            Message::Workspace => {
                self.pins.workspace(theme)?;
                self.refresh_pins();
            }
            Message::CloseWorkspace => {
                if let Some(workspace) = self.pins.workspace.take() {
                    workspace.window.hide()?;
                }
            }
            Message::Library(result) => {
                if let Some(workspace) = &mut self.pins.workspace {
                    workspace.update(result?, &self.pins.views);
                }
                self.request_pin_thumbnail()?;
            }
            Message::Filter(group) => {
                if let Some(workspace) = &mut self.pins.workspace {
                    workspace.filter(group.into());
                }
                self.request_pin_thumbnail()?;
            }
            Message::CreateGroup(name) => self.storage.client().execute(
                move |db| db.create_pin_group(&name).map(|_| ()),
                |result| send(Message::Updated(result)),
            )?,
            Message::RenameGroup(id, name) => self.storage.client().execute(
                move |db| db.rename_pin_group(id, &name).map(|_| ()),
                |result| send(Message::Updated(result)),
            )?,
            Message::DeleteGroup(id) => self.storage.client().execute(
                move |db| db.delete_pin_group(id).map(|_| ()),
                |result| send(Message::Updated(result)),
            )?,
            Message::Viewport(range) => {
                if let Some(workspace) = &mut self.pins.workspace {
                    workspace.viewport(range);
                }
                self.request_pin_thumbnail()?;
            }
            Message::Thumbnail(id, result) => {
                self.pins.thumbnail_pending = false;
                if let Some(workspace) = &mut self.pins.workspace {
                    let image = result.unwrap_or_else(|error| {
                        eprintln!("Pin thumbnail {}: {error}", id);
                        None
                    });
                    workspace.thumbnail_ready(id, image);
                }
                self.request_pin_thumbnail()?;
            }
            Message::Move(id, group) => self.storage.client().execute(
                move |db| db.set_pin_workspace_group(id, group).map(|_| ()),
                |result| send(Message::Updated(result)),
            )?,
            Message::Remove(id) => {
                self.storage.client().execute(
                    move |db| db.delete_pin_workspace_item(id).map(|_| ()),
                    |result| send(Message::Updated(result)),
                )?;
                self.pins.views.remove(&id);
            }
            Message::Updated(result) => {
                result?;
                self.refresh_pins();
            }
            Message::Open(id) => {
                if let Some(view) = self.pins.views.get(&id) {
                    return Ok(view.window.show()?);
                }
                self.storage.client().execute(
                    move |db| db.load_pin_workspace_item(id),
                    |result| {
                        if let Err(error) = thread::Builder::new()
                            .name("clippy-pin-restore".into())
                            .spawn(move || {
                                let result = result.map_err(anyhow::Error::from).and_then(|item| {
                                    prepare(item.context("Pin no longer exists")?)
                                });
                                send(Message::Ready(result));
                            })
                        {
                            send(Message::Ready(Err(error.into())));
                        }
                    },
                )?;
            }
            Message::Created(request, result) => {
                self.pins.writing = false;
                let result = result.and_then(|prepared| self.pins.show(prepared, theme));
                match request {
                    PinRequest::Editor(id) => self.editor_output(
                        id,
                        result.map(|()| "Pinned image added to workspace".to_owned()),
                    ),
                    PinRequest::Launcher(generation) => {
                        self.launcher_pin_created(generation, result)
                    }
                    PinRequest::History => result?,
                }
                self.refresh_pins();
            }
            Message::Ready(result) => {
                self.pins.show(result?, theme)?;
                self.refresh_pins();
            }
            Message::PinClip(id) => self.storage.client().execute(
                move |db| store_clip_pin(db, id),
                move |result| send(Message::ClipReady(id, result)),
            )?,
            Message::ClipReady(id, result) => match result? {
                PinInput::Text(pin) => {
                    self.pins.show(*pin, theme)?;
                    self.refresh_pins();
                }
                PinInput::Image => self.handle_pins(Message::History(id, ImageDestination::Pin))?,
            },
            Message::History(id, destination) => {
                self.storage.client().execute(
                    move |db| {
                        if let Some(revision) = db.get_image_revision_for_clip(id)? {
                            return Ok(Ok(revision));
                        }
                        Ok(Err(db.get_bounded_image_snapshot(
                            id,
                            project::MAX_SOURCE_PNG_BYTES,
                        )?))
                    },
                    move |result| {
                        if let Err(error) = thread::Builder::new()
                            .name("clippy-editor-import".into())
                            .spawn(move || {
                                let result =
                                    result.map_err(anyhow::Error::from).and_then(|snapshot| {
                                        match snapshot {
                                            Ok(revision) => decode_revision(revision),
                                            Err(Some(snapshot)) => {
                                                let BoundedImageData::Bytes(bytes) = snapshot.image
                                                else {
                                                    return Err(anyhow!(
                                                    "Clipboard image is unavailable or too large"
                                                ));
                                                };
                                                let source = Arc::new(
                                                    project::decode_png(
                                                        &bytes,
                                                        project::MAX_SOURCE_PNG_BYTES,
                                                        "Clipboard image",
                                                    )
                                                    .map_err(anyhow::Error::msg)?,
                                                );
                                                Ok((source.clone(), Editor::new(source).snapshot()))
                                            }
                                            Err(None) => {
                                                Err(anyhow!("Clipboard image no longer exists"))
                                            }
                                        }
                                    });
                                send(Message::HistoryReady(destination, result));
                            })
                        {
                            send(Message::HistoryReady(destination, Err(error.into())));
                        }
                    },
                )?;
            }
            Message::HistoryReady(destination, result) => {
                let (image, project) = result?;
                match destination {
                    ImageDestination::Editor => self.open_editor_project(image, project)?,
                    ImageDestination::Viewer => {
                        self.open_viewer_project(image, project)?;
                        self.hide();
                    }
                    ImageDestination::Pin => self.pins.create(
                        PinRequest::History,
                        project,
                        image,
                        self.storage.client(),
                    )?,
                }
            }
            Message::View(id, action) => self.pin_action(id, action)?,
            Message::Status(id, result) => {
                if let Some(view) = self.pins.views.get(&id) {
                    view.window
                        .set_status(result.unwrap_or_else(|error| error.to_string()).into());
                }
            }
        }
        Ok(())
    }

    fn request_pin_thumbnail(&mut self) -> Result<()> {
        if !self.pins.thumbnail_pending {
            if let Some(workspace) = &mut self.pins.workspace {
                self.pins.thumbnail_pending = workspace.request_thumbnail(self.storage.client())?;
            }
        }
        Ok(())
    }

    fn pin_action(&mut self, id: i64, action: Action) -> Result<()> {
        let Some(view) = self.pins.views.get(&id) else {
            return Ok(());
        };
        match action {
            Action::Edit => {
                if let Content::Image { rendered, project } = &view.content {
                    self.open_editor_project(rendered.clone(), project.clone())?;
                }
            }
            Action::Copy => {
                let finish = Box::new(move |result: Result<()>| {
                    send(Message::Status(id, result.map(|()| "Copied".into())))
                });
                self.clipboard.submit(match &view.content {
                    Content::Image { rendered, .. } => clipboard::Request::WriteImage {
                        image: rendered.clone(),
                        finish,
                    },
                    Content::Text(text) => clipboard::Request::WriteText {
                        text: text.clone(),
                        finish,
                    },
                })?;
            }
            Action::Save => {
                if let Content::Image { rendered, .. } = &view.content {
                    let rendered = rendered.clone();
                    let target = SaveTarget::from_config(
                        &self.config,
                        &self.paths.screenshot_default(self.identity),
                    );
                    thread::Builder::new()
                        .name("clippy-pin-save".into())
                        .spawn(move || {
                            send(Message::Status(
                                id,
                                capture::save_png(&rendered, &target)
                                    .map(|path| format!("Saved to {}", path.display())),
                            ));
                        })?;
                }
            }
            Action::Resize(scale) => {
                view.window.set_zoom(scale);
                resize(&view.window, view.viewport, f64::from(scale));
                self.save_pin_state(id, presentation(&view.window))?;
            }
            Action::Update => {
                apply_presentation(&view.window);
                self.save_pin_state(id, presentation(&view.window))?;
            }
            Action::Close => {
                self.save_pin_state(id, presentation(&view.window))?;
                self.pins.views.remove(&id);
                self.refresh_pins();
            }
        }
        Ok(())
    }

    fn save_pin_state(&self, id: i64, state: PinWorkspacePresentation) -> Result<()> {
        self.storage.client().execute(
            move |db| db.update_pin_workspace_presentation(id, &state).map(|_| ()),
            |result| send(Message::Updated(result)),
        )?;
        Ok(())
    }
}
