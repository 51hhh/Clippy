use crate::{build_label, clip_row, AppWindow, CaptureAction, ClippyTray};
use anyhow::{Context, Result};
use clippy_core::{
    config::{parse_config_json, serialize_config_json, AppConfig},
    image_output::SaveTarget,
    models::ClipItem,
};
use clippy_platform::{
    capture::{FrozenOutput, Region},
    identity::{DeploymentIdentity, DistributionChannel},
    instance::Instance,
    paste::{capture_target, Target},
    paths::AppPaths,
    private_files::{replace_private_file, restrict_directory, write_private},
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
    sync::Arc,
    time::{Duration, Instant},
};

mod capture;
mod clipboard;
mod editor;
mod longshot;
mod pins;
mod recording;
mod settings;
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
    tray.on_open_actions(|| handle(Event::Tools(tools::Message::Show)));
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
    tray.on_quit(|| {
        let _ = slint::quit_event_loop();
    });

    let shortcuts = Shortcuts::start(
        identity.current_platform(),
        &config.global_shortcut,
        &config.capture_shortcut,
        |action| match action {
            Action::Open => post(Event::Open(false)),
            Action::Capture => post(Event::CaptureStart),
        },
        |message| post(Event::ShortcutStatus(message)),
    );
    let shortcut_status = shortcuts
        .as_ref()
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    let _shortcuts = shortcuts.ok();
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
            shortcut_status,
            action_status: SharedString::default(),
            capture_views: None,
            delayed_capture: Timer::default(),
            editor: None,
            editor_id: 0,
            pins: pins::Pins::default(),
            longshot: None,
            capture_task_id: 0,
            vision: vision::Vision::default(),
            recordings: recording::Recordings::default(),
            tools: tools::Tools::default(),
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
    APP.with(|app| app.borrow_mut().take());
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
    Changed(Result<(), StorageError>),
    CaptureStart,
    CaptureReady {
        started: Instant,
        result: Result<Vec<FrozenOutput>>,
    },
    CapturePlacementFailed(slint::Weak<crate::CaptureWindow>, anyhow::Error),
    CaptureCancel,
    CaptureSelection {
        index: usize,
        action: CaptureAction,
        bounds: [f32; 4],
    },
    CaptureClipboard(Result<image::RgbaImage>, Instant),
    CaptureFinished(Result<String>, Instant),
    EditorOpen(Result<image::RgbaImage>),
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
    shortcut_status: String,
    action_status: SharedString,
    capture_views: Option<Vec<capture::View>>,
    delayed_capture: Timer,
    editor: Option<editor::View>,
    editor_id: u64,
    pins: pins::Pins,
    longshot: Option<longshot::View>,
    capture_task_id: u64,
    vision: vision::Vision,
    recordings: recording::Recordings,
    tools: tools::Tools,
}

impl App {
    fn handle(&mut self, event: Event) {
        match event {
            Event::Open(settings) if self.capture_views.is_none() => {
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
                    Ok(views) => self.capture_views = Some(views),
                    Err(error) => self.capture_error(error),
                }
            }
            Event::CapturePlacementFailed(window, error) => {
                if window.upgrade().is_some() {
                    self.capture_views.take();
                    self.capture_error(error);
                }
            }
            Event::CaptureCancel => {
                if self.capture_views.take().is_some() {
                    self.set_busy(false);
                }
            }
            Event::CaptureSelection {
                index,
                action,
                bounds,
            } => {
                if let Some(views) = self.capture_views.take() {
                    let destination = match action {
                        CaptureAction::Copy => capture::Destination::Clipboard,
                        CaptureAction::Edit => capture::Destination::Editor,
                        CaptureAction::Longshot => capture::Destination::Longshot,
                        CaptureAction::Record => capture::Destination::Record,
                        CaptureAction::Save => capture::Destination::File(SaveTarget::from_config(
                            &self.config,
                            &self.paths.screenshot_default(self.identity),
                        )),
                    };
                    if let Err(error) = capture::output(views, index, bounds, destination) {
                        self.capture_error(error.into());
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
                if let Err(error) = result.and_then(|image| self.open_editor(image)) {
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
                post(Event::Pins(pins::Message::History(id)));
            }
        });
        window.on_preview_item(|id| {
            if let Ok(id) = id.parse() {
                post(Event::Tools(tools::Message::Preview(id)));
            }
        });
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
        let temporary = self.paths.config_file.with_extension("tmp");
        write_private(&temporary, serialize_config_json(config)?.as_bytes())?;
        replace_private_file(&temporary, &self.paths.config_file)?;
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
        let panel = self.window.is_some();
        self.set_busy(true);
        self.hide();
        if panel {
            Timer::single_shot(Duration::from_millis(32), move || {
                if let Err(error) = capture::start(started) {
                    use_app(|app| app.capture_error(error.into()));
                }
            });
        } else if let Err(error) = capture::start(started) {
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
