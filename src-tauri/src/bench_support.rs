//! criterion 基准用的内部入口。
//!
//! 这里是 `benches/` 唯一能碰到的内部实现：模块本身在 crate 里是私有的，
//! 基准通过这些 re-export/wrapper 调用真实生产代码，而不是在 bench 文件里
//! 复制一份实现——复制出来的基准量的是副本，和生产路径分叉后毫无意义。
//!
//! **不是稳定 API**：这里出现什么完全由基准需要决定，随时可以改。

pub use crate::models::{ClipItem, ContentType};
pub use crate::screenshot::{decode_png_base64, encode_png, png_dimensions, validate_png};
pub use crate::storage::StorageEngine;

/// 剪贴板去重哈希。轮询线程每次拿到新内容都要跑一遍全量内容。
pub fn compute_hash(data: &[u8]) -> String {
    crate::clipboard_watcher::content::compute_hash(data)
}

/// 图片轮询之间的"还是上一张吗"指纹。挡掉的是一整次 PNG 编码。
pub fn rgba_fingerprint(width: usize, height: usize, bytes: &[u8]) -> u64 {
    crate::clipboard_watcher::content::rgba_fingerprint(width, height, bytes)
}

/// 敏感内容判定。命中前缀会提前返回，最坏情况是整段文本转小写后多次 contains。
pub fn is_sensitive_text(text: &str) -> bool {
    crate::clipboard_watcher::content::is_sensitive_text(text)
}

/// HTML 转可搜索纯文本，逐字符扫描。
pub fn strip_html_tags(html: &str) -> String {
    crate::clipboard_watcher::content::strip_html_tags(html)
}
