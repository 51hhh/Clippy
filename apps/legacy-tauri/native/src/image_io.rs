use clippy_core::image_output::filename_candidates;
pub use clippy_core::image_output::{
    expand_user_path, system_screenshot_dir, unique_image_id, SaveTarget, DEFAULT_FILENAME_TEMPLATE,
};
use std::path::{Path, PathBuf};

pub fn png_to_clipboard_image(png: &[u8]) -> Result<arboard::ImageData<'static>, String> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|error| format!("PNG 解码失败: {error}"))?;
    // `into_rgba8` 而不是 `to_rgba8`：解出来本来就是 RGBA8 时（PNG 的常见情形）
    // 前者原地接管缓冲区，后者要再拷一份 16 MB。
    Ok(rgba_to_clipboard_image(image.into_rgba8()))
}

/// 已经解好的像素直接交给剪贴板，不再走一遍 PNG。
///
/// 给"这张图刚刚才解码过"的调用方用（截图提交要先校验再复制，见
/// `capture::CommitImage`）。`into_raw` 接管缓冲区，这里没有任何一次全图拷贝。
pub fn rgba_to_clipboard_image(rgba: image::RgbaImage) -> arboard::ImageData<'static> {
    let (width, height) = rgba.dimensions();
    arboard::ImageData {
        width: width as usize,
        height: height as usize,
        bytes: std::borrow::Cow::Owned(rgba.into_raw()),
    }
}

pub fn copy_png_to_clipboard(png: &[u8]) -> Result<(), String> {
    crate::clipboard_watcher::clipboard_set_image_with_retry(png_to_clipboard_image(png)?)
}

/// 缩到最长边不超过 `max_edge` 的 PNG，保持长宽比。本来就够小的原样返回。
///
/// 给列表行的缩略图用。行里那一格是 48 CSS px，而库里存的是原图（一张全屏截图就是
/// 2560×1600 / 几 MB）。为了画 48 px 把整张原图送进 webview 再解码，一次开面板十几个
/// 图片条目就是几十 MB IPC 加十几次全尺寸 PNG 解码，都落在 webview 那一个线程上。
///
/// `thumbnail` 而不是 `resize`：它是 image crate 里专为缩略图准备的快路径（先按整数倍
/// 采样再做一次三角过滤），质量对 48 px 完全够，速度比 Lanczos 高一个量级。
pub fn thumbnail_png(png: &[u8], max_edge: u32) -> Result<Vec<u8>, String> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|error| format!("PNG 解码失败: {error}"))?;
    if image.width() <= max_edge && image.height() <= max_edge {
        return Ok(png.to_vec());
    }
    let thumbnail = image.thumbnail(max_edge, max_edge).into_rgba8();
    let (width, height) = thumbnail.dimensions();
    crate::screenshot::encode_png(&thumbnail.into_raw(), width, height)
        .map_err(|error| error.to_string())
}

/// 按配置的目录与模板保存 PNG，返回实际写入的路径。
pub fn save_png(png: &[u8], prefix: &str, target: &SaveTarget) -> Result<PathBuf, String> {
    crate::screenshot::png_dimensions(png).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&target.directory)
        .map_err(|error| format!("创建截图目录失败: {error}"))?;
    let stem = target.filename_stem(prefix);
    write_new_png(&target.directory, &stem, png)
}

