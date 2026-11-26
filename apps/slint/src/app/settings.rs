use super::{post, use_app, App, Event};
use crate::{AppWindow, ClippyTheme, GeneralSettings, ShortcutField, TranslationSettings};
use anyhow::{ensure, Context, Result};
use clippy_core::{
    config::{AppConfig, TranslationServiceConfig},
    image_output::expand_user_path,
};
use clippy_platform::{
    identity::{DeploymentIdentity, DistributionChannel},
    shortcuts, startup,
};
use clippy_storage::{read_archive, write_new_archive, ArchiveScope};
use clippy_translate::{secrets, types::TranslationProvider};
use slint::{
    winit_030::{
        winit::{
            event::{ElementState, MouseButton, WindowEvent},
            keyboard::{KeyCode, ModifiersState, PhysicalKey},
        },
        EventResult, WinitWindowAccessor,
    },
    ComponentHandle, ModelRc, SharedString, VecModel,
};
use std::thread;

#[cfg(target_os = "linux")]
mod gnome_helper;

const THEMES: [&str; 6] = [
    "light",
    "dark",
    "nord",
    "solarized-light",
    "rose",
    "midnight",
];

pub(super) enum Message {
    Data(Result<String>),
    Stats(Result<String>),
    Credentials(Result<String>),
    #[cfg(target_os = "linux")]
    GnomeHelper(Result<clippy_platform::gnome_extension::Status>),
}

pub(super) fn namespace(identity: &DeploymentIdentity) -> String {
    match identity.channel {
        DistributionChannel::Production => secrets::KEYRING_SERVICE.to_owned(),
        DistributionChannel::Qa => {
            format!("{}.translation", identity.current_platform().application_id)
        }
    }
}

fn send(message: Message) {
    post(Event::Settings(message));
}

pub(super) fn theme_index(name: &str) -> i32 {
    THEMES.iter().position(|theme| *theme == name).unwrap_or(0) as i32
}

fn apply_theme(window: &AppWindow, theme: i32) {
    window.global::<ClippyTheme>().set_index(theme);
}

pub(super) fn connect_titlebar<T: ComponentHandle + 'static>(
    window: &T,
    contains: impl Fn(&T, f32, f32) -> bool + 'static,
) {
    connect_window_events(window, contains, |_, _| EventResult::Propagate);
}

pub(super) fn connect_window_frame<T: ComponentHandle + 'static>(
    window: &T,
    theme: ClippyTheme<'_>,
) {
    let border = theme.get_window_resize_border();
    let height = theme.get_titlebar_height();
    let inset = theme.get_titlebar_drag_inset();
    connect_titlebar(window, move |window, x, y| {
        let native = window.window();
        let width = native.size().to_logical(native.scale_factor()).width;
        x >= border && x < width - inset && y >= border && y < height
    });
}

fn connect_window_events<T: ComponentHandle + 'static>(
    window: &T,
    contains: impl Fn(&T, f32, f32) -> bool + 'static,
    mut intercept: impl FnMut(&T, &WindowEvent) -> EventResult + 'static,
) {
    let weak = window.as_weak();
    let mut cursor = None;
    window.window().on_winit_window_event(move |host, event| {
        let Some(window) = weak.upgrade() else {
            return EventResult::Propagate;
        };
        if matches!(intercept(&window, event), EventResult::PreventDefault) {
            return EventResult::PreventDefault;
        }
        match event {
            WindowEvent::CursorMoved { position, .. } => cursor = Some(*position),
            WindowEvent::CursorLeft { .. } => cursor = None,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(position) = cursor {
                    let position = position.to_logical::<f32>(f64::from(host.scale_factor()));
                    if contains(&window, position.x, position.y)
                        && host.with_winit_window(|window| window.drag_window().is_ok())
                            == Some(true)
                    {
                        // 窗口管理器接管拖动后可能吞掉释放事件。按下事件也不交给 Slint。
                        return EventResult::PreventDefault;
                    }
                }
            }
            _ => {}
        }
        EventResult::Propagate
    });
}

