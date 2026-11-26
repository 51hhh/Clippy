use super::{clipboard, editor, post, settings, App, Event};
use crate::{
    locale::tr, ClippyTheme, CodeMarker, CodeMatch, ImageTextWindow, TextBox, TextParagraph,
};
use anyhow::{anyhow, Context, Result};
use clippy_capture::png_io::encode_png;
use clippy_core::{clipboard::is_sensitive_text, config::AppConfig};
use clippy_platform::identity::DistributionChannel;
use clippy_storage::NewTranslation;
use clippy_translate::{
    direction::resolve_direction,
    secrets,
    types::{ProviderOptions, TranslationProvider, TranslationRequest, TranslationResult},
    TranslationService,
};
use clippy_vision::{code_detection, NativeOcr, StructuredOcr};
use image::RgbaImage;
use slint::{CloseRequestResponse, ComponentHandle, Model, ModelRc, VecModel};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
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
    SelectCode(u64, i32),
    Copy(u64, Option<i32>),
    Translate,
    Translated(u64, Selection, String, Result<TranslationResult>),
    Status(u64, String),
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Selection {
    Line(i32),
    Paragraph(i32),
}

impl Selection {
    fn name(self) -> &'static str {
        match self {
            Self::Line(_) => "line",
            Self::Paragraph(_) => "paragraph",
        }
    }
}

pub(super) enum Recognition {
    Ocr(StructuredOcr),
    Codes(code_detection::CodeScanResponse),
}

#[derive(Default)]
pub(super) struct Vision {
    pub(super) native: Option<Arc<NativeOcr>>,
    qa_override: bool,
    serial: u64,
    recognition_running: bool,
    translation_running: bool,
    window: Option<View>,
}

impl Vision {
    pub(super) fn new(channel: DistributionChannel) -> Self {
        let explicit = if channel == DistributionChannel::Qa {
            std::env::var_os("CLIPPY_QA_OCR_DIR").map(PathBuf::from)
        } else {
            None
        };
        let qa_override = explicit.is_some();
        let native = explicit
            .or_else(|| bundled_ocr_dir(&std::env::current_exe().ok()?))
            .map(|path| Arc::new(NativeOcr::new(path)));
        Self {
            native,
            qa_override,
            ..Self::default()
        }
    }

    pub(super) fn native_engine(&self) -> Option<Arc<NativeOcr>> {
        // 仅已显式配置的增强引擎覆盖随包默认值，升级时可继续使用原有模型。
        if self.qa_override || !clippy_vision::uses_enhanced_configuration() {
            self.native.clone()
        } else {
            None
        }
    }

