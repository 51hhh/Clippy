use anyhow::{Context, Result};
use image::{
    codecs::png::{CompressionType, FilterType, PngEncoder},
    ExtendedColorType, ImageEncoder,
};

pub fn encode_png(bytes: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let mut png = Vec::with_capacity(bytes.len().min(1024 * 1024));
    PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Adaptive)
        .write_image(bytes, width, height, ExtendedColorType::Rgba8)
        .context("PNG 编码失败")?;
    Ok(png)
}

#[cfg(test)]
pub(crate) fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    Ok(
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png)
            .into_dimensions()?,
    )
}

#[cfg(test)]
pub(crate) fn validate_png(bytes: &[u8]) -> Result<(u32, u32)> {
    use image::GenericImageView;
    Ok(image::load_from_memory_with_format(bytes, image::ImageFormat::Png)?.dimensions())
}
