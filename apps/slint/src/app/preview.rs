use super::{clipboard, editor, post, preview_document, settings, vision, App, Event};
use crate::{clip_metadata, truncate_preview, ClipPreview};
use anyhow::Result;
use clippy_core::{
    clipboard::strip_html_tags,
    models::{ClipItem, ContentType},
};
use clippy_storage::{
    BoundedImageData, BoundedImageSnapshot, NewTranslation, StorageEngine, StorageError,
};
use clippy_translate::types::TranslationResult;
use image::{DynamicImage, ImageError, ImageFormat, ImageReader, RgbaImage};
use slint::{ComponentHandle, LogicalSize, StyledText};
use std::{
    io::Cursor,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
};

pub(super) const MAX_PNG_BYTES: usize = 16 * 1024 * 1024;
const MAX_DECODED_BYTES: u64 = 48 * 1024 * 1024;
const MAX_IMAGE_EDGE: u32 = 8192;
const PREVIEW_EDGE: u32 = 800;
const MAX_TEXT_CHARS: usize = 32_768;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Request {
    id: i64,
    generation: u64,
}

pub(super) enum Message {
    Show(bool),
    Selection,
    Loaded(Request, Result<Details>),
    Recognized(Request, Result<String>),
    Translate(String),
    Translated(Request, String, Result<TranslationResult>),
    CopyText(String),
}

#[derive(Default)]
pub(super) struct Preview {
    selected: Option<Request>,
    generation: u64,
    in_flight: bool,
    ocr: Option<Arc<AtomicBool>>,
    pending_ocr: Option<(Request, Arc<RgbaImage>)>,
    translation: Option<Arc<AtomicBool>>,
}

impl Preview {
    pub(super) fn select(&mut self, id: Option<i64>) -> bool {
        if self.selected.map(|request| request.id) == id {
            return false;
        }
        self.generation += 1;
        self.pending_ocr = None;
        if let Some(cancel) = &self.ocr {
            cancel.store(true, Ordering::Release);
        }
        if let Some(cancel) = self.translation.take() {
            cancel.store(true, Ordering::Release);
        }
        self.selected = id.map(|id| Request {
            id,
            generation: self.generation,
        });
        true
    }

    fn begin(&mut self) -> Option<Request> {
        if self.in_flight {
            return None;
        }
        let request = self.selected?;
        self.in_flight = true;
        Some(request)
    }

    fn finish(&mut self, request: Request) -> bool {
        self.in_flight = false;
        self.selected == Some(request)
    }
}

pub(super) struct Details {
    metadata: String,
    kind: &'static str,
    body: Body,
}

struct PreviewInput {
    clip: ClipItem,
    snapshot: Option<BoundedImageSnapshot>,
    cached_ocr: Option<String>,
}

enum Body {
    Text(String, Option<StyledText>),
    Image {
        picture: RgbaImage,
        width: u32,
        height: u32,
        ocr: Option<String>,
        source: Option<Arc<RgbaImage>>,
    },
    Protected,
    Unavailable(&'static str),
}

impl Details {
    fn ocr_input(&self) -> Option<Arc<RgbaImage>> {
        match &self.body {
            Body::Image { source, .. } => source.clone(),
            _ => None,
        }
    }

