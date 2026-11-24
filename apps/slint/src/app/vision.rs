use super::{clipboard, editor, post, App, Event};
use crate::{ImageTextWindow, TextBox, TextParagraph};
use anyhow::{anyhow, Context, Result};
use clippy_capture::png_io::encode_png;
use clippy_core::{clipboard::is_sensitive_text, config::AppConfig};
use clippy_storage::NewTranslation;
use clippy_translate::{
    direction::resolve_direction,
    secrets,
    types::{ProviderOptions, TranslationProvider, TranslationRequest, TranslationResult},
    TranslationService,
};
use clippy_vision::{code_detection, StructuredOcr};
use image::RgbaImage;
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, VecModel};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
};

#[derive(Clone, Copy)]
pub(super) enum Mode {
    Recognize,
    Scan,
}

pub(super) enum Message {
    Recognized(u64, Result<Recognition>),
    SelectLine(i32),
    SelectParagraph(i32),
    Copy,
    Translate(i32),
    Translated(u64, i32, String, Result<TranslationResult>),
    Status(u64, String),
    Close,
}

pub(super) enum Recognition {
    Ocr(StructuredOcr),
    Text(String),
}

#[derive(Default)]
pub(super) struct Vision {
    serial: u64,
    recognition_running: bool,
    translation_running: bool,
    window: Option<View>,
}

struct View {
    id: u64,
    window: ImageTextWindow,
    lines: HashMap<i32, (i32, String)>,
    paragraphs: Vec<TextParagraph>,
    cancelled: Arc<AtomicBool>,
}

impl Drop for View {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        let _ = self.window.hide();
    }
}

fn send(message: Message) {
    post(Event::Vision(message));
}

impl View {
    fn open(id: u64, image: &RgbaImage, config: &AppConfig) -> Result<Self> {
        let window = ImageTextWindow::new()?;
        window.set_picture(editor::to_slint(image));
        window.set_image_width(image.width() as f32);
        window.set_image_height(image.height() as f32);
        window.set_busy(true);
        window.set_status("Reading image…".into());
        if let Some(service) = config.enabled_translation_services().first() {
            window.set_service(service.provider.clone().into());
        }
        window.on_select_line(|line| send(Message::SelectLine(line)));
        window.on_select_paragraph(|paragraph| send(Message::SelectParagraph(paragraph)));
        window.on_copy_result(|| send(Message::Copy));
        window.on_translate(|paragraph| send(Message::Translate(paragraph)));
        window.on_dismiss(|| send(Message::Close));
        window.window().on_close_requested(|| {
            send(Message::Close);
            CloseRequestResponse::KeepWindowShown
        });
        window.show()?;
        Ok(Self {
            id,
            window,
            lines: HashMap::new(),
            paragraphs: Vec::new(),
            cancelled: Arc::new(AtomicBool::new(false)),
        })
    }

    fn display(&mut self, result: Recognition) {
        match result {
            Recognition::Text(text) => {
                self.window.set_status(
                    if text.is_empty() {
                        "No barcode found"
                    } else {
                        "Code recognition"
                    }
                    .into(),
                );
                self.window.set_original(text.into());
            }
            Recognition::Ocr(result) => self.layout(result),
        }
    }