/// 逐个候选名用 `create_new` 试写：既不覆盖已有文件，也不会让同一毫秒内的
/// 两次保存选中同一个名字后互相覆盖。
fn write_new_png(directory: &Path, stem: &str, png: &[u8]) -> Result<PathBuf, String> {
    use std::io::Write;
    for candidate in filename_candidates(stem) {
        let path = directory.join(candidate);
        if path.exists() {
            continue;
        }
        // 先完整写入同目录临时文件并 flush，再用 hard_link 原子、且不覆盖地提交目标名。
        // 直接 create_new(target)+write 在磁盘满/进程中断时会留下看似成功的截断 PNG。
        let temp = directory.join(format!(".clippy-save-{}.tmp", unique_image_id()));
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("保存图片失败: {error}")),
        };
        let write_result = file.write_all(png).and_then(|()| file.sync_all());
        drop(file);
        if let Err(error) = write_result {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("保存图片失败: {error}"));
        }
        match std::fs::hard_link(&temp, &path) {
            Ok(()) => {
                let _ = std::fs::remove_file(&temp);
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = std::fs::remove_file(&temp);
                continue;
            }
            Err(error) => {
                let _ = std::fs::remove_file(&temp);
                return Err(format!("保存图片失败: {error}"));
            }
        }
    }
    Err("保存图片失败: 同名文件过多".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_names_get_a_suffix_instead_of_overwriting() {
        let candidates = filename_candidates("shot");
        assert_eq!(candidates[0], "shot.png");
        assert_eq!(candidates[1], "shot-2.png");
        assert_eq!(candidates.len(), 11);
        assert!(candidates.last().unwrap().starts_with("shot-"));
    }

    #[test]
    fn saving_twice_with_a_fixed_template_keeps_both_files() {
        let directory = tempfile::tempdir().expect("创建临时目录失败");
        let png = crate::screenshot::encode_png(&[0, 0, 0, 255], 1, 1).expect("编码 PNG 失败");
        let target = SaveTarget {
            directory: directory.path().to_path_buf(),
            template: "fixed".to_string(),
        };

        let first = save_png(&png, "clippy-screenshot", &target).expect("首次保存应当成功");
        let second = save_png(&png, "clippy-screenshot", &target).expect("重名保存应当成功");

        assert_eq!(first.file_name().unwrap(), "fixed.png");
        assert_eq!(second.file_name().unwrap(), "fixed-2.png");
        assert!(first.exists() && second.exists());
        assert!(
            std::fs::read_dir(directory.path())
                .unwrap()
                .all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")),
            "成功提交后不能遗留临时文件"
        );
    }

    #[test]
    fn thumbnails_shrink_the_long_edge_and_keep_the_aspect_ratio() {
        let wide = crate::screenshot::encode_png(&vec![9u8; 400 * 200 * 4], 400, 200)
            .expect("编码 PNG 失败");
        let thumbnail = thumbnail_png(&wide, 100).expect("缩略图失败");
        assert_eq!(
            crate::screenshot::png_dimensions(&thumbnail).expect("读尺寸失败"),
            (100, 50)
        );
        // 缩略图必须真的更小，否则这条路白花一次解码 + 编码。
        assert!(thumbnail.len() < wide.len());
    }

    #[test]
    fn images_already_small_enough_are_returned_untouched() {
        // 原样返回而不是重编码：小图重编码只会白花时间，还可能因为编码参数不同而变大。
        let small = crate::screenshot::encode_png(&[1, 2, 3, 255], 1, 1).expect("编码 PNG 失败");
        assert_eq!(thumbnail_png(&small, 128).expect("缩略图失败"), small);
    }

    #[test]
    fn thumbnailing_a_non_png_fails_instead_of_returning_garbage() {
        assert!(thumbnail_png(b"not a png", 128).is_err());
    }

    #[test]
    fn save_png_creates_a_missing_directory_and_rejects_non_png() {
        let directory = tempfile::tempdir().expect("创建临时目录失败");
        let target = SaveTarget {
            directory: directory.path().join("nested").join("shots"),
            template: "{prefix}".to_string(),
        };
        let png = crate::screenshot::encode_png(&[1, 2, 3, 255], 1, 1).expect("编码 PNG 失败");

        let path = save_png(&png, "clippy-pin", &target).expect("保存应当创建目录");
        assert_eq!(path.parent().unwrap(), target.directory);
        assert!(save_png(b"not a png", "clippy-pin", &target).is_err());
    }
}