    fn into_model(self, id: i64) -> ClipPreview {
        let mut model = ClipPreview {
            clip_id: id.to_string().into(),
            kind: self.kind.into(),
            metadata: self.metadata.into(),
            ..Default::default()
        };
        match self.body {
            Body::Text(text, styled) => {
                model.content = text.into();
                model.has_styles = styled.is_some();
                model.styled_content = styled.unwrap_or_default();
            }
            Body::Image {
                picture,
                width,
                height,
                ocr,
                source,
            } => {
                model.image = editor::to_slint(&picture);
                model.metadata = format!("{width} × {height} · {}", model.metadata).into();
                model.recognized = ocr.unwrap_or_default().into();
                model.ocr_busy = source.is_some();
            }
            Body::Protected => model.protected = true,
            Body::Unavailable(message) => model.status = message.into(),
        }
        model
    }
}

impl App {
    pub(super) fn preview_message(&mut self, message: Message) {
        match message {
            Message::Show(open) => {
                let Some(window) = self
                    .window
                    .as_ref()
                    .filter(|window| !window.get_show_settings())
                else {
                    return;
                };
                if window.get_preview_open() != open {
                    window.set_preview_open(open);
                    let native = window.window();
                    let height = native.size().to_logical(native.scale_factor()).height;
                    native.set_size(LogicalSize::new(window.get_panel_width(), height));
                    self.refresh_preview();
                }
            }
            Message::Selection => self.refresh_preview(),
            Message::Loaded(request, result) => {
                if !self.preview.finish(request) {
                    self.load_preview();
                    return;
                }
                let window = self.window.as_ref().expect("请求对应已展开的预览侧栏");
                if window.get_selected_sensitive()
                    || window.get_selected_id().parse() != Ok(request.id)
                {
                    return;
                }
                window.set_preview(match result {
                    Ok(details) => {
                        self.preview.pending_ocr =
                            details.ocr_input().map(|source| (request, source));
                        details.into_model(request.id)
                    }
                    Err(error) => {
                        eprintln!("Clipboard preview unavailable: {error}");
                        ClipPreview {
                            clip_id: request.id.to_string().into(),
                            status: "Preview unavailable".into(),
                            ..Default::default()
                        }
                    }
                });
                self.start_preview_ocr();
            }
            Message::Recognized(request, result) => {
                self.preview.ocr = None;
                if self.preview.selected == Some(request) {
                    if let Some(window) = self.window.as_ref() {
                        let mut preview = window.get_preview();
                        preview.ocr_busy = false;
                        match result {
                            Ok(text) => {
                                preview.recognized = truncate_preview(&text, MAX_TEXT_CHARS).into();
                                if self.vision.native.is_none() {
                                    let _ = self.storage.client().execute(
                                        move |db| db.set_ocr_text(request.id, &text),
                                        |_| {},
                                    );
                                }
                            }
                            Err(error) => preview.ocr_note = error.to_string().into(),
                        }
                        window.set_preview(preview);
                    }
                }
                self.start_preview_ocr();
            }
            Message::Translate(source) => {
                if let Err(error) = self.translate_preview(source) {
                    self.set_preview_translation_error(error.to_string());
                }
            }
            Message::Translated(request, source, result) => {
                if self.preview.selected != Some(request) {
                    return;
                }
                self.preview.translation.take();
                match result {
                    Ok(translation) => {
                        let window = self.window.as_ref().expect("翻译结果对应已打开的侧栏");
                        let mut preview = window.get_preview();
                        preview.translated =
                            truncate_preview(&translation.translated_text, MAX_TEXT_CHARS).into();
                        preview.translation_busy = false;
                        window.set_preview(preview);
                        let _ = self.storage.client().execute(
                            move |db| {
                                db.record_translation(&NewTranslation {
                                    clip_id: Some(request.id),
                                    provider: translation.provider.as_str(),
                                    source_language: translation
                                        .detected_source_language
                                        .as_deref()
                                        .unwrap_or("auto"),
                                    target_language: &translation.target_language,
                                    source_text: &source,
                                    translated_text: &translation.translated_text,
                                })
                            },
                            |_| {},
                        );
                    }
                    Err(error) => self.set_preview_translation_error(error.to_string()),
                }
            }
            Message::CopyText(text) => {
                if let Err(error) = self.clipboard.submit(clipboard::Request::WriteText {
                    text,
                    finish: Box::new(|result| {
                        let status =
                            result.map_or_else(|error| error.to_string(), |()| "Copied".into());
                        post(Event::ClipboardStatus(status));
                    }),
                }) {
                    self.status(error.to_string());
                }
            }
        }
    }

    fn refresh_preview(&mut self) {
        let Some(window) = &self.window else { return };
        let id = window.get_selected_id();
        let selected = (window.get_preview_open()
            && !window.get_selected_sensitive()
            && !id.is_empty())
        .then(|| {
            id.parse()
                .expect("The clipboard model carries database IDs")
        });
        if self.preview.select(selected) {
            window.set_preview(ClipPreview::default());
            self.load_preview();
        }
    }