    fn layout(&mut self, mut result: StructuredOcr) {
        let (width, height) = (f64::from(result.width), f64::from(result.height));
        result.lines.sort_by_key(|line| line.reading_order);
        result
            .paragraphs
            .sort_by_key(|paragraph| paragraph.reading_order);
        let mut boxes = Vec::new();
        let mut paragraph_bounds = HashMap::<u32, [f64; 4]>::new();
        for line in result.lines.into_iter().filter(|line| line.accepted) {
            let [first, second, third, fourth] = line.quad.as_slice() else {
                continue;
            };
            let [left, top, right, bottom] = line.quad.iter().fold(
                [width, height, 0.0, 0.0],
                |[left, top, right, bottom], [x, y]| {
                    [left.min(*x), top.min(*y), right.max(*x), bottom.max(*y)]
                },
            );
            let bounds = paragraph_bounds
                .entry(line.paragraph_id)
                .or_insert([left, top, right, bottom]);
            *bounds = [
                bounds[0].min(left),
                bounds[1].min(top),
                bounds[2].max(right),
                bounds[3].max(bottom),
            ];
            boxes.push(TextBox {
                line: line.id as i32,
                paragraph: line.paragraph_id as i32,
                outline: format!(
                    "M {},{} L {},{} L {},{} L {},{} Z",
                    first[0],
                    first[1],
                    second[0],
                    second[1],
                    third[0],
                    third[1],
                    fourth[0],
                    fourth[1],
                )
                .into(),
                left: (left / width) as f32,
                top: (top / height) as f32,
                width: ((right - left) / width) as f32,
                height: ((bottom - top) / height) as f32,
            });
            self.lines
                .insert(line.id as i32, (line.paragraph_id as i32, line.text));
        }
        self.paragraphs = result
            .paragraphs
            .into_iter()
            .filter_map(|paragraph| {
                let [left, top, right, bottom] = *paragraph_bounds.get(&paragraph.id)?;
                let source = paragraph
                    .line_ids
                    .into_iter()
                    .filter_map(|line| {
                        self.lines
                            .get(&(line as i32))
                            .map(|(_, text)| text.as_str())
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Some(TextParagraph {
                    id: paragraph.id as i32,
                    source: source.into(),
                    translated: Default::default(),
                    left: (left / width) as f32,
                    top: (top / height) as f32,
                    width: ((right - left) / width) as f32,
                    height: ((bottom - top) / height) as f32,
                    in_place: false,
                })
            })
            .collect();
        self.window.set_boxes(ModelRc::new(VecModel::from(boxes)));
        self.window.set_original(result.text.into());
        self.window
            .set_status(format!("OCR · {}", result.pipeline.id).into());
        self.update_paragraphs();
        if let Some(paragraph) = self.paragraphs.first() {
            self.window.set_selected_paragraph(paragraph.id);
        }
    }

    fn update_paragraphs(&self) {
        self.window
            .set_paragraphs(ModelRc::new(VecModel::from(self.paragraphs.clone())));
    }
}

impl App {
    pub(super) fn analyze_image(&mut self, image: Arc<RgbaImage>, mode: Mode) -> Result<()> {
        if self.vision.recognition_running {
            anyhow::bail!("An image is already being analyzed");
        }
        if matches!(mode, Mode::Recognize) && !self.config.ocr_enabled {
            anyhow::bail!("OCR is disabled in Settings");
        }
        self.vision.serial += 1;
        let id = self.vision.serial;
        let new_view = View::open(id, &image, &self.config)?;
        thread::Builder::new()
            .name("clippy-image-analysis".into())
            .spawn(move || {
                let result = match mode {
                    Mode::Scan => code_detection::scan_rgba(
                        image.as_raw().clone(),
                        image.width(),
                        image.height(),
                    )
                    .map(|result| {
                        Recognition::Text(
                            result
                                .results
                                .into_iter()
                                .map(|code| code.text)
                                .collect::<Vec<_>>()
                                .join("\n"),
                        )
                    })
                    .map_err(Into::into),
                    Mode::Recognize => recognize(&image).map(Recognition::Ocr),
                };
                send(Message::Recognized(id, result));
            })?;
        self.vision.window = Some(new_view);
        self.vision.recognition_running = true;
        Ok(())
    }

    pub(super) fn vision_message(&mut self, message: Message) {
        if let Err(error) = self.handle_vision(message) {
            if let Some(view) = self.vision.window.as_ref() {
                view.window.set_status(error.to_string().into());
            }
        }
    }

    fn handle_vision(&mut self, message: Message) -> Result<()> {
        match &message {
            Message::Recognized(..) => self.vision.recognition_running = false,
            Message::Translated(..) => self.vision.translation_running = false,
            _ => {}
        }
        let Some(view) = self.vision.window.as_mut() else {
            return Ok(());
        };
        match message {
            Message::Close => {
                self.vision.window.take();
            }
            Message::Recognized(id, result) if view.id == id => {
                view.window.set_busy(false);
                view.display(result?);
            }
            Message::SelectLine(line) => {
                if let Some((paragraph, text)) = view.lines.get(&line) {
                    view.window.set_selected_line(line);
                    view.window.set_selected_paragraph(*paragraph);
                    view.window.set_current_text(text.clone().into());
                }
            }
            Message::SelectParagraph(id) => {
                if let Some(paragraph) = view.paragraphs.iter().find(|paragraph| paragraph.id == id)
                {
                    view.window.set_selected_line(-1);
                    view.window.set_selected_paragraph(id);
                    view.window
                        .set_current_text(if paragraph.translated.is_empty() {
                            paragraph.source.clone()
                        } else {
                            paragraph.translated.clone()
                        });
                }
            }
            Message::Copy => {
                let id = view.id;
                let mut text = view.window.get_current_text();
                if text.is_empty() {
                    text = view.window.get_original();
                }
                self.clipboard.submit(clipboard::Request::WriteText {
                    text: text.to_string(),
                    finish: Box::new(move |result| {
                        send(Message::Status(
                            id,
                            result
                                .map(|()| "Copied to clipboard".to_owned())
                                .unwrap_or_else(|error| error.to_string()),
                        ))
                    }),
                })?;
            }
            Message::Translate(paragraph) if !self.vision.translation_running => {
                let selected = view
                    .paragraphs
                    .iter()
                    .find(|item| item.id == paragraph)
                    .context("Select a paragraph")?;
                let text = selected.source.to_string();
                let request = translation_request(&self.config, text.clone())?;
                let id = view.id;
                let cancel = view.cancelled.clone();
                let namespace = super::settings::namespace(self.identity);
                thread::Builder::new()
                    .name("clippy-image-translation".into())
                    .spawn(move || {
                        let result = translate(request, &namespace, &cancel);
                        send(Message::Translated(id, paragraph, text, result));
                    })?;
                self.vision.translation_running = true;
                view.window.set_busy(true);
                view.window
                    .set_status("Translating the selected paragraph…".into());
            }
            Message::Translated(id, paragraph, source, result) if view.id == id => {
                view.window.set_busy(false);
                let result = result?;
                let region = view
                    .paragraphs
                    .iter_mut()
                    .find(|item| item.id == paragraph)
                    .context("Paragraph no longer exists")?;
                region.translated = result.translated_text.clone().into();
                // 字符膨胀较大的结果留在侧栏，避免覆盖到相邻段落。
                region.in_place =
                    region.translated.chars().count() <= source.chars().count().saturating_mul(2);
                view.update_paragraphs();
                if view.window.get_selected_paragraph() == paragraph {
                    view.window
                        .set_current_text(result.translated_text.clone().into());
                }
                view.window.set_status("Paragraph translated".into());
                self.storage.client().execute(
                    move |db| {
                        db.record_translation(&NewTranslation {
                            clip_id: None,
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
                    move |result| {
                        if let Err(error) = result {
                            send(Message::Status(
                                id,
                                format!("Translation could not be added to history: {error}"),
                            ));
                        }
                    },
                )?;
            }
            Message::Status(id, status) if view.id == id => view.window.set_status(status.into()),
            _ => {}
        }
        Ok(())
    }
}

fn recognize(image: &RgbaImage) -> Result<StructuredOcr> {
    let png = encode_png(image.as_raw(), image.width(), image.height())?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(clippy_vision::recognize_layout(Arc::new(png)))
        .map_err(anyhow::Error::msg)
}

pub(super) fn translation_request(config: &AppConfig, text: String) -> Result<TranslationRequest> {
    if is_sensitive_text(&text) {
        anyhow::bail!("Sensitive text cannot be sent to a translation provider");
    }
    let service = config
        .enabled_translation_services()
        .into_iter()
        .next()
        .context("Enable a translation provider in Settings")?;
    let provider = service.provider.parse::<TranslationProvider>()?;
    let languages = resolve_direction(config, &text, None, None);
    Ok(TranslationRequest::with_options(
        text,
        languages.source,
        languages.target,
        provider,
        ProviderOptions {
            endpoint: service.endpoint.clone(),
            model: Some(service.model.clone()),
            region: Some(service.region.clone()),
            project: Some(service.project.clone()),
            ..ProviderOptions::default()
        },
        0,
    ))
}

pub(super) fn translate(
    request: TranslationRequest,
    namespace: &str,
    cancelled: &AtomicBool,
) -> Result<TranslationResult> {
    let credentials = secrets::get_credentials_in(namespace, request.provider)?;
    if cancelled.load(Ordering::Acquire) {
        return Err(anyhow!("Translation cancelled"));
    }
    Ok(TranslationService::new().translate(request, credentials)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_image_text_is_rejected_before_credentials_or_http() {
        let result = translation_request(&AppConfig::default(), "password: hunter2".into());
        assert!(matches!(result, Err(error) if error.to_string().contains("Sensitive")));
    }

    #[test]
    #[ignore = "需要系统 Tesseract 及英中语言包"]
    fn native_ocr_returns_readable_text_and_source_coordinates() {
        let fixture = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../legacy-tauri/native/ocr-sidecar/quality-fixtures/orientation-v1/images/line-0.png"
        ));
        let image = image::load_from_memory(fixture).unwrap().into_rgba8();
        let result = recognize(&image).unwrap();
        assert!(result.text.contains("OCR"), "{}", result.text);
        assert!(result.text.contains("SN-A1"), "{}", result.text);
        assert!(!result.paragraphs.is_empty());
        let quad = &result.lines[0].quad;
        assert_eq!(quad.len(), 4);
        assert!((20. ..55.).contains(&quad[0][0]), "{quad:?}");
        assert!((40. ..60.).contains(&quad[0][1]), "{quad:?}");
        assert!((400. ..450.).contains(&quad[2][0]), "{quad:?}");
        assert_eq!((result.width, result.height), (720, 150));
    }
}
