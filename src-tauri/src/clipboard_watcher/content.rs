use image::{ImageBuffer, RgbaImage};
use sha2::{Digest, Sha256};
use std::io::Cursor;

/// 简单去除 HTML 标签，用于生成 FTS 可搜索的纯文本。
pub(crate) fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
}

/// 检测文本是否可能包含敏感内容（密码、Token、API Key 等）。
pub(crate) fn is_sensitive_text(text: &str) -> bool {
    if text.len() < 8 {
        return false;
    }
    const PREFIXES: &[&str] = &[
        "sk-",
        "sk_live_",
        "sk_test_",
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "github_pat_",
        "AKIA",
        "Bearer ",
        "eyJ",
        "xox",
        "glpat-",
        "npm_",
        "pypi-",
    ];
    if PREFIXES.iter().any(|prefix| text.starts_with(prefix)) {
        return true;
    }

    let lower = text.to_lowercase();
    (lower.contains("password") || lower.contains("passwd") || lower.contains("secret"))
        && (lower.contains('=') || lower.contains(':'))
}

pub(crate) fn compute_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// 将 arboard 的 RGBA 图片数据编码为 PNG 字节。
pub(super) fn encode_image_to_png(img: &arboard::ImageData) -> Option<Vec<u8>> {
    let buffer: RgbaImage =
        ImageBuffer::from_raw(img.width as u32, img.height as u32, img.bytes.to_vec())?;
    let mut png_bytes = Vec::new();
    let mut cursor = Cursor::new(&mut png_bytes);
    if let Err(error) = buffer.write_to(&mut cursor, image::ImageFormat::Png) {
        log::warn!("PNG 编码失败: {}", error);
        return None;
    }
    Some(png_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_detection_covers_tokens_and_assignments() {
        assert!(is_sensitive_text("ghp_1234567890"));
        assert!(is_sensitive_text("password: hunter2"));
        assert!(!is_sensitive_text("ordinary clipboard text"));
        assert!(!is_sensitive_text("short"));
    }

    #[test]
    fn html_fallback_drops_markup() {
        assert_eq!(strip_html_tags("<p>Hello <b>world</b></p>"), "Hello world");
    }
}
