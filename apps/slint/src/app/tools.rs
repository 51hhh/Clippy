use super::{clipboard, pins, post, preview_document, settings, vision, App, Event};
use crate::{locale::tr, ClippyTheme, ToolsWindow, TranslationRow};
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use clippy_core::{
    clipboard::{is_sensitive_text, MAX_CLIPBOARD_TEXT_BYTES},
    models::{ClipItem, TranslationHistoryEntry},
};
use clippy_storage::NewTranslation;
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use pulldown_cmark::{Event as Markdown, Parser, TagEnd};
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, StyledText, VecModel};
use std::{
    io::Write,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

const URL_ESCAPE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

pub(super) enum Message {
    Show,
    Close,
    Screenshot,
    #[cfg(target_os = "linux")]
    SystemScreenshot,
    #[cfg(target_os = "linux")]
    SystemScreenshotReady(u64, Result<Option<image::RgbaImage>>),
    Preview(i64),
    PreviewReady(u64, Result<ClipItem>),
    LatestImage,
    ImageReady(u64, Result<Option<i64>>),
    Run(i32, String),
    Copy(String),
    Speak(String),
    LoadHistory,
    ClearHistory,
    History(u64, Result<Vec<TranslationHistoryEntry>>),
    Completed(u64, Result<Option<TextOutput>>),
    Note(u64, Result<String>),
}

pub(super) struct TextOutput {
    plain: String,
    markup: Option<StyledText>,
}

#[derive(Default)]
pub(super) struct Tools {
    window: Option<ToolsWindow>,
    serial: u64,
    running: bool,
    cancel: Option<Arc<AtomicBool>>,
    #[cfg(target_os = "linux")]
    system_capture: Option<Arc<AtomicBool>>,
    clip_id: Option<i64>,
}

impl Drop for Tools {
    fn drop(&mut self) {
        self.close();
    }
}

fn send(message: Message) {
    post(Event::Tools(message));
}

impl Tools {
    fn show(&mut self, theme: i32) -> Result<()> {
        if let Some(window) = &self.window {
            window.global::<ClippyTheme>().set_index(theme);
            return Ok(window.show()?);
        }
        self.serial += 1;
        let window = ToolsWindow::new()?;
        window.global::<ClippyTheme>().set_index(theme);
        window.set_busy(self.running);
        window.on_dismiss(|| send(Message::Close));
        window.on_screenshot(|| send(Message::Screenshot));
        #[cfg(target_os = "linux")]
        {
            window.set_system_screenshot_available(clippy_platform::session::is_wayland());
            window.set_system_screenshot_pending(self.system_capture.is_some());
            window.on_system_screenshot(|| send(Message::SystemScreenshot));
        }
        window.on_use_image(|| send(Message::LatestImage));
        window.on_run_action(|action, text| send(Message::Run(action, text.to_string())));
        window.on_copy_text(|text| send(Message::Copy(text.to_string())));
        window.on_speak(|text| send(Message::Speak(text.to_string())));
        window.on_reload_history(|| send(Message::LoadHistory));
        window.on_clear_history(|| send(Message::ClearHistory));
        settings::connect_window_frame(&window, window.global::<ClippyTheme>());
        window.window().on_close_requested(|| {
            send(Message::Close);
            CloseRequestResponse::KeepWindowShown
        });
        window.show()?;
        self.window = Some(window);
        Ok(())
    }

    fn close(&mut self) {
        self.serial += 1;
        self.clip_id = None;
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        #[cfg(target_os = "linux")]
        if let Some(cancel) = &self.system_capture {
            cancel.store(true, Ordering::Release);
        }
        if let Some(window) = self.window.take() {
            let _ = window.hide();
        }
    }

    fn busy(&mut self, busy: bool) {
        self.running = busy;
        if let Some(window) = &self.window {
            window.set_busy(busy);
        }
    }

    fn note(&self, note: impl Into<slint::SharedString>) {
        if let Some(window) = &self.window {
            window.set_note(tr(note.into().as_str()).into());
        }
    }
}

fn markdown_text(source: &str) -> String {
    let mut plain = String::new();
    for event in Parser::new(source) {
        match event {
            Markdown::Text(value) | Markdown::Code(value) => plain.push_str(&value),
            Markdown::SoftBreak | Markdown::HardBreak => plain.push('\n'),
            Markdown::Start(pulldown_cmark::Tag::Item) => plain.push_str("• "),
            Markdown::End(
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item | TagEnd::CodeBlock,
            ) if !plain.ends_with('\n') => plain.push('\n'),
            _ => {}
        }
    }
    plain
}

fn transform(action: i32, text: &str) -> Result<String> {
    Ok(match action {
        1 => markdown_text(text),
        2 => serde_json::to_string_pretty(&serde_json::from_str::<serde_json::Value>(text)?)?,
        3 => STANDARD.encode(text),
        4 => String::from_utf8(STANDARD.decode(text.trim())?)?,
        5 => utf8_percent_encode(text, URL_ESCAPE).to_string(),
        6 => percent_decode_str(text).decode_utf8()?.into_owned(),
        7 => text
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        8 => {
            let text = text.trim();
            ensure!(
                text.is_ascii() && text.len().is_multiple_of(2),
                "Enter pairs of hex digits"
            );
            let bytes = (0..text.len())
                .step_by(2)
                .map(|index| u8::from_str_radix(&text[index..index + 2], 16))
                .collect::<Result<Vec<_>, _>>()?;
            String::from_utf8(bytes)?
        }
        _ => anyhow::bail!("Select a text action"),
    })
}

fn play_spoken(text: &str, language: &str, cancel: &AtomicBool) -> Result<()> {
    ensure!(
        !is_sensitive_text(text),
        "Sensitive text cannot be sent for speech"
    );
    let audio = clippy_translate::tts::fetch_audio(text, Some(language))?;
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut file = tempfile::NamedTempFile::new()?;
    file.write_all(&STANDARD.decode(audio.audio_base64)?)?;
    let mut child = Command::new("ffplay")
        .args(["-nodisp", "-autoexit", "-loglevel", "error", "-i"])
        .arg(file.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Speech playback requires ffplay")?;
    while !cancel.load(Ordering::Acquire) {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "Speech playback failed");
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}

impl App {
    pub(super) fn tools_message(&mut self, message: Message) {
        if let Err(error) = self.handle_tools(message) {
            self.tools.note(error.to_string());
        }
    }

    fn handle_tools(&mut self, message: Message) -> Result<()> {
        let theme = settings::theme_index(&self.config.theme);
        match message {
            Message::Close => self.tools.close(),
            Message::Show => self.tools.show(theme)?,
            Message::Screenshot => {
                self.tools.close();
                self.start_capture();
            }
            #[cfg(target_os = "linux")]
            Message::SystemScreenshot => {
                ensure!(
                    !self.tools.running && self.tools.system_capture.is_none(),
                    "Wait for the current action"
                );
                let id = self.tools.serial;
                let cancel = Arc::new(AtomicBool::new(false));
                let cancelled = cancel.clone();
                thread::Builder::new()
                    .name("clippy-system-screenshot".into())
                    .spawn(move || {
                        send(Message::SystemScreenshotReady(
                            id,
                            super::capture::system_screenshot(&cancelled),
                        ));
                    })?;
                self.tools.system_capture = Some(cancel);
                self.tools
                    .window
                    .as_ref()
                    .expect("系统截图由操作窗口发起")
                    .set_system_screenshot_pending(true);
                self.tools.note("Opening system screenshot…");
            }
            #[cfg(target_os = "linux")]
            Message::SystemScreenshotReady(id, result) => {
                self.tools.system_capture.take();
                if let Some(window) = &self.tools.window {
                    window.set_system_screenshot_pending(false);
                }
                // 系统界面留给桌面回收。操作窗口关闭后，旧请求的结果随代次丢弃。
                if id == self.tools.serial {
                    if let Some(image) = result? {
                        self.open_editor(image)?;
                        self.tools.close();
                    } else {
                        self.tools.note("System screenshot cancelled");
                    }
                }
            }
            Message::Preview(id) => {
                self.tools.close();
                self.tools.show(theme)?;
                let current = self.tools.serial;
                self.storage.client().execute(
                    move |storage| storage.get_clip_detail_by_id(id),
                    move |result| send(Message::PreviewReady(current, result.map_err(Into::into))),
                )?;
            }
            Message::PreviewReady(id, result) if id == self.tools.serial => {
                let clip = result?;
                let window = self.tools.window.as_ref().expect("预览窗口已打开");
                self.tools.clip_id = Some(clip.id);
                if clip.is_sensitive {
                    self.tools.note("Protected clipboard item");
                } else {
                    window.set_source(clip.text_content.unwrap_or_default().into());
                    self.tools.note(format!(
                        "{} · {} bytes",
                        clip.content_type.as_str(),
                        clip.byte_size
                    ));
                }
            }
            Message::LatestImage => {
                let current = self.tools.serial;
                self.storage.client().execute(
                    |storage| storage.get_latest_image_id(),
                    move |result| send(Message::ImageReady(current, result.map_err(Into::into))),
                )?;
            }
            Message::ImageReady(id, result) if id == self.tools.serial => {
                let clip = result?.context("No image in clipboard history")?;
                self.tools.close();
                self.pin_message(pins::Message::History(clip, pins::ImageDestination::Editor));
            }
            Message::Run(action, source) => self.run_text_action(action, source)?,
            Message::Copy(text) => {
                let id = self.tools.serial;
                self.clipboard.submit(clipboard::Request::WriteText {
                    text,
                    finish: Box::new(move |result| {
                        send(Message::Note(id, result.map(|()| "Copied".into())));
                    }),
                })?;
            }
            Message::Speak(text) => {
                ensure!(!self.tools.running, "Wait for the current action");
                let language = self.config.translation_target_language.clone();
                let id = self.tools.serial;
                let cancel = Arc::new(AtomicBool::new(false));
                let worker_cancel = cancel.clone();
                thread::Builder::new()
                    .name("clippy-speech".into())
                    .spawn(move || {
                        send(Message::Completed(
                            id,
                            play_spoken(&text, &language, &worker_cancel).map(|()| None),
                        ));
                    })?;
                self.tools.cancel = Some(cancel);
                self.tools.note("Fetching speech…");
                self.tools.busy(true);
            }
            Message::LoadHistory | Message::ClearHistory => {
                let id = self.tools.serial;
                let clear = matches!(message, Message::ClearHistory);
                self.storage.client().execute(
                    move |storage| {
                        if clear {
                            storage.clear_translation_history()?;
                        }
                        storage.translation_history(None, 50)
                    },
                    move |result| send(Message::History(id, result.map_err(Into::into))),
                )?;
            }
            Message::History(id, result) if id == self.tools.serial => {
                let rows = result?
                    .into_iter()
                    .map(|row| TranslationRow {
                        title: format!(
                            "{} · {} → {}",
                            row.provider, row.source_language, row.target_language
                        )
                        .into(),
                        source: row.source_text.into(),
                        result: row.translated_text.into(),
                    })
                    .collect::<Vec<_>>();
                if let Some(window) = &self.tools.window {
                    window.set_history(ModelRc::new(VecModel::from(rows)));
                }
            }
            Message::Completed(id, result) => {
                self.tools.cancel.take();
                self.tools.busy(false);
                if id == self.tools.serial {
                    if let Some(output) = result? {
                        if let Some(window) = &self.tools.window {
                            window.set_result(output.plain.into());
                            window.set_has_result_markup(output.markup.is_some());
                            window.set_result_markup(output.markup.unwrap_or_default());
                        }
                    }
                    self.tools.note("Done");
                }
            }
            Message::Note(id, result) if id == self.tools.serial => self.tools.note(result?),
            _ => {}
        }
        Ok(())
    }

    fn run_text_action(&mut self, action: i32, source: String) -> Result<()> {
        ensure!(!self.tools.running, "Wait for the current action");
        ensure!(
            source.len() <= MAX_CLIPBOARD_TEXT_BYTES,
            "Text exceeds the clipboard limit"
        );
        let id = self.tools.serial;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        if action == 0 {
            let request = vision::translation_request(&self.config, source.clone())?;
            let namespace = settings::namespace(self.identity);
            let database = self.storage.client();
            let clip_id = self.tools.clip_id;
            thread::Builder::new()
                .name("clippy-text-translation".into())
                .spawn(move || {
                    let result =
                        vision::translate(request, &namespace, &worker_cancel).map(|result| {
                            let text = result.translated_text.clone();
                            if !worker_cancel.load(Ordering::Acquire) {
                                let _ = database.execute(
                                    move |storage| {
                                        storage.record_translation(&NewTranslation {
                                            clip_id,
                                            provider: result.provider.as_str(),
                                            source_language: result
                                                .detected_source_language
                                                .as_deref()
                                                .unwrap_or("auto"),
                                            target_language: &result.target_language,
                                            source_text: &source,
                                            translated_text: &result.translated_text,
                                        })
                                    },
                                    move |logged| {
                                        if let Err(error) = logged {
                                            send(Message::Note(id, Err(error.into())));
                                        }
                                    },
                                );
                            }
                            TextOutput {
                                plain: text,
                                markup: None,
                            }
                        });
                    send(Message::Completed(id, result.map(Some)));
                })?;
        } else {
            thread::Builder::new()
                .name("clippy-transform-text".into())
                .spawn(move || {
                    let output = transform(action, &source).map(|plain| TextOutput {
                        plain,
                        markup: if action == 1 {
                            preview_document::markdown(&source)
                        } else {
                            None
                        },
                    });
                    send(Message::Completed(id, output.map(Some)));
                })?;
        }
        self.tools.cancel = Some(cancel);
        self.tools.busy(true);
        self.tools.note("Working…");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codecs_round_trip_and_markdown_keeps_links_inert() {
        for (encode, decode) in [(3, 4), (5, 6), (7, 8)] {
            let input = "剪贴板 /+? 🦀";
            assert_eq!(
                transform(decode, &transform(encode, input).unwrap()).unwrap(),
                input
            );
        }
        let preview = markdown_text(
            "# Title\n\n<script>alert(1)</script>\n\n[local](http://127.0.0.1:42959/)",
        );
        assert_eq!(preview, "Title\nlocal\n");
    }
}
