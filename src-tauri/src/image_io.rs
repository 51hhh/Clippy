use crate::models::AppConfig;
use chrono::{DateTime, Local};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static IMAGE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// 内置文件名模板。扩展名固定为 `.png`，模板只描述主干。
pub const DEFAULT_FILENAME_TEMPLATE: &str = "{prefix}-{date}_{time}";

/// 同名时最多追加多少个序号，用完再退回时间戳序列号，避免无限试探。
const MAX_NAME_ATTEMPTS: u32 = 9;

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

/// 一次保存的落盘位置。配置里的空值在这里就解析成内置默认，
/// 调用方拿到的目录与模板一定可以直接用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveTarget {
    pub directory: PathBuf,
    pub template: String,
}

impl SaveTarget {
    pub fn from_config(config: &AppConfig, default_directory: &Path) -> Self {
        Self {
            directory: resolve_save_dir(&config.screenshot_save_dir, default_directory),
            template: resolve_template(&config.screenshot_filename_template),
        }
    }
}

/// 按配置的目录与模板保存 PNG，返回实际写入的路径。
pub fn save_png(png: &[u8], prefix: &str, target: &SaveTarget) -> Result<PathBuf, String> {
    crate::screenshot::png_dimensions(png).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&target.directory)
        .map_err(|error| format!("创建截图目录失败: {error}"))?;
    let stem = render_filename(&target.template, prefix, Local::now());
    write_new_png(&target.directory, &stem, png)
}

/// 系统默认保存目录。优先使用 Tauri 解析出的 Pictures；不可用时落到同样由 Tauri
/// 解析出的应用数据目录，避免猜测各平台的用户目录名。
pub fn system_screenshot_dir(picture_dir: Option<&Path>, app_data_dir: &Path) -> PathBuf {
    picture_dir
        .map(|directory| directory.join("Clippy"))
        .unwrap_or_else(|| app_data_dir.join("Screenshots"))
}

pub fn unique_image_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("{millis}-{}", next_sequence())
}

fn next_sequence() -> u64 {
    IMAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

fn home_dir() -> Option<PathBuf> {
    environment_path("HOME")
        .or_else(|| environment_path("USERPROFILE"))
        .or_else(|| {
            let drive = std::env::var_os("HOMEDRIVE").filter(|value| !value.is_empty())?;
            let path = std::env::var_os("HOMEPATH").filter(|value| !value.is_empty())?;
            let mut combined = PathBuf::from(drive);
            combined.push(path);
            Some(combined)
        })
}

fn environment_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// 配置留空表示跟随内置默认目录，不把当前默认路径固化进用户配置。
fn resolve_save_dir(configured: &str, default_directory: &Path) -> PathBuf {
    let configured = configured.trim();
    if configured.is_empty() {
        return default_directory.to_path_buf();
    }
    expand_user_path(configured)
}

pub fn expand_user_path(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = home_dir() {
            return home;
        }
    }
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// 模板只用来生成文件名，因此这里去掉多余的 `.png` 后缀并清掉路径分隔符：
/// 用户写错模板不应该把文件写到目录之外。
fn resolve_template(configured: &str) -> String {
    let configured = configured.trim();
    let without_extension = configured
        .strip_suffix(".png")
        .or_else(|| configured.strip_suffix(".PNG"))
        .unwrap_or(configured);
    let sanitized = sanitize_filename(without_extension);
    if sanitized.is_empty() {
        return DEFAULT_FILENAME_TEMPLATE.to_string();
    }
    sanitized
}

/// 展开模板占位符。渲染结果再清洗一次，因为占位符的值也可能来自配置。
fn render_filename(template: &str, prefix: &str, now: DateTime<Local>) -> String {
    let expanded = template
        .replace("{prefix}", prefix)
        .replace("{date}", &now.format("%Y-%m-%d").to_string())
        .replace("{time}", &now.format("%H-%M-%S").to_string())
        .replace("{unix}", &now.timestamp_millis().to_string())
        .replace("{seq}", &next_sequence().to_string());
    let sanitized = sanitize_filename(&expanded);
    if sanitized.is_empty() {
        // 模板把整个文件名清空时退回前缀，保证仍有可读的文件名。
        return sanitize_filename(prefix);
    }
    sanitized
}

/// 只保留能安全出现在三平台文件名里的字符。Windows 禁止的标点、路径分隔符与控制字符
/// 统一换成 `-`；前导点与尾部点/空格也去掉，避免隐藏文件和 Windows 路径错误。
fn sanitize_filename(value: &str) -> String {
    let replaced: String = value
        .chars()
        .map(|character| {
            if matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            ) || character.is_control()
            {
                '-'
            } else {
                character
            }
        })
        .collect();
    let sanitized = replaced
        .trim()
        .trim_start_matches('.')
        .trim_end_matches([' ', '.'])
        .to_string();
    if is_windows_reserved_name(&sanitized) {
        format!("_{sanitized}")
    } else {
        sanitized
    }
}