    pub(super) fn start_recognition(
        &mut self,
        start: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<()> {
        anyhow::ensure!(
            !self.recognition_running,
            "{}",
            tr("An image is already being analyzed")
        );
        start(self)?;
        self.recognition_running = true;
        Ok(())
    }

    pub(super) fn recognition_finished(&mut self) {
        self.recognition_running = false;
    }

    pub(super) fn shutdown(&self) {
        if let Some(native) = &self.native {
            native.shutdown();
        }
    }
}

pub(super) fn code_match(index: usize, code: code_detection::CodeScanResult) -> CodeMatch {
    let format = match code.format.as_str() {
        "qr_code" => "QR Code",
        "code_39" => "Code 39",
        "code_128" => "Code 128",
        "ean_13" => "EAN-13",
        format => format,
    };
    CodeMatch {
        id: index as i32,
        format: format.into(),
        text: code.text.into(),
    }
}

fn bundled_ocr_dir(executable: &Path) -> Option<PathBuf> {
    let bin = executable.parent()?;
    #[cfg(target_os = "linux")]
    let resources = bin.parent()?.join("lib").join(executable.file_name()?);
    #[cfg(target_os = "macos")]
    let resources = bin.parent()?.join("Resources");
    #[cfg(target_os = "windows")]
    let resources = bin;
    let bundle = resources.join("ocr");
    bundle.is_dir().then_some(bundle)
}

struct View {
    id: u64,
    window: ImageTextWindow,
    lines: HashMap<i32, (i32, String)>,
    boxes: Vec<TextBox>,
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
    fn open(id: u64, image: &RgbaImage, config: &AppConfig, mode: Mode) -> Result<Self> {
        let window = ImageTextWindow::new()?;
        let appearance = window.global::<ClippyTheme>();
        appearance.set_index(settings::theme_index(&config.theme));
        settings::connect_window_frame(&window, appearance);
        window.set_picture(editor::to_slint(image));
        window.set_image_width(image.width() as f32);
        window.set_image_height(image.height() as f32);
        window.set_scan_mode(matches!(mode, Mode::Scan));
        window.set_busy(true);
        window.set_status(tr("Reading image…").into());
        if let Some(service) = config.enabled_translation_services().first() {
            window.set_service(service.provider.clone().into());
        }
        window.on_select_line(|line| send(Message::SelectLine(line)));
        window.on_select_paragraph(|paragraph| send(Message::SelectParagraph(paragraph)));
        window.on_select_code(move |code| send(Message::SelectCode(id, code)));
        window.on_copy_code(move |code| send(Message::Copy(id, Some(code))));
        window.on_copy_result(move || send(Message::Copy(id, None)));
        window.on_translate(|| send(Message::Translate));
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
            boxes: Vec::new(),
            paragraphs: Vec::new(),
            cancelled: Arc::new(AtomicBool::new(false)),
        })
    }

    fn display(&mut self, result: Recognition) {
        match result {
            Recognition::Codes(result) => self.display_codes(result),
            Recognition::Ocr(result) => self.layout(result),
        }
    }

    fn display_codes(&self, result: code_detection::CodeScanResponse) {
        let width = self.window.get_image_width();
        let height = self.window.get_image_height();
        let mut markers = Vec::new();
        let mut matches = Vec::with_capacity(result.results.len());
        for (index, code) in result.results.into_iter().enumerate() {
            let id = index as i32;
            markers.extend(code.points.iter().map(|point| CodeMarker {
                id,
                x: point.x / width,
                y: point.y / height,
            }));
            matches.push(code_match(index, code));
        }
        let original = matches
            .iter()
            .map(|code| code.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        self.window.set_status(if matches.is_empty() {
            tr("No barcode found").into()
        } else {
            Default::default()
        });
        self.window.set_original(original.into());
        self.window.set_scan_limited(result.limited);
        self.window
            .set_code_matches(ModelRc::new(VecModel::from(matches)));
        self.window
            .set_code_markers(ModelRc::new(VecModel::from(markers)));
    }

    fn select_code(&self, id: i32) -> slint::SharedString {
        let code = self
            .window
            .get_code_matches()
            .row_data(id as usize)
            .expect("扫码操作来自当前结果列表");
        self.window.set_selected_code(id);
        self.window.set_current_text(code.text.clone());
        code.text
    }

    fn layout(&mut self, mut result: StructuredOcr) {
        let (width, height) = (f64::from(result.width), f64::from(result.height));
        result.lines.sort_by_key(|line| line.reading_order);
        result
            .paragraphs
            .sort_by_key(|paragraph| paragraph.reading_order);
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
            self.boxes.push(TextBox {
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
                translated: Default::default(),
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
                })
            })
            .collect();
        self.update_boxes();
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

    fn update_boxes(&self) {
        self.window
            .set_boxes(ModelRc::new(VecModel::from(self.boxes.clone())));
    }

    fn selection(&self) -> Option<Selection> {
        let line = self.window.get_selected_line();
        let paragraph = self.window.get_selected_paragraph();
        if line >= 0 {
            Some(Selection::Line(line))
        } else {
            (paragraph >= 0).then_some(Selection::Paragraph(paragraph))
        }
    }

    fn remember_translation(&mut self, selection: Selection, translated: &str) {
        match selection {
            Selection::Line(id) => {
                let region = self
                    .boxes
                    .iter_mut()
                    .find(|region| region.line == id)
                    .expect("当前窗口保留被翻译的行");
                region.translated = translated.into();
                self.update_boxes();
            }
            Selection::Paragraph(id) => {
                let region = self
                    .paragraphs
                    .iter_mut()
                    .find(|region| region.id == id)
                    .expect("当前窗口保留被翻译的段落");
                region.translated = translated.into();
                self.update_paragraphs();
            }
        }
        if self.selection() == Some(selection) {
            self.window.set_current_text(translated.into());
        }
    }
}