fn connect_shortcut_recorder(window: &AppWindow) {
    let mut modifiers = ModifiersState::empty();
    connect_window_events(
        window,
        AppWindow::invoke_titlebar_contains,
        move |window, event| {
            match event {
                WindowEvent::ModifiersChanged(value) => modifiers = value.state(),
                WindowEvent::Focused(false) => {
                    // X11 已注册热键会临时接管键盘，录制仍由 Esc、切页和关闭结束。
                    modifiers = ModifiersState::empty();
                }
                WindowEvent::KeyboardInput {
                    event,
                    is_synthetic,
                    ..
                } if window.get_recording_shortcut() != ShortcutField::None => {
                    if event.state == ElementState::Pressed && !event.repeat && !is_synthetic {
                        if let PhysicalKey::Code(key) = event.physical_key {
                            if key == KeyCode::Escape {
                                window.set_recording_shortcut(ShortcutField::None);
                            } else if let Some(chord) = recorded_shortcut(key, modifiers) {
                                match window.get_recording_shortcut() {
                                    ShortcutField::Clipboard => {
                                        window.set_panel_shortcut_draft(chord.into())
                                    }
                                    ShortcutField::Pin => {
                                        window.set_pin_shortcut_draft(chord.into())
                                    }
                                    ShortcutField::Screenshot => {
                                        window.set_capture_shortcut_draft(chord.into())
                                    }
                                    ShortcutField::None => {
                                        unreachable!("Shortcut recording is active")
                                    }
                                }
                                window.set_recording_shortcut(ShortcutField::None);
                            }
                        }
                    }
                    return EventResult::PreventDefault;
                }
                _ => {}
            }
            EventResult::Propagate
        },
    );
}

fn recorded_shortcut(key: KeyCode, modifiers: ModifiersState) -> Option<String> {
    if !(modifiers.control_key() || modifiers.alt_key() || modifiers.super_key())
        || matches!(
            key,
            KeyCode::CapsLock | KeyCode::NumLock | KeyCode::ScrollLock | KeyCode::Escape
        )
    {
        return None;
    }
    let name = format!("{key:?}");
    let key = name
        .strip_prefix("Key")
        .or_else(|| name.strip_prefix("Digit"))
        .filter(|value| value.len() == 1)
        .unwrap_or(&name);
    let super_key = if cfg!(target_os = "macos") {
        "Command"
    } else {
        "Super"
    };
    let keys = [
        (modifiers.control_key(), "Ctrl"),
        (modifiers.alt_key(), "Alt"),
        (modifiers.shift_key(), "Shift"),
        (modifiers.super_key(), super_key),
        (true, key),
    ];
    let chord = keys
        .into_iter()
        .filter_map(|(active, key)| active.then_some(key))
        .collect::<Vec<_>>()
        .join("+");
    shortcuts::can_parse(&chord).then_some(chord)
}

pub(super) fn connect(window: &AppWindow, config: &AppConfig) {
    populate(window, config);
    let destination = config
        .enabled_translation_services()
        .first()
        .map(|service| service.provider.as_str())
        .unwrap_or("No provider configured");
    window.set_translation_destination(destination.into());
    let index = config
        .enabled_translation_services()
        .first()
        .and_then(|service| service.provider.parse::<TranslationProvider>().ok())
        .and_then(|provider| {
            TranslationProvider::all()
                .iter()
                .position(|item| *item == provider)
        })
        .unwrap_or(0);
    window.set_provider_index(index as i32);
    translation(window, config, index as i32).expect("设置页中的翻译服务索引有效");
    window.set_tmux_available(cfg!(target_os = "linux"));
    if window.get_show_settings() {
        let defaults = AppConfig::default();
        window.set_shortcut_defaults(ModelRc::new(VecModel::from(vec![
            SharedString::from(defaults.global_shortcut),
            defaults.pin_shortcut.into(),
            defaults.capture_shortcut.into(),
        ])));
        window.set_wayland_session(clippy_platform::session::is_wayland());
        window.set_gnome_session(clippy_platform::session::is_gnome());
        #[cfg(target_os = "linux")]
        if window.get_wayland_session() && window.get_gnome_session() {
            gnome_helper::connect(window);
        }
        connect_shortcut_recorder(window);
    }
    window.on_apply_general(|settings| {
        use_app(|app| {
            let result = app.apply_general(settings);
            app.settings_result(result);
        })
    });
    window.on_select_provider(|index| {
        use_app(|app| {
            let window = app.window.as_ref().expect("设置窗口已打开");
            window.set_provider_index(index);
            if let Err(error) = translation(window, &app.config, index) {
                app.status(error.to_string());
            }
        })
    });
    window.on_apply_translation(|index, value| {
        use_app(|app| {
            let result = app.apply_translation(index, value);
            app.settings_result(result);
        })
    });
    window.on_store_credentials(|index, key, secret| {
        use_app(|app| {
            let result = app.write_credentials(index, Some((key.to_string(), secret.to_string())));
            if let Err(error) = result {
                app.status(error.to_string());
            }
        })
    });
    window.on_delete_credentials(|index| {
        use_app(|app| {
            let result = app.write_credentials(index, None);
            if let Err(error) = result {
                app.status(error.to_string());
            }
        })
    });
    window.on_forget_paste(|| {
        use_app(|app| {
            #[cfg(target_os = "linux")]
            {
                match app
                    .clipboard
                    .submit(super::clipboard::Request::ForgetPermission)
                {
                    Ok(()) => {
                        app.target = None;
                        app.set_busy(true);
                    }
                    Err(error) => app.status(error.to_string()),
                }
            }
            #[cfg(not(target_os = "linux"))]
            app.status("Manage paste access in your system privacy settings.");
        })
    });
    window.on_set_autostart(|enabled| {
        use_app(|app| {
            let message = startup::set_enabled(app.identity, enabled).map(|_| {
                app.window
                    .as_ref()
                    .expect("设置窗口已打开")
                    .set_autostart(enabled);
                "Login startup updated".into()
            });
            app.settings_result(message);
        })
    });
    window.on_data_action(|action, path| {
        use_app(|app| {
            if let Err(error) = app.data_action(action, path.as_str()) {
                app.status(error.to_string());
            }
        })
    });
}

