//! 录屏结果库的私有持久首帧缩略图缓存。

use crate::private_files::restrict_directory;
#[cfg(feature = "recording-vp9-prototype")]
use crate::private_files::{replace_private_file, restrict_file, write_private};
use std::fs;
use std::io;
#[cfg(feature = "recording-vp9-prototype")]
use std::io::Cursor;
use std::path::Path;
#[cfg(any(feature = "recording-vp9-prototype", test))]
use std::path::PathBuf;
use std::sync::Mutex;

#[cfg(feature = "recording-vp9-prototype")]
use super::manifest::RecordingThumbnailSource;

const CACHE_DIRECTORY: &str = "recording-thumbnails";
#[cfg(feature = "recording-vp9-prototype")]
const MAX_THUMBNAIL_EDGE: u32 = 320;
#[cfg(feature = "recording-vp9-prototype")]
const MAX_THUMBNAIL_BYTES: u64 = 512 * 1024;
const MAX_CACHE_ENTRIES: usize = 64;
#[cfg(feature = "recording-vp9-prototype")]
const MAX_DECODE_PIXELS: u64 = 16_777_216;

/// 冷缓存只允许一个 VP9 解码任务，避免结果页快速滚动造成多份全尺寸 RGBA 峰值。
#[derive(Debug, Default)]
pub(crate) struct RecordingThumbnailManager {
    generation: Mutex<()>,
}

impl RecordingThumbnailManager {
    #[cfg(feature = "recording-vp9-prototype")]
    pub(in crate::recording) fn load_or_generate(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        source: &RecordingThumbnailSource,
    ) -> Result<Vec<u8>, String> {
        let _generation = self
            .generation
            .lock()
            .map_err(|error| format!("录屏缩略图生成状态损坏: {error}"))?;
        let session_directory = prepare_session_cache(app_data_dir, session_id)?;
        let cache_name = format!("{}.png", source.artifact.sha256);
        let cache_path = session_directory.join(&cache_name);
        if let Some(bytes) = read_valid_cache(&cache_path)? {
            remove_other_managed_files(&session_directory, &cache_name)?;
            return Ok(bytes);
        }

        let bytes = generate_thumbnail(source)?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_THUMBNAIL_BYTES {
            return Err("录屏缩略图 PNG 超过 512 KiB 上限".to_string());
        }
        let temporary = session_directory.join(format!(".{cache_name}.partial"));
        remove_managed_path(&temporary)?;
        let result = write_private(&temporary, &bytes)
            .and_then(|_| replace_private_file(&temporary, &cache_path))
            .and_then(|_| sync_directory(&session_directory));
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(|error| format!("写入录屏缩略图缓存失败: {error}"))?;
        remove_other_managed_files(&session_directory, &cache_name)?;
        Ok(bytes)
    }

    /// 缩略图是派生数据。清理失败只记录错误，调用方仍应继续删除真实录屏。
    pub(in crate::recording) fn clear_session(
        &self,
        app_data_dir: &Path,
        session_id: &str,
    ) -> Result<(), String> {
        let _generation = self
            .generation
            .lock()
            .map_err(|error| format!("录屏缩略图生成状态损坏: {error}"))?;
        if !valid_identifier(session_id) {
            return Err("录屏缩略图会话身份无效".to_string());
        }
        let root = app_data_dir.join(CACHE_DIRECTORY);
        let root_metadata = match fs::symlink_metadata(&root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("读取录屏缩略图目录失败: {error}")),
        };
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            return Err("录屏缩略图根路径不是普通目录".to_string());
        }
        restrict_directory(&root)
            .map_err(|error| format!("收紧录屏缩略图根目录权限失败: {error}"))?;
        let session_directory = root.join(session_id);
        let metadata = match fs::symlink_metadata(&session_directory) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("读取录屏缩略图会话目录失败: {error}")),
        };
        if metadata.file_type().is_symlink() {
            fs::remove_file(&session_directory)
                .map_err(|error| format!("删除录屏缩略图符号链接失败: {error}"))?;
            return Ok(());
        }
        if !metadata.is_dir() {
            return Err("录屏缩略图会话路径不是普通目录".to_string());
        }
        restrict_directory(&session_directory)
            .map_err(|error| format!("收紧录屏缩略图会话目录权限失败: {error}"))?;
        let mut entries = fs::read_dir(&session_directory)
            .map_err(|error| format!("读取录屏缩略图缓存失败: {error}"))?;
        for _ in 0..MAX_CACHE_ENTRIES {
            let Some(entry) = entries
                .next()
                .transpose()
                .map_err(|error| format!("读取录屏缩略图缓存项失败: {error}"))?
            else {
                break;
            };
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if is_managed_cache_name(&name) {
                remove_managed_path(&entry.path())?;
            }
        }
        match fs::remove_dir(&session_directory) {
            Ok(()) => {
                sync_directory(&root).map_err(|error| format!("同步录屏缩略图目录失败: {error}"))
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
                ) =>
            {
                Ok(())
            }
            Err(error) => Err(format!("删除录屏缩略图会话目录失败: {error}")),
        }
    }
}

