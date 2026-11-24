pub(crate) use clippy_core::clipboard::{
    compute_hash, is_sensitive_text, rgba_fingerprint, strip_html_tags, validate_image_layout,
    MAX_CLIPBOARD_TEXT_BYTES,
};

pub(super) fn encode_image_to_png(img: &arboard::ImageData) -> Option<Vec<u8>> {
    let (width, height) = match validate_image_layout(img.width, img.height, img.bytes.len()) {
        Ok(dimensions) => dimensions,
        Err(error) => {
            log::warn!("拒绝异常剪贴板图片: {error}");
            return None;
        }
    };
    match crate::screenshot::encode_png(&img.bytes, width, height) {
        Ok(png) => Some(png),
        Err(error) => {
            log::warn!("PNG 编码失败: {error:#}");
            None
        }
    }
}
