use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static IMAGE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub fn png_to_clipboard_image(png: &[u8]) -> Result<arboard::ImageData<'static>, String> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|error| format!("PNG 解码失败: {error}"))?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(arboard::ImageData {
        width: width as usize,
        height: height as usize,
        bytes: std::borrow::Cow::Owned(rgba.into_raw()),
    })
}

pub fn copy_png_to_clipboard(png: &[u8]) -> Result<(), String> {
    crate::clipboard_watcher::clipboard_set_image_with_retry(png_to_clipboard_image(png)?)
}

pub fn save_png(png: &[u8], prefix: &str) -> Result<PathBuf, String> {
    crate::screenshot::png_dimensions(png).map_err(|error| error.to_string())?;
    let directory = default_screenshot_dir();
    std::fs::create_dir_all(&directory).map_err(|error| format!("创建截图目录失败: {error}"))?;
    let path = directory.join(format!("{prefix}-{}.png", unique_image_id()));
    std::fs::write(&path, png).map_err(|error| format!("保存图片失败: {error}"))?;
    Ok(path)
}

pub fn default_screenshot_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Pictures")
        .join("Clippy")
}

pub fn unique_image_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let sequence = IMAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{millis}-{sequence}")
}