#[cfg(any(feature = "recording-vp9-prototype", test))]
fn prepare_session_cache(app_data_dir: &Path, session_id: &str) -> Result<PathBuf, String> {
    if !valid_identifier(session_id) {
        return Err("录屏缩略图会话身份无效".to_string());
    }
    let root = app_data_dir.join(CACHE_DIRECTORY);
    ensure_private_directory(&root, "录屏缩略图根目录")?;
    let session_directory = root.join(session_id);
    ensure_private_directory(&session_directory, "录屏缩略图会话目录")?;
    Ok(session_directory)
}

#[cfg(any(feature = "recording-vp9-prototype", test))]
fn ensure_private_directory(path: &Path, label: &str) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(format!("{label}不是普通目录"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| format!("创建{label}失败: {error}"))?;
        }
        Err(error) => return Err(format!("读取{label}失败: {error}")),
    }
    restrict_directory(path).map_err(|error| format!("收紧{label}权限失败: {error}"))
}

#[cfg(feature = "recording-vp9-prototype")]
fn read_valid_cache(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("读取录屏缩略图缓存失败: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        remove_managed_path(path)?;
        return Ok(None);
    }
    if metadata.len() == 0 || metadata.len() > MAX_THUMBNAIL_BYTES {
        fs::remove_file(path).map_err(|error| format!("删除无效录屏缩略图失败: {error}"))?;
        return Ok(None);
    }
    restrict_file(path).map_err(|error| format!("收紧录屏缩略图权限失败: {error}"))?;
    let bytes = fs::read(path).map_err(|error| format!("读取录屏缩略图失败: {error}"))?;
    let dimensions =
        image::ImageReader::with_format(Cursor::new(bytes.as_slice()), image::ImageFormat::Png)
            .into_dimensions();
    let valid = dimensions
        .map(|(width, height)| {
            width > 0 && height > 0 && width <= MAX_THUMBNAIL_EDGE && height <= MAX_THUMBNAIL_EDGE
        })
        .unwrap_or(false)
        && image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).is_ok();
    if valid {
        Ok(Some(bytes))
    } else {
        fs::remove_file(path).map_err(|error| format!("删除损坏录屏缩略图失败: {error}"))?;
        Ok(None)
    }
}

#[cfg(feature = "recording-vp9-prototype")]
fn remove_other_managed_files(directory: &Path, keep: &str) -> Result<(), String> {
    let mut entries =
        fs::read_dir(directory).map_err(|error| format!("读取录屏缩略图缓存失败: {error}"))?;
    for _ in 0..MAX_CACHE_ENTRIES {
        let Some(entry) = entries
            .next()
            .transpose()
            .map_err(|error| format!("读取录屏缩略图缓存项失败: {error}"))?
        else {
            break;
        };
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name != keep && is_managed_cache_name(&name) {
            remove_managed_path(&entry.path())?;
        }
    }
    Ok(())
}

fn remove_managed_path(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("读取录屏缩略图缓存项失败: {error}")),
    };
    if metadata.is_file() || metadata.file_type().is_symlink() {
        fs::remove_file(path).map_err(|error| format!("删除录屏缩略图缓存项失败: {error}"))
    } else {
        Err("录屏缩略图缓存项不是普通文件".to_string())
    }
}

