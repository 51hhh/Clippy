use super::types::TranslationError;
use crate::models::{ClipItem, ContentType};
use crate::storage::StorageEngine;
use std::sync::{Arc, Mutex};

pub(super) struct ClipTranslationInput {
    clip: ClipItem,
    cached_ocr: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PreparedClipText {
    Ready(String),
    NeedsOcr { clip_id: i64, image: Vec<u8> },
}

/// 条目、图片和 OCR 缓存一次锁内读取，避免图片路径再次查询同一 BLOB。
pub(super) fn load_clip_input(
    storage: &Arc<Mutex<StorageEngine>>,
    id: i64,
) -> Result<ClipTranslationInput, TranslationError> {
    let storage = storage.lock().map_err(|_| TranslationError::Internal)?;
    let clip = storage
        .get_clip_by_id(id)
        .map_err(|_| TranslationError::ClipUnavailable)?;
    let cached_ocr = if clip.content_type == ContentType::Image {
        storage
            .get_ocr_text(id)
            .map_err(|_| TranslationError::ClipUnavailable)?
    } else {
        None
    };
    Ok(ClipTranslationInput { clip, cached_ocr })
}

pub(super) fn cache_ocr_text(
    storage: &Arc<Mutex<StorageEngine>>,
    clip_id: i64,
    text: &str,
) -> Result<(), TranslationError> {
    let storage = storage.lock().map_err(|_| TranslationError::Internal)?;
    storage
        .set_ocr_text(clip_id, text)
        .map_err(|_| TranslationError::ClipUnavailable)
}

pub(super) fn prepare_clip_text(
    input: ClipTranslationInput,
) -> Result<PreparedClipText, TranslationError> {
    let ClipTranslationInput { clip, cached_ocr } = input;
    if clip.is_sensitive {
        return Err(TranslationError::SensitiveContent);
    }

    match clip.content_type {
        ContentType::Text => meaningful_text(clip.text_content)
            .map(PreparedClipText::Ready)
            .ok_or(TranslationError::EmptyInput),
        ContentType::Html => meaningful_text(clip.text_content)
            .or_else(|| {
                clip.html_content
                    .as_deref()
                    .map(html_to_plain_text)
                    .filter(|text| !text.trim().is_empty())
            })
            .map(PreparedClipText::Ready)
            .ok_or(TranslationError::EmptyInput),
        ContentType::Image => {
            if let Some(text) = meaningful_text(cached_ocr) {
                return Ok(PreparedClipText::Ready(text));
            }
            clip.image_data
                .map(|image| PreparedClipText::NeedsOcr {
                    clip_id: clip.id,
                    image,
                })
                .ok_or(TranslationError::ImageUnavailable)
        }
    }
}

fn meaningful_text(text: Option<String>) -> Option<String> {
    text.filter(|value| !value.trim().is_empty())
}

fn html_to_plain_text(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut tag = String::new();
    let mut hidden_tag: Option<String> = None;
    for character in html.chars() {
        if character == '<' && !in_tag {
            in_tag = true;
            tag.clear();
            continue;
        }
        if in_tag {
            if character != '>' {
                tag.push(character);
                continue;
            }
            let raw_name = tag.trim().trim_start_matches('/').trim_start();
            let name = raw_name
                .split(|character: char| character.is_whitespace() || character == '/')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            let closing = tag.trim_start().starts_with('/');
            if let Some(hidden) = hidden_tag.as_deref() {
                if closing && name == hidden {
                    hidden_tag = None;
                }
            } else if !closing
                && matches!(
                    name.as_str(),
                    "head" | "script" | "style" | "svg" | "template"
                )
            {
                hidden_tag = Some(name);
            } else {
                output.push(' ');
            }
            in_tag = false;
            continue;
        }
        if hidden_tag.is_none() {
            output.push(character);
        }
    }
    output
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(
        content_type: ContentType,
        text_content: Option<&str>,
        html_content: Option<&str>,
        image_data: Option<Vec<u8>>,
        is_sensitive: bool,
    ) -> ClipItem {
        ClipItem {
            id: 12,
            content_type,
            text_content: text_content.map(str::to_string),
            html_content: html_content.map(str::to_string),
            image_data,
            content_hash: "hash".to_string(),
            is_favorite: false,
            is_sensitive,
            created_at: 0,
            byte_size: 0,
        }
    }

    #[test]
    fn html_prefers_stored_plain_text_and_falls_back_to_markup() {
        let preferred = prepare_clip_text(ClipTranslationInput {
            clip: clip(
                ContentType::Html,
                Some("Stored plain text"),
                Some("<p>Fallback</p>"),
                None,
                false,
            ),
            cached_ocr: None,
        });
        assert_eq!(
            preferred,
            Ok(PreparedClipText::Ready("Stored plain text".to_string()))
        );

        let fallback = prepare_clip_text(ClipTranslationInput {
            clip: clip(
                ContentType::Html,
                None,
                Some("<p>Hello&nbsp;<b>world</b></p>"),
                None,
                false,
            ),
            cached_ocr: None,
        });
        assert_eq!(
            fallback,
            Ok(PreparedClipText::Ready("Hello world".to_string()))
        );
    }

    #[test]
    fn image_reuses_cached_ocr_or_loaded_image_data() {
        let cached = prepare_clip_text(ClipTranslationInput {
            clip: clip(ContentType::Image, None, None, Some(vec![1, 2, 3]), false),
            cached_ocr: Some("Recognized".to_string()),
        });
        assert_eq!(
            cached,
            Ok(PreparedClipText::Ready("Recognized".to_string()))
        );

        let loaded = prepare_clip_text(ClipTranslationInput {
            clip: clip(ContentType::Image, None, None, Some(vec![1, 2, 3]), false),
            cached_ocr: None,
        });
        assert_eq!(
            loaded,
            Ok(PreparedClipText::NeedsOcr {
                clip_id: 12,
                image: vec![1, 2, 3],
            })
        );
    }

    #[test]
    fn sensitive_clip_is_rejected_before_content_selection() {
        let input = ClipTranslationInput {
            clip: clip(ContentType::Image, None, None, None, true),
            cached_ocr: Some("must not leave the device".to_string()),
        };
        assert_eq!(
            prepare_clip_text(input),
            Err(TranslationError::SensitiveContent)
        );
    }

    #[test]
    fn html_fallback_drops_non_visible_document_sections() {
        assert_eq!(
            html_to_plain_text(
                "<head>metadata</head><p>Visible</p><script>secret()</script><style>hidden</style>"
            ),
            "Visible"
        );
    }
}
