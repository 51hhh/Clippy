use crate::{build_label, clip_row, AppWindow, CaptureAction, ClippyTray, ShortcutField};
use anyhow::{Context, Result};
use clippy_core::{
    config::{parse_config_json, serialize_config_json, AppConfig},
    image_output::SaveTarget,
    models::ClipItem,
};
use clippy_platform::{
    capture::{FrozenCapture, Region, RegionCancellation, ScreenSharingCancelled},
    identity::{DeploymentIdentity, DistributionChannel},
    instance::Instance,
    paste::{capture_target, Target},
    paths::AppPaths,
    private_files::{commit_private_replacement, restrict_directory, write_private},
    shortcuts::{Action, Shortcuts},
};
use clippy_storage::{ClipListRequest, StorageError, StorageRuntime};
use slint::{
    winit_030::WinitWindowAccessor, CloseRequestResponse, ComponentHandle, ModelRc, SharedString,
    Timer, TimerMode, VecModel,
};
use std::{
    cell::RefCell,
    fs, io,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

mod capture;
mod clipboard;
mod editor;
mod launcher;
mod longshot;
mod pins;
mod preview;
mod preview_document;
mod recording;
mod settings;
mod thumbnails;
mod tools;
mod upgrade;
mod vision;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn run(identity: &'static DeploymentIdentity) -> Result<()> {
    let paths = AppPaths::resolve(identity)?;
    fs::create_dir_all(&paths.data_dir)?;
    restrict_directory(&paths.data_dir)?;
    let instance = Instance::acquire(
        &paths.data_dir,
        identity.current_platform().single_instance_key,
    )?;
    if matches!(instance, Instance::Forwarded) {
        return Ok(());
    }
    if clippy_platform::session::is_wayland() {
        slint::BackendSelector::new()
            .with_winit_custom_application_handler(capture::CaptureWindowEvents)
            .select()
            .context("Cannot initialize the Wayland event loop")?;
    }
    let config = load_config(&paths, identity)?;
    if identity.channel == DistributionChannel::Production {
        upgrade::backup_existing(&paths)?;
    }
    clippy_vision::set_manifest_setting(&config.enhanced_ocr_manifest_path);
    let storage = if config.storage_mode == "memory" {
        StorageRuntime::in_memory()?
    } else {
        StorageRuntime::open(&paths.database_file)?
    };
    let clipboard =
        clipboard::Worker::start(storage.client(), paths.data_dir.clone(), config.max_history)?;
    let tray = ClippyTray::new()?;
    tray.set_product_name(identity.product_name.into());
    tray.on_open_panel(|| handle(Event::Open(false)));
    tray.on_screenshot(|| handle(Event::CaptureStart));
    tray.on_delayed_screenshot(|| {
        use_app(|app| {
            app.status("Screenshot in 3 seconds");
            app.delayed_capture
                .start(TimerMode::SingleShot, Duration::from_secs(3), || {
                    post(Event::CaptureStart);
                });
        });
    });
    tray.on_save_screens(|| use_app(App::save_screens));
    tray.on_open_settings(|| handle(Event::Open(true)));
    tray.on_open_actions(|| handle(Event::Launcher(launcher::Message::OpenFromTray)));
    tray.on_open_workspace(|| handle(Event::Pins(pins::Message::Workspace)));
    tray.on_recordings(|| handle(Event::Recordings(recording::Message::Show)));
    tray.on_recording_pause(|| {
        handle(Event::Recordings(recording::Message::Control(
            recording::Control::PauseResume,
        )))
    });
    tray.on_recording_stop(|| {
        handle(Event::Recordings(recording::Message::Control(
            recording::Control::Stop,
        )))
    });
    tray.on_stop_scrolling(|| {
        handle(Event::Longshot(longshot::Message::Action(
            longshot::Action::Stop,
        )))
    });
    tray.on_quit(|| {
        let _ = slint::quit_event_loop();
    });

    let shortcuts = Shortcuts::start(
        identity.current_platform(),
        &config.global_shortcut,
        &config.pin_shortcut,
        &config.capture_shortcut,
        |action| post(Event::GlobalShortcut(action)),
        |message| post(Event::ShortcutStatus(message)),
    );
    let shortcut_status = shortcuts
        .as_ref()
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    APP.with(|app| {
        app.replace(Some(App {
            identity,
            config,
            paths,
            window: None,
            target: None,
            clipboard,
            storage,
            generation: 0,
            busy: false,
            #[cfg(target_os = "linux")]
            gnome_helper_busy: false,
            #[cfg(target_os = "linux")]
            gnome_settings_pending: None,
            shortcuts: shortcuts.ok(),
            shortcut_status,
            action_status: SharedString::default(),
            capture_selection: None,
            capture_after_close: None,
            capture_shutdown: RegionCancellation::default(),
            delayed_capture: Timer::default(),
            editor: None,
            editor_id: 0,
            pins: pins::Pins::default(),
            preview: preview::Preview::default(),
            thumbnails: thumbnails::Thumbnails::default(),
            tray: tray.clone_strong(),
            longshot: None,
            capture_task_id: 0,
            vision: vision::Vision::new(identity.channel),
            recordings: recording::Recordings::default(),
            tools: tools::Tools::default(),
            launcher: launcher::Launcher::default(),
        }))
    });
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(300), move || {
        if instance.take_activation().unwrap_or(false) {
            handle(Event::Open(false));
        }
    });
    let tray_result = tray.show();
    if tray_result.is_err() || std::env::args_os().skip(1).any(|arg| arg == "--open") {
        handle(Event::Open(false));
    }
    let result = slint::run_event_loop_until_quit();
    timer.stop();
    APP.with(|app| {
        if let Some(app) = app.borrow_mut().take() {
            app.capture_shutdown.cancel();
            app.vision.shutdown();
        }
    });
    result?;
    Ok(())
}