fn is_managed_cache_name(name: &str) -> bool {
    let sha = name.strip_suffix(".png").or_else(|| {
        name.strip_prefix('.')
            .and_then(|name| name.strip_suffix(".png.partial"))
    });
    sha.is_some_and(valid_sha256)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && value != "."
        && value != ".."
}

#[cfg(feature = "recording-vp9-prototype")]
fn generate_thumbnail(source: &RecordingThumbnailSource) -> Result<Vec<u8>, String> {
    use super::mux::webm_remux::{first_vp9_keyframe, WebmRemuxSource, WebmRemuxSpec};
    use image::ImageEncoder;
    use shiguredo_libvpx::{Decoder, DecoderCodec, DecoderConfig};

    let pixels = u64::from(source.width)
        .checked_mul(u64::from(source.height))
        .filter(|pixels| *pixels > 0 && *pixels <= MAX_DECODE_PIXELS)
        .ok_or_else(|| "录屏缩略图源像素超过 16777216 上限".to_string())?;
    let packet = first_vp9_keyframe(
        &WebmRemuxSource {
            path: source.artifact.path.clone(),
            byte_length: source.artifact.byte_length,
            sha256: source.artifact.sha256.clone(),
            started_at_ns: 0,
            duration_ns: source.duration_ns,
            frame_count: source.frame_count,
        },
        WebmRemuxSpec {
            width: source.width,
            height: source.height,
            fps_numerator: source.target_fps_numerator,
            fps_denominator: source.target_fps_denominator,
        },
    )?;
    let mut decoder = Decoder::new(DecoderConfig::new(DecoderCodec::Vp9))
        .map_err(|error| format!("创建 VP9 缩略图解码器失败: {error}"))?;
    decoder
        .decode(&packet)
        .map_err(|error| format!("解码 VP9 缩略图失败: {error}"))?;
    let rgba = {
        let frame = require_first_frame(
            decoder
                .next_frame()
                .map_err(|error| format!("读取 VP9 缩略图帧失败: {error}"))?,
        )?;
        let y_stride = frame.y_stride();
        let u_stride = frame.u_stride();
        let v_stride = frame.v_stride();
        validate_decoded_layout(
            frame.is_high_depth(),
            frame.width(),
            frame.height(),
            y_stride,
            u_stride,
            v_stride,
            source.width,
            source.height,
        )?;
        let rgba_bytes = pixels
            .checked_mul(4)
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or_else(|| "VP9 缩略图 RGBA 大小溢出".to_string())?;
        let mut rgba = vec![0_u8; rgba_bytes];
        let planar = yuv::YuvPlanarImage {
            y_plane: frame.y_plane(),
            y_stride: u32::try_from(y_stride)
                .map_err(|_| "VP9 缩略图 Y stride 溢出".to_string())?,
            u_plane: frame.u_plane(),
            u_stride: u32::try_from(u_stride)
                .map_err(|_| "VP9 缩略图 U stride 溢出".to_string())?,
            v_plane: frame.v_plane(),
            v_stride: u32::try_from(v_stride)
                .map_err(|_| "VP9 缩略图 V stride 溢出".to_string())?,
            width: source.width,
            height: source.height,
        };
        yuv::yuv420_to_rgba(
            &planar,
            &mut rgba,
            source
                .width
                .checked_mul(4)
                .ok_or_else(|| "VP9 缩略图 RGBA stride 溢出".to_string())?,
            yuv::YuvRange::Limited,
            yuv::YuvStandardMatrix::Bt709,
        )
        .map_err(|error| format!("转换 VP9 缩略图颜色失败: {error}"))?;
        rgba
    };
    reject_additional_frame(
        decoder
            .next_frame()
            .map_err(|error| format!("读取额外 VP9 缩略图帧失败: {error}"))?,
        "单个 VP9 缩略图 packet 输出了多帧",
    )?;
    decoder
        .finish()
        .map_err(|error| format!("结束 VP9 缩略图解码失败: {error}"))?;
    reject_additional_frame(
        decoder
            .next_frame()
            .map_err(|error| format!("刷新 VP9 缩略图帧失败: {error}"))?,
        "VP9 缩略图解码结束后输出了额外画面",
    )?;

    let image = image::RgbaImage::from_raw(source.width, source.height, rgba)
        .ok_or_else(|| "构造 VP9 缩略图像素失败".to_string())?;
    let thumbnail = image::DynamicImage::ImageRgba8(image)
        .thumbnail(MAX_THUMBNAIL_EDGE, MAX_THUMBNAIL_EDGE)
        .to_rgba8();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            thumbnail.as_raw(),
            thumbnail.width(),
            thumbnail.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| format!("编码录屏缩略图 PNG 失败: {error}"))?;
    Ok(png)
}

