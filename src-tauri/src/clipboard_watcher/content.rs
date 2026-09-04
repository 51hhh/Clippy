use sha2::{Digest, Sha256};

/// 允许完整 8K 原图，但拒绝异常剪贴板提供者制造的超大分配。
const MAX_CLIPBOARD_IMAGE_DIMENSION: usize = 16_384;
const MAX_CLIPBOARD_IMAGE_PIXELS: usize = 40_000_000;
const RGBA_BYTES_PER_PIXEL: usize = 4;

pub(super) fn validate_image_layout(
    width: usize,
    height: usize,
    byte_len: usize,
) -> Result<(u32, u32), &'static str> {
    if width == 0 || height == 0 {
        return Err("图片尺寸不能为空");
    }
    if width > MAX_CLIPBOARD_IMAGE_DIMENSION || height > MAX_CLIPBOARD_IMAGE_DIMENSION {
        return Err("图片尺寸超过安全上限");
    }
    let pixels = width.checked_mul(height).ok_or("图片尺寸计算溢出")?;
    if pixels > MAX_CLIPBOARD_IMAGE_PIXELS {
        return Err("图片像素数超过安全上限");
    }
    let expected = pixels
        .checked_mul(RGBA_BYTES_PER_PIXEL)
        .ok_or("图片字节数计算溢出")?;
    if byte_len != expected {
        return Err("图片 RGBA 字节长度与尺寸不匹配");
    }
    let width = u32::try_from(width).map_err(|_| "图片宽度无法编码")?;
    let height = u32::try_from(height).map_err(|_| "图片高度无法编码")?;
    Ok((width, height))
}

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

/// 剪贴板里这张图**这一轮和上一轮是不是同一张**的指纹。
///
/// 只用来做轮询之间的短路，永不入库、永不和 `content_hash` 比较，所以不需要抗碰撞的
/// 密码学哈希：这里要的是"扫一遍 8 MB RGBA 尽量便宜"。轮询每 500 ms 都会走一遍，
/// 而它挡掉的是一整次 PNG 编码（1080p ~77 ms）。
///
/// 宽高一起进哈希：同样的字节按不同宽高解读是不同的图。
pub(crate) fn rgba_fingerprint(width: usize, height: usize, bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    width.hash(&mut hasher);
    height.hash(&mut hasher);
    hasher.write(bytes);
    hasher.finish()
}

/// 将 arboard 的 RGBA 图片数据编码为 PNG 字节。
///
/// 走 `screenshot::encode_png` 而不是自己建 `ImageBuffer` 再 `write_to`：后者要求
/// 拥有所有权，于是先 `to_vec()` 拷一份 8 MB 的 RGBA 才开始编码。全项目只留一个
/// PNG 编码器，watcher 和截图链路编出来的字节也就必然一致。
///
/// 压缩级别不是这里的动机——实测 `write_to` 的默认级别和 `CompressionType::Fast`
/// 在 1080p 上输出字节完全相同、耗时也一样（都是 ~85 ms）。省下的只有那次拷贝。
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

    /// 指纹要认出"同一张图"，也要认出"字节一样但宽高不同"——后者是不同的图，
    /// 若被判成同一张，改完窗口大小重新复制就再也进不了历史。
    #[test]
    fn the_fingerprint_separates_pixels_and_dimensions() {
        let bytes: Vec<u8> = (0..(8 * 4 * 4)).map(|index| (index % 251) as u8).collect();
        let base = rgba_fingerprint(8, 4, &bytes);

        assert_eq!(base, rgba_fingerprint(8, 4, &bytes), "同一张图必须同指纹");
        assert_ne!(base, rgba_fingerprint(4, 8, &bytes), "宽高换了就是另一张图");

        let mut changed = bytes.clone();
        changed[17] = changed[17].wrapping_add(1);
        assert_ne!(base, rgba_fingerprint(8, 4, &changed), "改一个字节要认出来");
    }

    #[test]
    fn html_fallback_drops_markup() {
        assert_eq!(strip_html_tags("<p>Hello <b>world</b></p>"), "Hello world");
    }

    #[test]
    fn image_layout_accepts_4k_and_8k_without_changing_dimensions() {
        for (width, height) in [(3840usize, 2160usize), (7680, 4320)] {
            let byte_len = width * height * RGBA_BYTES_PER_PIXEL;
            assert_eq!(
                validate_image_layout(width, height, byte_len).unwrap(),
                (width as u32, height as u32)
            );
        }
    }

    #[test]
    fn image_layout_rejects_budget_overflow_and_mismatched_rgba() {
        assert!(validate_image_layout(0, 1, 0).is_err());
        assert!(validate_image_layout(usize::MAX, 2, 0).is_err());
        assert!(validate_image_layout(10_000, 5_000, 200_000_000).is_err());
        assert!(validate_image_layout(32, 32, 4095).is_err());
        assert!(validate_image_layout(32, 32, 4097).is_err());
    }

    #[test]
    fn image_layout_accepts_the_pixel_budget_boundary() {
        let width = 8_000usize;
        let height = 5_000usize;
        assert_eq!(
            validate_image_layout(width, height, width * height * RGBA_BYTES_PER_PIXEL).unwrap(),
            (8_000, 5_000)
        );
    }
}