fn load_config(paths: &AppPaths, identity: &DeploymentIdentity) -> Result<AppConfig> {
    match fs::read_to_string(&paths.config_file) {
        Ok(json) => {
            let mut config = parse_config_json(&json).context("Invalid Clippy configuration")?;
            config.migrate();
            Ok(config)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut config = AppConfig {
                auto_paste: false,
                ..AppConfig::default()
            };
            if identity.channel == DistributionChannel::Qa {
                config.global_shortcut = "Ctrl+Alt+Shift+J".into();
                config.capture_shortcut = "Ctrl+Alt+Shift+K".into();
            }
            Ok(config)
        }
        Err(error) => Err(error.into()),
    }
}

enum Event {
    Open(bool),
    GlobalShortcut(Action),
    Hide,
    HistoryChanged,
    History(u64, Result<Vec<ClipItem>, StorageError>),
    ClipboardStatus(String),
    ShortcutStatus(String),
    Copied(Result<(Option<Target>, Option<u64>)>),
    Pasted(Result<()>),
    Permission(Result<()>),
    #[cfg(target_os = "linux")]
    PermissionForgotten(Result<()>),
    #[cfg(target_os = "linux")]
    TmuxCaptured(Result<String>),
    Settings(settings::Message),
    Tools(tools::Message),
    Launcher(launcher::Message),
    Preview(preview::Message),
    Thumbnails(thumbnails::Message),
    Changed(Result<(), StorageError>),
    CaptureStart,
    CaptureReady {
        started: Instant,
        result: Result<FrozenCapture>,
    },
    CaptureWindows {
        started: Instant,
        result: Result<Vec<Vec<clippy_platform::capture::window_pick::WindowTarget>>>,
    },
    CapturePlacementFailed(slint::Weak<crate::CaptureWindow>, anyhow::Error),
    CaptureWindowDestroyed(slint::winit_030::winit::window::WindowId, Instant),
    CaptureCancel,
    CaptureSelection {
        area: capture::Area,
        action: CaptureAction,
    },
    CaptureClipboard(Result<image::RgbaImage>, Instant),
    CaptureFinished(Result<String>, Instant),
    EditorOpen(Result<capture::EditedCapture>),
    EditorAction(editor::Action),
    EditorRendered(u64, Result<image::RgbaImage>),
    EditorOutput(u64, Result<String>),
    Pins(pins::Message),
    LongshotStart(Result<(Region, image::RgbaImage)>),
    Longshot(longshot::Message),
    Vision(vision::Message),
    RecordingStart(Result<(Region, (u32, u32))>),
    Recordings(recording::Message),
}

fn post(event: Event) {
    let _ = slint::invoke_from_event_loop(move || handle(event));
}

fn handle(event: Event) {
    use_app(|app| app.handle(event));
}

fn use_app(f: impl FnOnce(&mut App)) {
    APP.with(|state| {
        if let Some(app) = state.borrow_mut().as_mut() {
            f(app);
        }
    });
}