#[cfg(feature = "recording-vp9-prototype")]
fn require_first_frame<T>(frame: Option<T>) -> Result<T, String> {
    frame.ok_or_else(|| "VP9 缩略图 packet 没有输出画面".to_string())
}

#[cfg(feature = "recording-vp9-prototype")]
fn reject_additional_frame<T>(frame: Option<T>, message: &'static str) -> Result<(), String> {
    if frame.is_some() {
        Err(message.to_string())
    } else {
        Ok(())
    }
}

#[cfg(feature = "recording-vp9-prototype")]
#[allow(clippy::too_many_arguments)]
fn validate_decoded_layout(
    high_depth: bool,
    actual_width: usize,
    actual_height: usize,
    y_stride: usize,
    u_stride: usize,
    v_stride: usize,
    expected_width: u32,
    expected_height: u32,
) -> Result<(), String> {
    if high_depth
        || actual_width != expected_width as usize
        || actual_height != expected_height as usize
    {
        return Err("VP9 缩略图帧格式或尺寸与清单不一致".to_string());
    }
    let width = expected_width as usize;
    let chroma_width = width.div_ceil(2);
    if !(width..=width.saturating_mul(4)).contains(&y_stride)
        || !(chroma_width..=width.saturating_mul(2)).contains(&u_stride)
        || !(chroma_width..=width.saturating_mul(2)).contains(&v_stride)
    {
        return Err("VP9 缩略图 plane stride 越界".to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "recording-vp9-prototype")]
    use crate::recording::manifest::{RecordingThumbnailSource, ResolvedRecordingArtifact};
    #[cfg(feature = "recording-vp9-prototype")]
    use crate::recording::mux::vp9_webm::Vp9WebmWriter;
    #[cfg(feature = "recording-vp9-prototype")]
    use sha2::{Digest, Sha256};
    #[cfg(feature = "recording-vp9-prototype")]
    use std::sync::Arc;

    #[cfg(feature = "recording-vp9-prototype")]
    fn png_fixture(width: u32, height: u32) -> Vec<u8> {
        use image::ImageEncoder;

        let rgba = vec![0_u8; (u64::from(width) * u64::from(height) * 4) as usize];
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)
            .unwrap();
        png
    }

    #[cfg(feature = "recording-vp9-prototype")]
    fn fixture_source(directory: &Path, color: [u8; 3]) -> RecordingThumbnailSource {
        let width = 640;
        let height = 360;
        let rgba = (0..width * height)
            .flat_map(|_| [color[0], color[1], color[2], 255])
            .collect::<Vec<_>>();
        let mut writer = Vp9WebmWriter::new(Cursor::new(Vec::new()), width, height, 10, 1).unwrap();
        writer.push_rgba(&rgba, 0).unwrap();
        let output = writer.finish_with_stats(100_000_000).unwrap();
        let bytes = output.writer.into_inner();
        let path = directory.join("recording.webm");
        fs::write(&path, &bytes).unwrap();
        RecordingThumbnailSource {
            artifact: ResolvedRecordingArtifact {
                path,
                suggested_file_name: "recording.webm".to_string(),
                byte_length: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&bytes)),
            },
            width,
            height,
            target_fps_numerator: 10,
            target_fps_denominator: 1,
            duration_ns: 100_000_000,
            frame_count: 1,
        }
    }

    #[test]
    fn cache_names_are_exact_and_session_ids_cannot_escape() {
        let sha = "a".repeat(64);
        assert!(is_managed_cache_name(&format!("{sha}.png")));
        assert!(is_managed_cache_name(&format!(".{sha}.png.partial")));
        assert!(!is_managed_cache_name("thumbnail.png"));
        assert!(valid_identifier("recording-01_a.b"));
        assert!(!valid_identifier("../recording"));
        assert!(!valid_identifier("recording/01"));
    }

    #[test]
    fn clear_removes_only_managed_files_and_never_blocks_on_unknown_entries() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = prepare_session_cache(temporary.path(), "session-1").unwrap();
        let sha = "b".repeat(64);
        fs::write(directory.join(format!("{sha}.png")), b"managed").unwrap();
        fs::write(directory.join("unknown.txt"), b"keep").unwrap();
        RecordingThumbnailManager::default()
            .clear_session(temporary.path(), "session-1")
            .unwrap();
        assert!(!directory.join(format!("{sha}.png")).exists());
        assert_eq!(fs::read(directory.join("unknown.txt")).unwrap(), b"keep");
    }

    #[cfg(feature = "recording-vp9-prototype")]
    #[test]
    fn validates_decoder_frame_count_depth_geometry_and_strides() {
        assert_eq!(require_first_frame(Some(7_u8)).unwrap(), 7);
        assert!(require_first_frame::<u8>(None).is_err());
        assert!(reject_additional_frame::<u8>(None, "extra").is_ok());
        assert_eq!(
            reject_additional_frame(Some(7_u8), "extra").unwrap_err(),
            "extra"
        );

        assert!(validate_decoded_layout(false, 640, 360, 640, 320, 320, 640, 360).is_ok());
        assert!(validate_decoded_layout(true, 640, 360, 640, 320, 320, 640, 360).is_err());
        assert!(validate_decoded_layout(false, 639, 360, 640, 320, 320, 640, 360).is_err());
        assert!(validate_decoded_layout(false, 640, 359, 640, 320, 320, 640, 360).is_err());
        assert!(validate_decoded_layout(false, 640, 360, 639, 320, 320, 640, 360).is_err());
        assert!(validate_decoded_layout(false, 640, 360, 2_561, 320, 320, 640, 360).is_err());
        assert!(validate_decoded_layout(false, 640, 360, 640, 319, 320, 640, 360).is_err());
        assert!(validate_decoded_layout(false, 640, 360, 640, 320, 1_281, 640, 360).is_err());
    }

    #[cfg(feature = "recording-vp9-prototype")]
    #[test]
    fn rejects_oversized_and_dimension_forged_cache_files() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = prepare_session_cache(temporary.path(), "invalid-cache").unwrap();

        let oversized = directory.join(format!("{}.png", "c".repeat(64)));
        fs::write(&oversized, vec![0_u8; MAX_THUMBNAIL_BYTES as usize + 1]).unwrap();
        assert!(read_valid_cache(&oversized).unwrap().is_none());
        assert!(!oversized.exists());

        let forged_dimensions = directory.join(format!("{}.png", "d".repeat(64)));
        fs::write(&forged_dimensions, png_fixture(321, 1)).unwrap();
        assert!(read_valid_cache(&forged_dimensions).unwrap().is_none());
        assert!(!forged_dimensions.exists());
    }

    #[cfg(feature = "recording-vp9-prototype")]
    #[test]
    fn decodes_one_vp9_keyframe_persists_png_and_hits_cache_without_source() {
        let temporary = tempfile::tempdir().unwrap();
        let source = fixture_source(temporary.path(), [32, 96, 180]);
        let manager = RecordingThumbnailManager::default();
        let first = manager
            .load_or_generate(temporary.path(), "session-vp9", &source)
            .unwrap();
        let image = image::load_from_memory_with_format(&first, image::ImageFormat::Png)
            .unwrap()
            .to_rgba8();
        assert_eq!(image.dimensions(), (320, 180));
        let pixel = image.get_pixel(160, 90).0;
        assert!(pixel[0].abs_diff(32) <= 8, "red={}", pixel[0]);
        assert!(pixel[1].abs_diff(96) <= 8, "green={}", pixel[1]);
        assert!(pixel[2].abs_diff(180) <= 8, "blue={}", pixel[2]);

        fs::remove_file(&source.artifact.path).unwrap();
        let cached = manager
            .load_or_generate(temporary.path(), "session-vp9", &source)
            .unwrap();
        assert_eq!(cached, first);
    }

    #[cfg(feature = "recording-vp9-prototype")]
    #[test]
    fn rejects_hash_and_geometry_mismatches_and_regenerates_corrupt_cache() {
        let temporary = tempfile::tempdir().unwrap();
        let source = fixture_source(temporary.path(), [180, 64, 24]);
        let manager = RecordingThumbnailManager::default();

        let mut bad_hash = source.clone();
        bad_hash.artifact.sha256 = "0".repeat(64);
        assert!(manager
            .load_or_generate(temporary.path(), "bad-hash", &bad_hash)
            .is_err());

        let mut bad_geometry = source.clone();
        bad_geometry.width = 638;
        assert!(manager
            .load_or_generate(temporary.path(), "bad-geometry", &bad_geometry)
            .is_err());

        let mut too_many_pixels = source.clone();
        too_many_pixels.width = 4_097;
        too_many_pixels.height = 4_097;
        assert!(manager
            .load_or_generate(temporary.path(), "too-many-pixels", &too_many_pixels)
            .unwrap_err()
            .contains("16777216"));

        let first = manager
            .load_or_generate(temporary.path(), "regenerate", &source)
            .unwrap();
        let cache = temporary
            .path()
            .join(CACHE_DIRECTORY)
            .join("regenerate")
            .join(format!("{}.png", source.artifact.sha256));
        fs::write(&cache, b"not png").unwrap();
        let regenerated = manager
            .load_or_generate(temporary.path(), "regenerate", &source)
            .unwrap();
        assert_eq!(regenerated, first);
        assert_ne!(fs::read(cache).unwrap(), b"not png");

        let changed = fixture_source(temporary.path(), [20, 180, 220]);
        let changed_png = manager
            .load_or_generate(temporary.path(), "regenerate", &changed)
            .unwrap();
        assert_ne!(changed_png, first);
        let entries = fs::read_dir(temporary.path().join(CACHE_DIRECTORY).join("regenerate"))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].file_name().to_str(),
            Some(format!("{}.png", changed.artifact.sha256).as_str())
        );
    }

    #[cfg(all(feature = "recording-vp9-prototype", unix))]
    #[test]
    fn replaces_symlinked_cache_without_touching_its_target() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().unwrap();
        let source = fixture_source(temporary.path(), [90, 40, 150]);
        let directory = prepare_session_cache(temporary.path(), "symlink-cache").unwrap();
        let outside = temporary.path().join("outside.png");
        fs::write(&outside, b"outside").unwrap();
        let cache = directory.join(format!("{}.png", source.artifact.sha256));
        symlink(&outside, &cache).unwrap();

        RecordingThumbnailManager::default()
            .load_or_generate(temporary.path(), "symlink-cache", &source)
            .unwrap();
        assert_eq!(fs::read(outside).unwrap(), b"outside");
        assert!(fs::symlink_metadata(cache).unwrap().is_file());
    }

    #[cfg(feature = "recording-vp9-prototype")]
    #[test]
    fn serializes_duplicate_cold_requests_into_one_cache_entry() {
        let temporary = tempfile::tempdir().unwrap();
        let source = Arc::new(fixture_source(temporary.path(), [40, 160, 80]));
        let manager = Arc::new(RecordingThumbnailManager::default());
        let root = temporary.path().to_path_buf();
        let workers = (0..2)
            .map(|_| {
                let source = Arc::clone(&source);
                let manager = Arc::clone(&manager);
                let root = root.clone();
                std::thread::spawn(move || manager.load_or_generate(&root, "parallel", &source))
            })
            .collect::<Vec<_>>();
        let first = workers
            .into_iter()
            .map(|worker| worker.join().unwrap().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(first[0], first[1]);
        let entries = fs::read_dir(root.join(CACHE_DIRECTORY).join("parallel"))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(entries.len(), 1);
    }
}