    fn load_preview(&mut self) {
        let Some(request) = self.preview.begin() else {
            return;
        };
        let ocr_enabled = self.config.ocr_enabled;
        let use_legacy_cache = self.vision.native.is_none();
        let result = self.storage.client().execute(
            move |db| read_details(db, request.id, use_legacy_cache),
            move |result| match result {
                Ok(PreviewInput {
                    clip,
                    snapshot,
                    cached_ocr,
                }) => {
                    if (clip.content_type == ContentType::Html && !clip.is_sensitive)
                        || snapshot.as_ref().is_some_and(|item| {
                            !item.is_sensitive && matches!(item.image, BoundedImageData::Bytes(_))
                        })
                    {
                        // 解码不占用 SQLite 线程，快速选中多项时只保留最新一项候选。
                        if let Err(error) = thread::Builder::new()
                            .name("clippy-preview".into())
                            .spawn(move || {
                                loaded(
                                    request,
                                    Ok(prepare(clip, snapshot, ocr_enabled, cached_ocr)),
                                )
                            })
                        {
                            loaded(request, Err(error.into()));
                        }
                    } else {
                        loaded(
                            request,
                            Ok(prepare(clip, snapshot, ocr_enabled, cached_ocr)),
                        );
                    }
                }
                Err(error) => loaded(request, Err(error.into())),
            },
        );
        if let Err(error) = result {
            loaded(request, Err(error.into()));
        }
    }

    fn start_preview_ocr(&mut self) {
        if self.preview.ocr.is_some() {
            return;
        }
        let Some((request, source)) = self.preview.pending_ocr.take() else {
            return;
        };
        let native = self.vision.native_engine();
        let cancelled = Arc::new(AtomicBool::new(false));
        self.preview.ocr = Some(cancelled.clone());
        if let Err(error) = thread::Builder::new()
            .name("clippy-preview-ocr".into())
            .spawn(move || {
                let result = vision::recognize(source, native, cancelled).map(|result| result.text);
                post(Event::Preview(Message::Recognized(request, result)));
            })
        {
            post(Event::Preview(Message::Recognized(
                request,
                Err(error.into()),
            )));
        }
    }

    fn translate_preview(&mut self, source: String) -> Result<()> {
        let Some(request) = self.preview.selected else {
            return Ok(());
        };
        if self.preview.translation.is_some() {
            return Ok(());
        }
        let translation = vision::translation_request(&self.config, source.clone())?;
        let namespace = settings::namespace(self.identity);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        thread::Builder::new()
            .name("clippy-preview-translation".into())
            .spawn(move || {
                let result = vision::translate(translation, &namespace, &worker_cancel);
                post(Event::Preview(Message::Translated(request, source, result)));
            })?;
        self.preview.translation = Some(cancel);
        let window = self.window.as_ref().expect("翻译由侧栏触发");
        let mut preview = window.get_preview();
        preview.translation_busy = true;
        preview.translation_note = Default::default();
        window.set_preview(preview);
        Ok(())
    }

    fn set_preview_translation_error(&self, error: String) {
        if let Some(window) = self.window.as_ref() {
            let mut preview = window.get_preview();
            preview.translation_busy = false;
            preview.translation_note = error.into();
            window.set_preview(preview);
        }
    }
}

fn loaded(request: Request, result: Result<Details>) {
    post(Event::Preview(Message::Loaded(request, result)));
}

fn read_details(
    db: &StorageEngine,
    id: i64,
    use_legacy_cache: bool,
) -> Result<PreviewInput, StorageError> {
    let clip = db.get_clip_detail_by_id(id)?;
    let snapshot = if clip.content_type == ContentType::Image && !clip.is_sensitive {
        db.get_bounded_image_snapshot(id, MAX_PNG_BYTES)?
    } else {
        None
    };
    let cached_ocr = if use_legacy_cache && snapshot.is_some() {
        db.get_ocr_text(id)?
    } else {
        None
    };
    Ok(PreviewInput {
        clip,
        snapshot,
        cached_ocr,
    })
}

fn prepare(
    clip: ClipItem,
    snapshot: Option<BoundedImageSnapshot>,
    ocr_enabled: bool,
    ocr: Option<String>,
) -> Details {
    let kind = match clip.content_type {
        ContentType::Text => "Text",
        ContentType::Html => "Rich text",
        ContentType::Image => "Image",
    };
    let body = if clip.is_sensitive || snapshot.as_ref().is_some_and(|item| item.is_sensitive) {
        Body::Protected
    } else {
        match clip.content_type {
            ContentType::Text | ContentType::Html => {
                let styled = clip
                    .html_content
                    .as_deref()
                    .and_then(preview_document::html);
                let text = clip.text_content.unwrap_or_else(|| {
                    clip.html_content
                        .as_deref()
                        .map(strip_html_tags)
                        .unwrap_or_default()
                });
                Body::Text(truncate_preview(&text, MAX_TEXT_CHARS), styled)
            }
            ContentType::Image => match snapshot.map(|item| item.image) {
                Some(BoundedImageData::Bytes(bytes)) => match decode_preview_png(&bytes) {
                    Ok(image) => {
                        let (width, height) = (image.width(), image.height());
                        let needs_ocr = ocr_enabled && ocr.is_none() && width >= 32 && height >= 16;
                        let (picture, source) = if needs_ocr {
                            let picture = image.thumbnail(PREVIEW_EDGE, PREVIEW_EDGE).into_rgba8();
                            (picture, Some(Arc::new(image.into_rgba8())))
                        } else {
                            (resize_preview(image, PREVIEW_EDGE), None)
                        };
                        Body::Image {
                            picture,
                            width,
                            height,
                            ocr,
                            source,
                        }
                    }
                    Err(ImageError::Limits(_)) => Body::Unavailable("Image too large to preview"),
                    Err(_) => Body::Unavailable("Could not preview image"),
                },
                Some(BoundedImageData::TooLarge) => Body::Unavailable("Image too large to preview"),
                _ => Body::Unavailable("Image data unavailable"),
            },
        }
    };
    Details {
        metadata: clip_metadata(clip.byte_size, clip.created_at),
        kind,
        body,
    }
}

pub(super) fn thumbnail(bytes: &[u8], edge: u32) -> Result<RgbaImage, ImageError> {
    decode_preview_png(bytes).map(|image| resize_preview(image, edge))
}

fn decode_preview_png(bytes: &[u8]) -> Result<DynamicImage, ImageError> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_EDGE);
    limits.max_image_height = Some(MAX_IMAGE_EDGE);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    reader.decode()
}