struct App {
    identity: &'static DeploymentIdentity,
    config: AppConfig,
    paths: AppPaths,
    window: Option<AppWindow>,
    target: Option<Target>,
    clipboard: clipboard::Worker,
    storage: StorageRuntime,
    generation: u64,
    busy: bool,
    #[cfg(target_os = "linux")]
    gnome_helper_busy: bool,
    #[cfg(target_os = "linux")]
    gnome_settings_pending: Option<Box<AppConfig>>,
    shortcuts: Option<Shortcuts>,
    shortcut_status: String,
    action_status: SharedString,
    capture_selection: Option<capture::Selection>,
    capture_after_close: Option<capture::PendingOutput>,
    capture_shutdown: RegionCancellation,
    delayed_capture: Timer,
    editor: Option<editor::View>,
    editor_id: u64,
    pins: pins::Pins,
    preview: preview::Preview,
    thumbnails: thumbnails::Thumbnails,
    tray: ClippyTray,
    longshot: Option<longshot::View>,
    capture_task_id: u64,
    vision: vision::Vision,
    recordings: recording::Recordings,
    tools: tools::Tools,
    launcher: launcher::Launcher,
}

impl App {
    fn handle(&mut self, event: Event) {
        match event {
            Event::GlobalShortcut(action)
                if !self.window.as_ref().is_some_and(|window| {
                    window.get_recording_shortcut() != ShortcutField::None
                }) =>
            {
                match action {
                    Action::Open => self.handle(Event::Open(false)),
                    Action::Pin => {
                        if let Some(window) = self.window.as_ref().filter(|window| {
                            !window.get_show_settings() && !window.get_loading_history()
                        }) {
                            if let Ok(id) = window.get_selected_id().parse() {
                                self.pin_message(pins::Message::PinClip(id));
                            }
                        }
                    }
                    Action::Capture => self.handle(Event::CaptureStart),
                }
            }
            Event::GlobalShortcut(_) => {}
            Event::Open(settings) if self.capture_selection.is_none() => {
                if let Err(error) = self.open(settings) {
                    eprintln!("Unable to open Clippy: {error}");
                }
            }
            Event::Open(_) => {}
            Event::Hide => self.hide(),
            Event::HistoryChanged => self.reload(),
            Event::History(generation, result) if generation == self.generation => {
                if let Some(window) = &self.window {
                    window.set_loading_history(false);
                    match result {
                        Ok(clips) => {
                            let selected = window.get_selected_id();
                            let rows = clips.into_iter().map(clip_row).collect::<Vec<_>>();
                            let count = rows.len();
                            let index = rows
                                .iter()
                                .position(|item| item.id == selected)
                                .unwrap_or(0);
                            window.set_selected_index(if count == 0 { -1 } else { index as i32 });
                            window.set_selected_id(
                                rows.get(index)
                                    .map(|item| item.id.clone())
                                    .unwrap_or_default(),
                            );
                            if window.get_favorites_only() {
                                window.set_favorite_count(count as i32);
                            } else {
                                window.set_all_count(count as i32);
                                window.set_favorite_count(
                                    rows.iter().filter(|item| item.favorite).count() as i32,
                                );
                            }
                            window.set_can_load_more(count == window.get_history_limit() as usize);
                            window.set_clips(ModelRc::new(VecModel::from(rows)));
                            window.set_history_status(match count {
                                0 => "No matching items".into(),
                                1 => "1 item".into(),
                                _ => format!("{count} items").into(),
                            });
                            self.refresh_thumbnails();
                        }
                        Err(error) => window
                            .set_history_status(format!("History unavailable: {error}").into()),
                    }
                }
            }
            Event::History(_, _) => {}
            Event::ClipboardStatus(message) => self.status(message),
            Event::ShortcutStatus(message) => {
                self.shortcut_status = message;
                if let Some(window) = &self.window {
                    window.set_shortcut_status(self.shortcut_status.clone().into());
                }
            }
            Event::Copied(Ok((Some(target), generation))) if self.config.auto_paste => {
                self.hide();
                if let Err(error) = self
                    .clipboard
                    .submit(clipboard::Request::Paste { target, generation })
                {
                    self.complete(format!("Copied. {error}"));
                }
            }
            Event::Copied(Ok(_)) => self.complete("Copied to clipboard"),
            Event::Copied(Err(error)) => self.complete(format!("Could not copy: {error}")),
            Event::Pasted(Ok(())) => self.complete("Pasted"),
            Event::Pasted(Err(error)) => {
                eprintln!("Copied. Auto paste unavailable: {error}");
                self.complete(format!("Copied. Auto paste unavailable: {error}"));
            }
            Event::Permission(result) => {
                self.complete(match result {
                    Ok(()) => "Paste permission is ready".to_owned(),
                    Err(error) => format!("Paste permission unavailable: {error}"),
                });
            }
            #[cfg(target_os = "linux")]
            Event::PermissionForgotten(result) => {
                self.complete(match result {
                    Ok(()) => "Saved paste permission removed".to_owned(),
                    Err(error) => error.to_string(),
                });
            }
            #[cfg(target_os = "linux")]
            Event::TmuxCaptured(result) => {
                let result = result.and_then(|text| {
                    self.clipboard.submit(clipboard::Request::WriteText {
                        text,
                        finish: Box::new(|result| {
                            post(Event::Copied(result.map(|()| (None, None))))
                        }),
                    })
                });
                if let Err(error) = result {
                    self.complete(format!("Could not capture tmux pane: {error}"));
                }
            }
            Event::Settings(message) => self.settings_message(message),
            Event::Tools(message) => self.tools_message(message),
            Event::Launcher(message) => self.launcher_message(message),
            Event::Preview(message) => self.preview_message(message),
            Event::Thumbnails(message) => self.thumbnails_message(message),
            Event::Changed(result) => {
                if let Err(error) = result {
                    self.status(format!("Could not update history: {error}"));
                }
                self.reload();
            }
            Event::CaptureStart => self.start_capture(),
            Event::CaptureReady { started, result } => {
                let diagnostic = self.identity.channel == DistributionChannel::Qa;
                if diagnostic {
                    eprintln!(
                        "capture_freeze elapsed_ms={:.1}",
                        started.elapsed().as_secs_f64() * 1000.0
                    );
                }
                match result.and_then(|frozen| capture::present(frozen, started, diagnostic)) {
                    Ok(selection) => self.capture_selection = Some(selection),
                    Err(error) if error.is::<ScreenSharingCancelled>() => {
                        self.complete("Screenshot cancelled");
                    }
                    Err(error) => self.capture_error(error),
                }
            }
            Event::CaptureWindows { started, result } => {
                if let Some(selection) = &self.capture_selection {
                    selection.install_window_targets(
                        started,
                        result,
                        self.identity.channel == DistributionChannel::Qa,
                    );
                }
            }
            Event::CapturePlacementFailed(window, error) => {
                if window.upgrade().is_some() {
                    self.capture_selection.take();
                    self.capture_error(error);
                }
            }
            Event::CaptureWindowDestroyed(id, closed) => {
                #[cfg(target_os = "linux")]
                self.longshot_window_destroyed(id, closed);
                if self
                    .capture_after_close
                    .as_ref()
                    .is_some_and(|pending| pending.window_id == id)
                {
                    if let Err(error) = self.capture_after_close.take().unwrap().start(closed) {
                        self.capture_error(error);
                    }
                }
            }
            Event::CaptureCancel => {
                if self.capture_selection.take().is_some() {
                    self.set_busy(false);
                }
            }
            Event::CaptureSelection { area, action } => {
                if let Some(selection) = self.capture_selection.take() {
                    use capture::{Destination, StillDestination};
                    let destination = match action {
                        CaptureAction::Copy => Destination::Still(StillDestination::Clipboard),
                        CaptureAction::Edit => Destination::Still(StillDestination::Editor),
                        CaptureAction::Longshot => Destination::Longshot,
                        CaptureAction::Record => Destination::Record,
                        CaptureAction::Save => {
                            Destination::Still(StillDestination::File(SaveTarget::from_config(
                                &self.config,
                                &self.paths.screenshot_default(self.identity),
                            )))
                        }
                    };
                    match capture::output(selection, area, destination) {
                        Ok(pending) => self.capture_after_close = pending,
                        Err(error) => self.capture_error(error.into()),
                    }
                }
            }
            Event::CaptureClipboard(result, started) => {
                let result = result.and_then(|image| {
                    self.clipboard.submit(clipboard::Request::WriteImage {
                        image: Arc::new(image),
                        finish: Box::new(move |result| {
                            post(Event::CaptureFinished(
                                result.map(|()| "Screenshot copied".to_owned()),
                                started,
                            ))
                        }),
                    })
                });
                if let Err(error) = result {
                    self.capture_error(error);
                }
            }
            Event::CaptureFinished(result, started) => match result {
                Ok(status) => {
                    if self.identity.channel == DistributionChannel::Qa {
                        eprintln!(
                            "capture_commit elapsed_ms={:.1}",
                            started.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                    self.complete(status);
                }
                Err(error) => self.capture_error(error),
            },
            Event::EditorOpen(result) => {
                self.set_busy(false);
                if let Err(error) =
                    result.and_then(|(image, project)| self.open_editor_project(image, project))
                {
                    self.capture_error(error);
                }
            }
            Event::EditorAction(action) => self.edit(action),
            Event::EditorRendered(id, result) => self.editor_rendered(id, result),
            Event::EditorOutput(id, result) => self.editor_output(id, result),
            Event::Pins(message) => self.pin_message(message),
            Event::LongshotStart(result) => self.start_longshot(result),
            Event::Longshot(message) => self.longshot_message(message),
            Event::Vision(message) => self.vision_message(message),
            Event::RecordingStart(input) => self.start_recording(input),
            Event::Recordings(message) => self.recording_message(message),
        }
    }

    fn open(&mut self, settings: bool) -> Result<()> {
        if let Some(window) = &self.window {
            if window.get_show_settings() == settings {
                if settings {
                    settings::show(window, self.identity);
                    window
                        .window()
                        .with_winit_window(|window| window.set_minimized(false));
                }
                return Ok(window.show()?);
            }
            self.hide();
        }

        self.target = self.config.auto_paste.then(capture_target).flatten();
        let window = AppWindow::new()?;
        window.set_product_name(self.identity.product_name.into());
        window.set_build_label(build_label(self.identity).into());
        window.set_auto_paste(self.config.auto_paste);
        window.set_show_settings(settings);
        window.set_busy(self.busy);
        #[cfg(target_os = "linux")]
        window.set_settings_save_pending(self.gnome_settings_pending.is_some());
        window.set_action_status(self.action_status.clone());
        window.set_shortcut_status(self.shortcut_status.clone().into());
        settings::connect(&window, &self.config);
        if settings {
            settings::show(&window, self.identity);
        }
        window.on_reload_history(|| use_app(App::reload));
        window.on_close_panel(|| post(Event::Hide));
        window.on_copy_item(|id, paste| use_app(|app| app.copy(id.as_str(), paste)));
        window.on_modify_item(|id, favorite| use_app(|app| app.modify(id.as_str(), favorite)));
        window.on_set_auto_paste(|value| use_app(|app| app.set_auto_paste(value)));
        window.on_authorize_paste(|| use_app(App::authorize));
        window.on_edit_history_image(|id| {
            if let Ok(id) = id.parse() {
                post(Event::Pins(pins::Message::History(
                    id,
                    pins::ImageDestination::Viewer,
                )));
            }
        });
        window.on_preview_item(|id| {
            if let Ok(id) = id.parse() {
                post(Event::Tools(tools::Message::Preview(id)));
            }
        });
        window.on_show_preview(|open| post(Event::Preview(preview::Message::Show(open))));
        window.on_preview_selection_changed(|| post(Event::Preview(preview::Message::Selection)));
        window.on_translate_preview(|text| {
            post(Event::Preview(preview::Message::Translate(
                text.to_string(),
            )));
        });
        window.on_copy_preview_text(|text| {
            post(Event::Preview(preview::Message::CopyText(text.to_string())));
        });
        window.on_history_viewport_changed(|first, end| {
            post(Event::Thumbnails(thumbnails::Message::Viewport(first, end)));
        });
        window.on_open_launcher(|| post(Event::Launcher(launcher::Message::OpenFromHistory)));
        window.window().on_close_requested(|| {
            post(Event::Hide);
            CloseRequestResponse::HideWindow
        });
        window.show()?;
        if self.identity.channel == DistributionChannel::Qa {
            eprintln!("panel_shown");
        }
        self.window = Some(window);
        self.reload();
        Ok(())
    }

    fn hide(&mut self) {
        self.preview.select(None);
        self.thumbnails.clear();
        if let Some(window) = self.window.take() {
            let _ = window.hide();
            if self.identity.channel == DistributionChannel::Qa {
                eprintln!("panel_hidden");
            }
        }
        self.target = None;
    }

    fn reload(&mut self) {
        let Some(window) = &self.window else { return };
        window.set_loading_history(true);
        self.generation += 1;
        let query = window.get_query();
        let result = ClipListRequest::new(
            self.generation,
            (!query.is_empty()).then(|| query.to_string()),
            window.get_favorites_only(),
            0,
            i64::from(window.get_history_limit()),
        )
        .and_then(|request| {
            self.storage
                .client()
                .list_clips(request, |generation, result| {
                    post(Event::History(generation, result))
                })
        });
        if let Err(error) = result {
            window.set_loading_history(false);
            self.status(format!("History unavailable: {error}"));
        }
    }

    fn copy(&mut self, id: &str, paste: bool) {
        if self.busy || id.is_empty() || self.window.as_ref().unwrap().get_loading_history() {
            return;
        }
        let id = id
            .parse()
            .expect("The clipboard model carries database IDs");
        let target = if paste && self.config.auto_paste {
            self.target
        } else {
            None
        };
        match self
            .clipboard
            .submit(clipboard::Request::Copy { id, target })
        {
            Ok(()) => self.set_busy(true),
            Err(error) => self.status(error.to_string()),
        }
    }

    fn modify(&mut self, id: &str, favorite: bool) {
        if self.busy || id.is_empty() {
            return;
        }
        let id = id
            .parse()
            .expect("The clipboard model carries database IDs");
        let result = self.storage.client().execute(
            move |database| {
                if favorite {
                    database.toggle_favorite(id).map(|_| ())
                } else {
                    database.delete_clip(id)
                }
            },
            |result| post(Event::Changed(result)),
        );
        if let Err(error) = result {
            self.status(format!("Could not update history: {error}"));
        }
    }

    fn set_auto_paste(&mut self, value: bool) {
        let next = AppConfig {
            auto_paste: value,
            ..self.config.clone()
        };
        if let Err(error) = self.save_config(&next) {
            self.status(format!("Could not save settings: {error}"));
            return;
        }
        self.config = next;
        if let Some(window) = &self.window {
            window.set_auto_paste(value);
        }
        if !value {
            self.target = None;
            let _ = self.clipboard.submit(clipboard::Request::ClosePermission);
        }
    }

    fn save_config(&self, config: &AppConfig) -> Result<()> {
        #[cfg(target_os = "linux")]
        anyhow::ensure!(
            self.gnome_settings_pending.is_none(),
            "Wait for the shortcut save to finish"
        );
        Self::write_config(&self.paths.config_file, config)
    }

    fn write_config(path: &Path, config: &AppConfig) -> Result<()> {
        let temporary = path.with_extension("tmp");
        write_private(&temporary, serialize_config_json(config)?.as_bytes())?;
        commit_private_replacement(&temporary, path)?;
        Ok(())
    }

    fn authorize(&mut self) {
        if !self.busy && self.config.auto_paste {
            match self.clipboard.submit(clipboard::Request::Authorize) {
                Ok(()) => self.set_busy(true),
                Err(error) => self.status(error.to_string()),
            }
        }
    }

    fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
        if let Some(window) = &self.window {
            window.set_busy(busy);
        }
    }

    fn status(&mut self, message: impl Into<SharedString>) {
        self.action_status = message.into();
        if let Some(window) = &self.window {
            window.set_action_status(self.action_status.clone());
        }
    }

    fn complete(&mut self, message: impl Into<slint::SharedString>) {
        self.set_busy(false);
        self.status(message);
    }

    fn start_capture(&mut self) {
        self.delayed_capture.stop();
        if self.busy {
            return;
        }
        let started = Instant::now();
        let cancellation = self.capture_shutdown.clone();
        let panel = self.window.is_some() || self.launcher.is_open();
        self.set_busy(true);
        self.launcher.close();
        self.hide();
        if panel {
            Timer::single_shot(Duration::from_millis(32), move || {
                if let Err(error) = capture::start(started, cancellation) {
                    use_app(|app| app.capture_error(error.into()));
                }
            });
        } else if let Err(error) = capture::start(started, cancellation) {
            self.capture_error(error.into());
        }
    }

    fn save_screens(&mut self) {
        if self.busy {
            return;
        }
        self.delayed_capture.stop();
        let target =
            SaveTarget::from_config(&self.config, &self.paths.screenshot_default(self.identity));
        let panel = self.window.is_some();
        self.set_busy(true);
        self.hide();
        if panel {
            Timer::single_shot(Duration::from_millis(32), move || {
                if let Err(error) = capture::save_screens(target) {
                    use_app(|app| app.capture_error(error.into()));
                }
            });
        } else if let Err(error) = capture::save_screens(target) {
            self.capture_error(error.into());
        }
    }

    fn capture_error(&mut self, error: anyhow::Error) {
        eprintln!("Screenshot unavailable: {error:#}");
        self.complete(format!("Screenshot unavailable: {error:#}"));
        if let Err(error) = self.open(false) {
            eprintln!("Unable to open Clippy: {error}");
        }
    }
}