pub(super) fn show(window: &AppWindow, identity: &DeploymentIdentity) {
    match startup::enabled(identity) {
        Ok(enabled) => {
            window.set_autostart_available(true);
            window.set_autostart(enabled);
        }
        Err(_) => window.set_autostart_available(false),
    }
}

fn populate(window: &AppWindow, config: &AppConfig) {
    let theme = theme_index(&config.theme);
    window.set_general_settings(GeneralSettings {
        max_history: config.max_history.to_string().into(),
        panel_shortcut: config.global_shortcut.clone().into(),
        pin_shortcut: config.pin_shortcut.clone().into(),
        capture_shortcut: config.capture_shortcut.clone().into(),
        theme,
        save_directory: config.screenshot_save_dir.clone().into(),
        filename_template: config.screenshot_filename_template.clone().into(),
        ocr_enabled: config.ocr_enabled,
        ocr_manifest: config.enhanced_ocr_manifest_path.clone().into(),
    });
    apply_theme(window, theme);
}

fn provider(index: i32) -> Result<TranslationProvider> {
    TranslationProvider::all()
        .get(index as usize)
        .copied()
        .context("Select a translation provider")
}

fn translation(window: &AppWindow, config: &AppConfig, index: i32) -> Result<()> {
    let provider = provider(index)?;
    let service = config
        .translation_services
        .iter()
        .find(|entry| entry.provider == provider.as_str());
    let active = config.enabled_translation_services();
    window.set_translation_settings(TranslationSettings {
        enabled: active
            .first()
            .is_some_and(|entry| entry.provider == provider.as_str()),
        endpoint: service
            .map(|entry| entry.endpoint.as_str())
            .unwrap_or_default()
            .into(),
        model: service
            .map(|entry| entry.model.as_str())
            .unwrap_or_default()
            .into(),
        project: service
            .map(|entry| entry.project.as_str())
            .unwrap_or_default()
            .into(),
        region: service
            .map(|entry| entry.region.as_str())
            .unwrap_or_default()
            .into(),
        source: config.translation_source_language.clone().into(),
        target: config.translation_target_language.clone().into(),
    });
    Ok(())
}

fn language(value: &str, optional: bool) -> Result<String> {
    let value = value.trim();
    if optional && value.is_empty() {
        return Ok("auto".into());
    }
    ensure!(
        !value.is_empty()
            && value.len() <= 24
            && value
                .split('-')
                .all(|part| !part.is_empty()
                    && part.bytes().all(|byte| byte.is_ascii_alphanumeric())),
        "Enter a language code such as en or zh-Hans"
    );
    ensure!(
        optional || !value.eq_ignore_ascii_case("auto"),
        "Select a target language"
    );
    Ok(value.to_owned())
}

impl App {
    fn settings_result(&mut self, result: Result<String>) {
        self.status(result.unwrap_or_else(|error| error.to_string()));
    }