fn resize_preview(image: DynamicImage, edge: u32) -> RgbaImage {
    if image.width() > edge || image.height() > edge {
        image.thumbnail(edge, edge).into_rgba8()
    } else {
        image.into_rgba8()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clippy_capture::png_io::encode_png;

    #[test]
    fn selection_updates_coalesce_and_closed_requests_do_not_publish() {
        let mut preview = Preview::default();
        assert!(preview.select(Some(1)));
        let first = preview.begin().unwrap();
        assert!(preview.select(Some(2)));
        assert!(preview.select(Some(3)));
        assert!(preview.begin().is_none());
        assert!(!preview.finish(first));
        let third = preview.begin().unwrap();
        assert_eq!(third.id, 3);

        assert!(preview.select(None));
        assert!(preview.select(Some(3)));
        assert!(!preview.finish(third));
        let reopened = preview.begin().unwrap();
        assert_eq!(reopened.id, 3);
        assert!(!preview.select(Some(3)));
        assert!(preview.finish(reopened));
    }

    #[test]
    fn preview_decodes_and_downscales_rgba_with_an_allocation_limit() {
        let original = [255, 0, 0, 128, 0, 128, 255, 0];
        let image = thumbnail(&encode_png(&original, 2, 1).unwrap(), PREVIEW_EDGE).unwrap();
        assert_eq!(image.as_raw(), &original);
        let wide = encode_png(&vec![128; 1_200 * 18 * 4], 1_200, 18).unwrap();
        let reduced = thumbnail(&wide, PREVIEW_EDGE).unwrap();
        assert_eq!(reduced.dimensions(), (800, 12));
        assert_eq!(reduced.get_pixel(0, 0)[3], 128);

        let width = MAX_IMAGE_EDGE + 1;
        let oversized = encode_png(&vec![0; width as usize * 4], width, 1).unwrap();
        assert!(matches!(
            thumbnail(&oversized, PREVIEW_EDGE),
            Err(ImageError::Limits(_))
        ));
        assert!(thumbnail(b"invalid PNG", PREVIEW_EDGE).is_err());
    }

    #[test]
    fn rich_text_is_plain_and_sensitive_body_does_not_enter_the_ui_model() {
        let clip = ClipItem {
            id: 9,
            content_type: ContentType::Html,
            text_content: None,
            html_content: Some("<b>中文</b><img src='https://invalid.test/sample.png'>".into()),
            image_data: None,
            content_hash: "fixture".into(),
            is_favorite: false,
            created_at: 0,
            byte_size: 70,
            is_sensitive: false,
        };
        let model = prepare(clip.clone(), None, false, None).into_model(clip.id);
        assert_eq!(model.content.as_str(), "中文");
        assert_eq!(model.kind.as_str(), "Rich text");
        let protected = ClipItem {
            is_sensitive: true,
            ..clip
        };
        let model = prepare(protected, None, false, None).into_model(9);
        assert!(model.protected);
        assert!(model.content.is_empty());
    }
}