fn is_windows_reserved_name(value: &str) -> bool {
    let basename = value.split('.').next().unwrap_or_default();
    let uppercase = basename.to_ascii_uppercase();
    matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || uppercase
            .strip_prefix("COM")
            .or_else(|| uppercase.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
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

fn filename_candidates(stem: &str) -> Vec<String> {
    let mut candidates = Vec::with_capacity(MAX_NAME_ATTEMPTS as usize + 2);
    candidates.push(format!("{stem}.png"));
    for index in 2..=MAX_NAME_ATTEMPTS + 1 {
        candidates.push(format!("{stem}-{index}.png"));
    }
    // 模板不含时间占位符时前面的候选名可能都被占用，最后用一定唯一的序列号兜底。
    candidates.push(format!("{stem}-{}.png", unique_image_id()));
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_time() -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 8, 29, 21, 5, 7)
            .single()
            .expect("固定时间应当有唯一本地时刻")
    }

    #[test]
    fn default_template_uses_prefix_date_and_time() {
        assert_eq!(
            render_filename(DEFAULT_FILENAME_TEMPLATE, "clippy-screenshot", fixed_time()),
            "clippy-screenshot-2026-08-29_21-05-07"
        );
    }

    #[test]
    fn template_placeholders_expand_independently() {
        let now = fixed_time();
        assert_eq!(render_filename("shot-{date}", "p", now), "shot-2026-08-29");
        assert_eq!(render_filename("{time} {prefix}", "p", now), "21-05-07 p");
        assert_eq!(
            render_filename("{unix}", "p", now),
            now.timestamp_millis().to_string()
        );
        // {seq} 每次渲染都不同，只要求它是数字且会推进。
        let first = render_filename("{seq}", "p", now);
        let second = render_filename("{seq}", "p", now);
        assert!(first.parse::<u64>().unwrap() < second.parse::<u64>().unwrap());
    }

    #[test]
    fn template_cannot_escape_the_save_directory() {
        // 分隔符与前导点被清掉，模板写错也只会生成目录内的普通文件。
        assert_eq!(resolve_template("../../etc/passwd"), "-..-etc-passwd");
        assert_eq!(
            render_filename("{prefix}", "../evil", fixed_time()),
            "-evil"
        );
        assert_eq!(resolve_template(".hidden"), "hidden");
    }

    #[test]
    fn template_is_portable_to_windows() {
        assert_eq!(resolve_template("capture:21*05?07"), "capture-21-05-07");
        assert_eq!(resolve_template("shot. "), "shot");
        assert_eq!(resolve_template("CON"), "_CON");
        assert_eq!(resolve_template("lpt9.notes"), "_lpt9.notes");
        assert_eq!(resolve_template("COM10"), "COM10");
    }

    #[test]
    fn empty_or_extension_only_templates_fall_back_to_the_default() {
        assert_eq!(resolve_template(""), DEFAULT_FILENAME_TEMPLATE);
        assert_eq!(resolve_template("   "), DEFAULT_FILENAME_TEMPLATE);
        assert_eq!(resolve_template(".png"), DEFAULT_FILENAME_TEMPLATE);
        // 模板固定写 .png 时只去掉一次后缀，主干仍然保留。
        assert_eq!(resolve_template("shot.png"), "shot");
        // 渲染后为空时退回前缀，而不是写出一个只有扩展名的文件。
        assert_eq!(render_filename("{prefix}", "  ", fixed_time()), "");
        assert_eq!(
            render_filename(".", "clippy-pin", fixed_time()),
            "clippy-pin"
        );
    }

    #[test]
    fn save_dir_expands_tilde_and_falls_back_to_the_default() {
        let home = home_dir().expect("测试环境应当有 HOME");
        let default = PathBuf::from("/system/Pictures/Clippy");
        assert_eq!(resolve_save_dir("", &default), default);
        assert_eq!(resolve_save_dir("  ", &default), default);
        assert_eq!(resolve_save_dir("~", &default), home);
        assert_eq!(resolve_save_dir("~/Shots", &default), home.join("Shots"));
        assert_eq!(
            resolve_save_dir("/tmp/shots", &default),
            PathBuf::from("/tmp/shots")
        );
        // `~` 只在开头且紧跟分隔符时展开，普通相对路径原样保留。
        assert_eq!(resolve_save_dir("a~b", &default), PathBuf::from("a~b"));
    }

    #[test]
    fn save_target_from_config_resolves_empty_values() {
        let mut config = AppConfig::default();
        let system_default = PathBuf::from("/system/Pictures/Clippy");
        let target = SaveTarget::from_config(&config, &system_default);
        assert_eq!(target.directory, system_default);
        assert_eq!(target.template, DEFAULT_FILENAME_TEMPLATE);

        config.screenshot_save_dir = "/tmp/clippy-shots".to_string();
        config.screenshot_filename_template = "cap-{date}.png".to_string();
        let target = SaveTarget::from_config(&config, Path::new("/unused"));
        assert_eq!(target.directory, PathBuf::from("/tmp/clippy-shots"));
        assert_eq!(target.template, "cap-{date}");
    }

    #[test]
    fn system_directory_uses_tauri_paths_without_guessing_platform_folders() {
        let pictures = PathBuf::from("/system/Pictures");
        let app_data = PathBuf::from("/system/AppData/Clippy");
        assert_eq!(
            system_screenshot_dir(Some(&pictures), &app_data),
            pictures.join("Clippy")
        );
        assert_eq!(
            system_screenshot_dir(None, &app_data),
            app_data.join("Screenshots")
        );
    }

    #[test]
    fn windows_tilde_separator_expands_like_the_unix_separator() {
        let home = home_dir().expect("测试环境应当有 HOME");
        assert_eq!(expand_user_path("~\\Shots"), home.join("Shots"));
    }

    #[test]
    fn duplicate_names_get_a_suffix_instead_of_overwriting() {
        let candidates = filename_candidates("shot");
        assert_eq!(candidates[0], "shot.png");
        assert_eq!(candidates[1], "shot-2.png");
        assert_eq!(candidates.len(), MAX_NAME_ATTEMPTS as usize + 2);
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