    fn apply_general(&mut self, value: GeneralSettings) -> Result<String> {
        let max_history = value
            .max_history
            .trim()
            .parse::<u32>()
            .context("Enter a history limit")?;
        let panel = value.panel_shortcut.trim();
        let pin = value.pin_shortcut.trim();
        let capture = value.capture_shortcut.trim();
        let shortcuts = shortcuts::validate(panel, pin, capture)?;
        let theme = THEMES.get(value.theme as usize).context("Select a theme")?;
        let shortcuts_changed = self.config.global_shortcut != panel
            || self.config.pin_shortcut != pin
            || self.config.capture_shortcut != capture;
        let next = AppConfig {
            max_history,
            global_shortcut: panel.into(),
            pin_shortcut: pin.into(),
            capture_shortcut: capture.into(),
            theme: (*theme).into(),
            screenshot_save_dir: value.save_directory.trim().into(),
            screenshot_filename_template: value.filename_template.trim().into(),
            ocr_enabled: value.ocr_enabled,
            enhanced_ocr_manifest_path: value.ocr_manifest.trim().into(),
            ..self.config.clone()
        };
        let wayland = clippy_platform::session::is_wayland();
        if shortcuts_changed && !wayland {
            let path = &self.paths.config_file;
            self.shortcuts
                .as_mut()
                .context("Global shortcuts are unavailable on this desktop")?
                .rebind_native(shortcuts, || Self::write_config(path, &next))?;
            self.handle(Event::ShortcutStatus(String::new()));
        } else {
            self.save_config(&next)?;
        }
        self.clipboard.set_limit(max_history);
        clippy_vision::set_manifest_setting(&next.enhanced_ocr_manifest_path);
        self.config = next;
        populate(self.window.as_ref().expect("设置窗口已打开"), &self.config);
        Ok(if shortcuts_changed && wayland {
            "Settings saved. Restart Clippy to apply shortcuts."
        } else {
            "Settings saved"
        }
        .into())
    }

    fn apply_translation(&mut self, index: i32, value: TranslationSettings) -> Result<String> {
        let provider = provider(index)?;
        let source = language(value.source.as_str(), true)?;
        let target = language(value.target.as_str(), false)?;
        let mut next = self.config.clone();
        let endpoint = value.endpoint.trim().to_owned();
        if !endpoint.is_empty() {
            let uri = endpoint
                .parse::<http::Uri>()
                .context("Enter a valid HTTP(S) endpoint")?;
            ensure!(
                matches!(uri.scheme_str(), Some("http" | "https")) && uri.host().is_some(),
                "Enter a valid HTTP(S) endpoint"
            );
        }
        let entry = TranslationServiceConfig {
            provider: provider.as_str().into(),
            enabled: value.enabled,
            endpoint,
            model: value.model.trim().into(),
            region: value.region.trim().into(),
            project: value.project.trim().into(),
        };
        next.translation_services
            .retain(|saved| saved.provider != entry.provider);
        next.translation_services.insert(0, entry);
        next.translation_source_language = source;
        next.translation_target_language = target;
        self.save_config(&next)?;
        self.config = next;
        translation(
            self.window.as_ref().expect("设置窗口已打开"),
            &self.config,
            index,
        )?;
        Ok("Translation settings saved".into())
    }

    fn write_credentials(
        &mut self,
        index: i32,
        credentials: Option<(String, String)>,
    ) -> Result<()> {
        ensure!(!self.busy, "Wait for the current operation");
        let provider = provider(index)?;
        let namespace = namespace(self.identity);
        thread::Builder::new()
            .name("clippy-keychain".into())
            .spawn(move || {
                let result = match credentials {
                    Some((key, secret)) => {
                        secrets::set_credentials_in(&namespace, provider, &key, Some(&secret))
                            .map(|()| "Credentials saved to the system keychain".into())
                    }
                    None => secrets::delete_credentials_in(&namespace, provider)
                        .map(|()| "Stored credentials removed".into()),
                };
                send(Message::Credentials(result.map_err(Into::into)));
            })?;
        self.set_busy(true);
        Ok(())
    }

