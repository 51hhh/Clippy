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
    Ok(())
}
