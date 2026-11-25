//! 程序化写入系统剪贴板的唯一三个出口。
//!
//! **写入成功后必须敲一下 `wake::nudge()`。** watcher 是 500 ms 轮询的，不敲的话
//! "刚复制的内容"最多要 500 ms 之后才进历史，而"截图复制完立刻去 Pin 剪贴板最新一条"
//! 这条路会因此 pin 到上一条。理由与替代方案见 `wake` 模块的模块注释。
//! 新增写入路径时照着补这一句，别在调用方那边补。

use super::wake;
use arboard::Clipboard;
use std::time::Duration;

/// 某些 WM/合成器首次写入时会资源忙，等待 30ms 后重试一次。
pub fn clipboard_set_text_with_retry(text: &str) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|error| error.to_string())?;
    if let Err(first_error) = clipboard.set_text(text) {
        log::warn!("clipboard set_text failed, retrying once: {first_error}");
        std::thread::sleep(Duration::from_millis(30));
        let mut retry = Clipboard::new().map_err(|error| error.to_string())?;
        retry.set_text(text).map_err(|error| error.to_string())?;
    }
    wake::nudge();
    Ok(())
}

pub fn clipboard_set_image_with_retry(img_data: arboard::ImageData) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|error| error.to_string())?;
    if let Err(first_error) = clipboard.set_image(img_data.clone()) {
        log::warn!("clipboard set_image failed, retrying once: {first_error}");
        std::thread::sleep(Duration::from_millis(30));
        let mut retry = Clipboard::new().map_err(|error| error.to_string())?;
        retry
            .set_image(img_data)
            .map_err(|error| error.to_string())?;
    }
    wake::nudge();
    Ok(())
}

pub fn clipboard_set_html_with_retry(html: &str, alt_text: Option<&str>) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|error| error.to_string())?;
    if let Err(first_error) = clipboard.set().html(html, alt_text) {
        log::warn!("clipboard set_html failed, retrying once: {first_error}");
        std::thread::sleep(Duration::from_millis(30));
        let mut retry = Clipboard::new().map_err(|error| error.to_string())?;
        retry
            .set()
            .html(html, alt_text)
            .map_err(|error| error.to_string())?;
    }
    wake::nudge();
    Ok(())
}