    fn data_action(&mut self, action: i32, path: &str) -> Result<()> {
        let client = self.storage.client();
        match action {
            0 => client.execute(
                |storage| storage.get_stats(),
                |result| {
                    send(Message::Stats(
                        result
                            .map(|stats| {
                                format!(
                    "{} history items · {} favorites · {} images · {:.2} MiB database",
                    stats["total"], stats["favorites"], stats["image_count"],
                    stats["db_size"].as_u64().unwrap_or(0) as f64 / 1024. / 1024.,
                )
                            })
                            .map_err(Into::into),
                    ));
                },
            )?,
            1 | 2 => {
                ensure!(!self.busy, "Wait for the current operation");
                let path = expand_user_path(path.trim());
                ensure!(
                    path.is_absolute() && path.to_string_lossy().ends_with(".clippy.zip"),
                    "Enter an absolute .clippy.zip path"
                );
                if action == 1 {
                    client.execute(
                        |storage| storage.export_archive_snapshot(ArchiveScope::Full, false),
                        move |snapshot| {
                            let result =
                                snapshot.map_err(anyhow::Error::from).and_then(|snapshot| {
                                    write_new_archive(&path, &snapshot)
                                        .map_err(anyhow::Error::msg)
                                        .map(|archive| {
                                            format!(
                                                "Exported {} clips and {} pins to {}",
                                                archive.clips, archive.workspaces, archive.path
                                            )
                                        })
                                });
                            send(Message::Data(result));
                        },
                    )?;
                } else {
                    thread::Builder::new().name("clippy-archive-import".into()).spawn(move || {
                        let queued = read_archive(&path).map_err(anyhow::Error::msg).and_then(|snapshot| {
                            client.execute(move |storage| storage.import_archive_snapshot(&snapshot), |result| {
                                send(Message::Data(result.map(|summary| format!(
                                    "Imported {} clips and {} pins · {} clips merged",
                                    summary.clips_added, summary.workspaces_added, summary.clips_merged,
                                )).map_err(Into::into)));
                            }).map_err(Into::into)
                        });
                        if let Err(error) = queued { send(Message::Data(Err(error))); }
                    })?;
                }
                self.set_busy(true);
            }
            #[cfg(target_os = "linux")]
            3 => self.capture_tmux()?,
            4 => {
                let message = format!(
                    "{} / {} · data directory {}",
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                    self.paths.data_dir.display(),
                );
                self.window
                    .as_ref()
                    .expect("设置窗口已打开")
                    .set_data_status(message.into());
            }
            _ => {}
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn capture_tmux(&mut self) -> Result<()> {
        use clippy_core::clipboard::MAX_CLIPBOARD_TEXT_BYTES;
        use std::{
            io::Read,
            process::{Command, Stdio},
        };
        ensure!(!self.busy, "Wait for the current operation");
        thread::Builder::new()
            .name("clippy-tmux".into())
            .spawn(move || {
                let result = (|| {
                    let mut child = Command::new("tmux")
                        .args(["capture-pane", "-p"])
                        .stdin(Stdio::null())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .spawn()?;
                    let mut text = String::new();
                    let mut output = child
                        .stdout
                        .take()
                        .expect("tmux stdout 已重定向")
                        .take((MAX_CLIPBOARD_TEXT_BYTES + 1) as u64);
                    let read = output.read_to_string(&mut text);
                    drop(output);
                    let status = child.wait()?;
                    read?;
                    ensure!(
                        text.len() <= MAX_CLIPBOARD_TEXT_BYTES,
                        "tmux pane exceeds the clipboard limit"
                    );
                    ensure!(status.success(), "No active tmux pane");
                    Ok(text)
                })();
                post(Event::TmuxCaptured(result));
            })?;
        self.set_busy(true);
        Ok(())
    }

    pub(super) fn settings_message(&mut self, message: Message) {
        match message {
            Message::Data(result) => {
                self.set_busy(false);
                self.settings_result(result);
                self.reload();
                let _ = self.data_action(0, "");
            }
            Message::Stats(result) => {
                if let Some(window) = &self.window {
                    window.set_data_status(result.unwrap_or_else(|error| error.to_string()).into());
                }
            }
            Message::Credentials(result) => {
                self.set_busy(false);
                self.settings_result(result);
            }
            #[cfg(target_os = "linux")]
            Message::GnomeHelper(result) => gnome_helper::completed(self, result),
        }
    }
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;

    #[test]
    fn physical_keys_produce_supported_portable_shortcuts() {
        let ctrl_alt = ModifiersState::CONTROL | ModifiersState::ALT;
        assert_eq!(
            recorded_shortcut(KeyCode::KeyK, ctrl_alt).as_deref(),
            Some("Ctrl+Alt+K")
        );
        assert_eq!(
            recorded_shortcut(KeyCode::F8, ctrl_alt).as_deref(),
            Some("Ctrl+Alt+F8")
        );
        assert_eq!(
            recorded_shortcut(KeyCode::NumpadAdd, ModifiersState::CONTROL).as_deref(),
            Some("Ctrl+NumpadAdd")
        );
        assert!(recorded_shortcut(KeyCode::KeyK, ModifiersState::SHIFT).is_none());
        assert!(recorded_shortcut(KeyCode::ControlLeft, ModifiersState::CONTROL).is_none());
        assert!(recorded_shortcut(KeyCode::Escape, ctrl_alt).is_none());
        assert!(shortcuts::validate("Ctrl+Alt+K", "Ctrl+2", "Alt+Control+KeyK").is_err());
        assert!(shortcuts::validate("Ctrl+Alt+K", "Ctrl+2", "Control+Digit2").is_err());
    }
}