impl App {
    pub(super) fn analyze_image(&mut self, image: Arc<RgbaImage>, mode: Mode) -> Result<()> {
        let config = &self.config;
        self.vision.start_recognition(|vision| {
            if matches!(mode, Mode::Recognize) && !config.ocr_enabled {
                anyhow::bail!("OCR is disabled in Settings");
            }
            vision.serial += 1;
            let id = vision.serial;
            let new_view = View::open(id, &image, config, mode)?;
            let native = vision.native_engine();
            let cancelled = new_view.cancelled.clone();
            thread::Builder::new()
                .name("clippy-image-analysis".into())
                .spawn(move || {
                    let result = match mode {
                        Mode::Scan => code_detection::scan_rgba(
                            image.as_raw().clone(),
                            image.width(),
                            image.height(),
                        )
                        .map(Recognition::Codes)
                        .map_err(Into::into),
                        Mode::Recognize => {
                            recognize(image, native, cancelled).map(Recognition::Ocr)
                        }
                    };
                    send(Message::Recognized(id, result));
                })?;
            vision.window = Some(new_view);
            Ok(())
        })
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
            Message::Recognized(..) => self.vision.recognition_finished(),
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
            Message::SelectCode(id, code) if view.id == id => {
                view.select_code(code);
            }
            Message::Copy(id, code) if view.id == id => {
                let text = match code {
                    Some(code) => view.select_code(code),
                    None if view.window.get_scan_mode() => view.window.get_original(),
                    None => {
                        let selected = view.window.get_current_text();
                        if selected.is_empty() {
                            view.window.get_original()
                        } else {
                            selected
                        }
                    }
                };
                self.clipboard.submit(clipboard::Request::WriteText {
                    text: text.to_string(),
                    finish: Box::new(move |result| {
                        send(Message::Status(
                            id,
                            result
                                .map(|()| tr("Copied to clipboard").to_owned())
                                .unwrap_or_else(|error| error.to_string()),
                        ))
                    }),
                })?;
            }
            Message::Translate if !self.vision.translation_running => {
                let selection = view.selection().context("Select a line or paragraph")?;
                let text = match selection {
                    Selection::Line(id) => view.lines[&id].1.clone(),
                    Selection::Paragraph(id) => view
                        .paragraphs
                        .iter()
                        .find(|region| region.id == id)
                        .expect("所选段落来自当前窗口")
                        .source
                        .to_string(),
                };
                let request = translation_request(&self.config, text.clone())?;
                let id = view.id;
                let cancel = view.cancelled.clone();
                let namespace = super::settings::namespace(self.identity);
                thread::Builder::new()
                    .name("clippy-image-translation".into())
                    .spawn(move || {
                        let result = translate(request, &namespace, &cancel);
                        send(Message::Translated(id, selection, text, result));
                    })?;
                self.vision.translation_running = true;
                view.window.set_busy(true);
                view.window
                    .set_status(format!("Translating the selected {}…", selection.name()).into());
            }
            Message::Translated(id, selection, source, result) if view.id == id => {
                view.window.set_busy(false);
                let result = result?;
                view.remember_translation(selection, &result.translated_text);
                view.window
                    .set_status(format!("Translated {}", selection.name()).into());
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

pub(super) fn recognize(
    image: Arc<RgbaImage>,
    native: Option<Arc<NativeOcr>>,
    cancelled: Arc<AtomicBool>,
) -> Result<StructuredOcr> {
    if cancelled.load(Ordering::Acquire) {
        anyhow::bail!("OCR cancelled");
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    if let Some(native) = native {
        return runtime
            .block_on(native.recognize(image, cancelled))
            .map_err(anyhow::Error::msg);
    }
    let png = encode_png(image.as_raw(), image.width(), image.height())?;
    runtime
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
    fn packaged_ocr_directory_is_relative_to_the_installed_executable() {
        let install = tempfile::tempdir().unwrap();
        #[cfg(target_os = "linux")]
        let (binary, resource) = ("usr/bin/clippy-app", "usr/lib/clippy-app/ocr");
        #[cfg(target_os = "macos")]
        let (binary, resource) = (
            "Clippy.app/Contents/MacOS/clippy-app",
            "Clippy.app/Contents/Resources/ocr",
        );
        #[cfg(target_os = "windows")]
        let (binary, resource) = ("Clippy/clippy-app.exe", "Clippy/ocr");
        let executable = install.path().join(binary);
        let bundle = install.path().join(resource);
        assert_eq!(bundled_ocr_dir(&executable), None);
        std::fs::create_dir_all(&bundle).unwrap();
        assert_eq!(bundled_ocr_dir(&executable), Some(bundle));
    }

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
        let result = recognize(Arc::new(image), None, Arc::new(AtomicBool::new(false))).unwrap();
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
