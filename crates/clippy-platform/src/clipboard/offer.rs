use super::{Content, ImageData};
use anyhow::{ensure, Context, Result};
use clippy_core::clipboard::{
    validate_image_layout, MAX_CLIPBOARD_PNG_BYTES, MAX_CLIPBOARD_TEXT_BYTES,
};
use image::{
    codecs::png::{CompressionType, FilterType, PngEncoder},
    ExtendedColorType, ImageEncoder, ImageFormat, ImageReader,
};
use std::{borrow::Cow, io::Cursor};

pub(crate) const PNG: &str = "image/png";
pub(crate) const HTML: &str = "text/html";
pub(crate) const TEXT: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];

pub enum Offer {
    Text(String),
    Html { html: String, text: String },
    Png(Vec<u8>),
}

impl Offer {
    pub fn encode(content: Content) -> Result<Self> {
        match content {
            Content::Text(text) => Ok(Self::Text(text)),
            Content::Html { html, text } => Ok(Self::Html { html, text }),
            Content::Image(image) => Self::encode_image(image),
        }
    }

    pub fn encode_image(image: ImageData<'_>) -> Result<Self> {
        let (width, height) = validate_image_layout(image.width, image.height, image.bytes.len())
            .map_err(anyhow::Error::msg)?;
        let mut png = Vec::with_capacity(image.bytes.len().min(1024 * 1024));
        PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Adaptive)
            .write_image(&image.bytes, width, height, ExtendedColorType::Rgba8)?;
        Ok(Self::Png(png))
    }

    pub(crate) fn types(&self) -> &[&str] {
        match self {
            Self::Text(_) => &TEXT,
            Self::Html { .. } => &[HTML, TEXT[0], TEXT[1], TEXT[2]],
            Self::Png(_) => &[PNG],
        }
    }

    pub(crate) fn bytes(&self, mime: &str) -> Option<&[u8]> {
        match self {
            Self::Text(text) if TEXT.contains(&mime) => Some(text.as_bytes()),
            Self::Html { html, .. } if mime == HTML => Some(html.as_bytes()),
            Self::Html { text, .. } if TEXT.contains(&mime) => Some(text.as_bytes()),
            Self::Png(png) if mime == PNG => Some(png),
            _ => None,
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        for mime in self.types() {
            ensure!(
                self.bytes(mime).expect("Advertised clipboard format").len() <= limit(mime),
                "Clipboard data exceeds its size limit"
            );
        }
        Ok(())
    }

    pub(crate) fn decode(&self) -> Result<Content> {
        Ok(match self {
            Self::Text(text) => Content::Text(text.clone()),
            Self::Html { html, text } => Content::Html {
                html: html.clone(),
                text: text.clone(),
            },
            Self::Png(png) => {
                let reader = || ImageReader::with_format(Cursor::new(png), ImageFormat::Png);
                let (width, height) = reader().into_dimensions()?;
                let (width, height) = (width as usize, height as usize);
                let bytes = width
                    .checked_mul(height)
                    .and_then(|n| n.checked_mul(4))
                    .context("PNG dimensions overflow")?;
                validate_image_layout(width, height, bytes).map_err(anyhow::Error::msg)?;
                Content::Image(ImageData {
                    width,
                    height,
                    bytes: Cow::Owned(reader().decode()?.into_rgba8().into_raw()),
                })
            }
        })
    }
}

pub(crate) fn limit(mime: &str) -> usize {
    if mime == PNG {
        MAX_CLIPBOARD_PNG_BYTES
    } else {
        MAX_CLIPBOARD_TEXT_BYTES
    }
}
