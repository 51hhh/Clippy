//! 受限 PNG 的本地二维码与条码识别。
//!
//! 本模块不读取文件、不访问网络，也不持久化识别结果。命令层只负责取出图片字节；其余
//! 耗时工作均在 blocking worker 中调用这里的纯函数。

use rxing::{BarcodeFormat, DecodeHints, Exceptions, Point};
use serde::Serialize;
use std::collections::HashSet;
use std::io::Cursor;

pub const MAX_PNG_BYTES: usize = 64 * 1024 * 1024;
const MAX_IMAGE_EDGE: u32 = 16_384;
const MAX_IMAGE_PIXELS: u64 = 40_000_000;
// 解码器可能先输出 RGBA；这里是唯一允许的原始像素工作区上限。
const MAX_DECODER_WORK_BYTES: usize = 160 * 1024 * 1024;
const MAX_SCAN_EDGE: u32 = 5_120;
const MAX_RESULTS: usize = 32;
const MAX_TEXT_BYTES_PER_RESULT: usize = 16 * 1024;
const MAX_TOTAL_TEXT_BYTES: usize = 64 * 1024;

/// Tauri IPC 的稳定错误对象；不携带数据库、PNG 或解码器的内部文本。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum CodeScanError {
    #[error("clip_not_found")]
    ClipNotFound,
    #[error("not_image")]
    NotImage,
    #[error("image_missing")]
    ImageMissing,
    #[error("image_too_large")]
    ImageTooLarge,
    #[error("image_invalid")]
    ImageInvalid,
    #[error("busy")]
    Busy,
    #[error("worker_failed")]
    WorkerFailed,
    #[error("decode_failed")]
    DecodeFailed,
    #[error("storage_failed")]
    StorageFailed,
    #[error("capture_unavailable")]
    CaptureUnavailable,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CodePoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CodeScanResult {
    pub format: String,
    pub text: String,
    pub points: Vec<CodePoint>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CodeScanResponse {
    pub results: Vec<CodeScanResult>,
    pub limited: bool,
}

struct PreparedLuma {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    original_width: u32,
    original_height: u32,
}

#[derive(Clone)]
struct Candidate {
    format: &'static str,
    text: String,
    points: Vec<CodePoint>,
}

/// 将 PNG 安全地解码成 8-bit 灰度。透明像素合成到白底，避免透明黑色误导扫描器。
fn decode_png_to_luma(png_bytes: &[u8]) -> Result<PreparedLuma, CodeScanError> {
    if png_bytes.len() > MAX_PNG_BYTES {
        return Err(CodeScanError::ImageTooLarge);
    }
    validate_png_container(png_bytes)?;

    let mut options = png::DecodeOptions::default();
    options.set_ignore_adler32(false);
    options.set_ignore_crc(false);
    options.set_skip_ancillary_crc_failures(false);
    let mut decoder = png::Decoder::new_with_options(Cursor::new(png_bytes), options);
    decoder.set_limits(png::Limits {
        bytes: MAX_DECODER_WORK_BYTES,
    });
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|_| CodeScanError::ImageInvalid)?;
    let (width, height) = reader.info().size();
    let pixel_count = validate_dimensions(width, height)?;
    let max_rgba_len = pixel_count
        .checked_mul(4)
        .ok_or(CodeScanError::ImageTooLarge)?;
    let output_len = reader.output_buffer_size();
    if output_len > max_rgba_len || output_len > MAX_DECODER_WORK_BYTES {
        return Err(CodeScanError::ImageTooLarge);
    }

    let mut decoded = reserve_zeroed(output_len)?;
    let info = reader
        .next_frame(&mut decoded)
        .map_err(|_| CodeScanError::ImageInvalid)?;
    reader.finish().map_err(|_| CodeScanError::ImageInvalid)?;
    if (info.width, info.height) != (width, height) || info.bit_depth != png::BitDepth::Eight {
        return Err(CodeScanError::ImageInvalid);
    }
    convert_to_luma_in_place(&mut decoded, info.color_type, pixel_count)?;

    let mut prepared = PreparedLuma {
        pixels: decoded,
        width,
        height,
        original_width: width,
        original_height: height,
    };
    downscale_if_needed(&mut prepared)?;
    Ok(prepared)
}

/// 将可信截图帧的 RGBA 选区直接转成灰度，避免为一次本地扫码先编码 PNG、再立即解码。
fn rgba_to_luma(rgba: Vec<u8>, width: u32, height: u32) -> Result<PreparedLuma, CodeScanError> {
    let pixel_count = validate_dimensions(width, height)?;
    let expected = pixel_count
        .checked_mul(4)
        .ok_or(CodeScanError::ImageTooLarge)?;
    if rgba.len() != expected {
        return Err(CodeScanError::ImageInvalid);
    }
    let mut pixels = reserve_zeroed(pixel_count)?;
    for (index, output) in pixels.iter_mut().enumerate() {
        let input = index * 4;
        let value = luminance(rgba[input], rgba[input + 1], rgba[input + 2]);
        *output = alpha_over_white(value, rgba[input + 3]);
    }
    let mut prepared = PreparedLuma {
        pixels,
        width,
        height,
        original_width: width,
        original_height: height,
    };
    downscale_if_needed(&mut prepared)?;
    Ok(prepared)
}

fn validate_png_container(png_bytes: &[u8]) -> Result<(), CodeScanError> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if !png_bytes.starts_with(SIGNATURE) {
        return Err(CodeScanError::ImageInvalid);
    }
    let mut cursor = SIGNATURE.len();
    loop {
        if png_bytes.len().saturating_sub(cursor) < 12 {
            return Err(CodeScanError::ImageInvalid);
        }
        let length = u32::from_be_bytes(
            png_bytes[cursor..cursor + 4]
                .try_into()
                .map_err(|_| CodeScanError::ImageInvalid)?,
        ) as usize;
        let end = cursor
            .checked_add(12)
            .and_then(|value| value.checked_add(length))
            .ok_or(CodeScanError::ImageInvalid)?;
        if end > png_bytes.len() {
            return Err(CodeScanError::ImageInvalid);
        }
        let chunk_type = &png_bytes[cursor + 4..cursor + 8];
        cursor = end;
        if chunk_type == b"IEND" {
            return (length == 0 && cursor == png_bytes.len())
                .then_some(())
                .ok_or(CodeScanError::ImageInvalid);
        }
    }
}

fn validate_dimensions(width: u32, height: u32) -> Result<usize, CodeScanError> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(CodeScanError::ImageTooLarge)?;
    if width == 0
        || height == 0
        || width > MAX_IMAGE_EDGE
        || height > MAX_IMAGE_EDGE
        || pixels > MAX_IMAGE_PIXELS
    {
        return Err(CodeScanError::ImageTooLarge);
    }
    usize::try_from(pixels).map_err(|_| CodeScanError::ImageTooLarge)
}

fn reserve_zeroed(length: usize) -> Result<Vec<u8>, CodeScanError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| CodeScanError::DecodeFailed)?;
    bytes.resize(length, 0);
    Ok(bytes)
}

fn convert_to_luma_in_place(
    decoded: &mut Vec<u8>,
    color_type: png::ColorType,
    pixel_count: usize,
) -> Result<(), CodeScanError> {
    let channels = match color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err(CodeScanError::ImageInvalid),
    };
    let source_len = pixel_count
        .checked_mul(channels)
        .ok_or(CodeScanError::ImageTooLarge)?;
    if decoded.len() < source_len {
        return Err(CodeScanError::ImageInvalid);
    }
    match color_type {
        png::ColorType::Grayscale => {}
        png::ColorType::GrayscaleAlpha => {
            for index in 0..pixel_count {
                let input = index * 2;
                decoded[index] = alpha_over_white(decoded[input], decoded[input + 1]);
            }
        }
        png::ColorType::Rgb => {
            for index in 0..pixel_count {
                let input = index * 3;
                decoded[index] = luminance(decoded[input], decoded[input + 1], decoded[input + 2]);
            }
        }
        png::ColorType::Rgba => {
            for index in 0..pixel_count {
                let input = index * 4;
                let value = luminance(decoded[input], decoded[input + 1], decoded[input + 2]);
                decoded[index] = alpha_over_white(value, decoded[input + 3]);
            }
        }
        png::ColorType::Indexed => unreachable!("索引色已在前面拒绝"),
    }
    decoded.truncate(pixel_count);
    Ok(())
}

fn luminance(red: u8, green: u8, blue: u8) -> u8 {
    ((u32::from(red) * 299 + u32::from(green) * 587 + u32::from(blue) * 114 + 500) / 1000) as u8
}

fn alpha_over_white(value: u8, alpha: u8) -> u8 {
    ((u32::from(value) * u32::from(alpha) + 255 * u32::from(255 - alpha) + 127) / 255) as u8
}

fn downscale_if_needed(image: &mut PreparedLuma) -> Result<(), CodeScanError> {
    let longest = image.width.max(image.height);
    if longest <= MAX_SCAN_EDGE {
        return Ok(());
    }
    let target_width =
        (u64::from(image.width) * u64::from(MAX_SCAN_EDGE) / u64::from(longest)).max(1) as u32;
    let target_height =
        (u64::from(image.height) * u64::from(MAX_SCAN_EDGE) / u64::from(longest)).max(1) as u32;
    let target_len = usize::try_from(u64::from(target_width) * u64::from(target_height))
        .map_err(|_| CodeScanError::ImageTooLarge)?;
    let mut scaled = reserve_zeroed(target_len)?;
    for target_y in 0..target_height {
        let source_y =
            (u64::from(target_y) * u64::from(image.height) / u64::from(target_height)) as u32;
        for target_x in 0..target_width {
            let source_x =
                (u64::from(target_x) * u64::from(image.width) / u64::from(target_width)) as u32;
            let target_index = (target_y as usize) * (target_width as usize) + target_x as usize;
            let source_index = (source_y as usize) * (image.width as usize) + source_x as usize;
            scaled[target_index] = image.pixels[source_index];
        }
    }
    image.pixels = scaled;
    image.width = target_width;
    image.height = target_height;
    Ok(())
}

fn possible_formats() -> HashSet<BarcodeFormat> {
    HashSet::from([BarcodeFormat::QR_CODE, BarcodeFormat::CODE_39])
}

fn stable_format(format: &BarcodeFormat) -> Option<&'static str> {
    match format {
        BarcodeFormat::QR_CODE => Some("qr_code"),
        BarcodeFormat::CODE_39 => Some("code_39"),
        _ => None,
    }
}

fn remap_points(
    points: &[Point],
    scale_x: f32,
    scale_y: f32,
    original_width: u32,
    original_height: u32,
) -> Vec<CodePoint> {
    points
        .iter()
        .filter_map(|point| {
            let x = point.x * scale_x;
            let y = point.y * scale_y;
            (x.is_finite()
                && y.is_finite()
                && x >= 0.0
                && y >= 0.0
                && x <= original_width as f32
                && y <= original_height as f32)
                .then_some(CodePoint { x, y })
        })
        .collect()
}

fn scan_prepared_luma(image: PreparedLuma) -> Result<CodeScanResponse, CodeScanError> {
    let scale_x = image.original_width as f32 / image.width as f32;
    let scale_y = image.original_height as f32 / image.height as f32;
    let mut hints = DecodeHints {
        PossibleFormats: Some(possible_formats()),
        TryHarder: Some(true),
        AlsoInverted: Some(true),
        ..DecodeHints::default()
    };
    let decoded = rxing::helpers::detect_multiple_in_luma_with_hints(
        image.pixels,
        image.width,
        image.height,
        &mut hints,
    );
    let results = match decoded {
        Ok(results) => results,
        Err(Exceptions::NotFoundException(_)) => return Ok(CodeScanResponse::default()),
        Err(_) => return Err(CodeScanError::DecodeFailed),
    };
    let candidates = results
        .into_iter()
        .filter_map(|result| {
            Some(Candidate {
                format: stable_format(result.getBarcodeFormat())?,
                text: result.getText().to_owned(),
                points: remap_points(
                    result.getPoints(),
                    scale_x,
                    scale_y,
                    image.original_width,
                    image.original_height,
                ),
            })
        })
        .collect();
    Ok(apply_output_limits(candidates))
}

/// 在 worker 中执行完整扫描。PNG 参数只在该函数内存活，不会写入日志或缓存。
pub(crate) fn scan_png(png_bytes: Vec<u8>) -> Result<CodeScanResponse, CodeScanError> {
    scan_prepared_luma(decode_png_to_luma(&png_bytes)?)
}

/// 扫描截图会话已经验证过的原始 RGBA 选区；像素不写入日志、缓存或数据库。
pub(crate) fn scan_rgba(
    rgba: Vec<u8>,
    width: u32,
    height: u32,
) -> Result<CodeScanResponse, CodeScanError> {
    scan_prepared_luma(rgba_to_luma(rgba, width, height)?)
}

fn candidate_anchor(candidate: &Candidate) -> (f32, f32) {
    candidate
        .points
        .iter()
        .min_by(|left, right| {
            left.y
                .total_cmp(&right.y)
                .then_with(|| left.x.total_cmp(&right.x))
        })
        .map(|point| (point.y, point.x))
        // 无定位点的结果不能抢占真实图片坐标；把它稳定地放到末尾。
        .unwrap_or((f32::INFINITY, f32::INFINITY))
}

fn apply_output_limits(mut candidates: Vec<Candidate>) -> CodeScanResponse {
    candidates.sort_by(|left, right| {
        let (left_top, left_left) = candidate_anchor(left);
        let (right_top, right_left) = candidate_anchor(right);
        left_top
            .total_cmp(&right_top)
            .then_with(|| left_left.total_cmp(&right_left))
            .then_with(|| left.format.cmp(right.format))
            .then_with(|| left.text.cmp(&right.text))
    });
    let mut seen = HashSet::new();
    let mut results = Vec::new();
    let mut text_bytes = 0usize;
    let mut limited = false;
    for candidate in candidates {
        let key = dedupe_key(&candidate);
        if !seen.insert(key) {
            continue;
        }
        let candidate_text_bytes = candidate.text.len();
        if candidate_text_bytes > MAX_TEXT_BYTES_PER_RESULT
            || text_bytes.saturating_add(candidate_text_bytes) > MAX_TOTAL_TEXT_BYTES
            || results.len() >= MAX_RESULTS
        {
            limited = true;
            continue;
        }
        text_bytes += candidate_text_bytes;
        results.push(CodeScanResult {
            format: candidate.format.to_owned(),
            text: candidate.text,
            points: candidate.points,
        });
    }
    CodeScanResponse { results, limited }
}

fn dedupe_key(candidate: &Candidate) -> (String, String, Vec<(i32, i32)>) {
    let points = candidate
        .points
        .iter()
        .map(|point| {
            (
                (point.x * 4.0).round() as i32,
                (point.y * 4.0).round() as i32,
            )
        })
        .collect();
    (candidate.format.to_owned(), candidate.text.clone(), points)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn png_from_luma(width: u32, height: u32, luma: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("测试 PNG 头");
            writer.write_image_data(luma).expect("测试 PNG 像素");
        }
        out
    }

    fn png_from_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("测试 PNG 头");
            writer.write_image_data(rgba).expect("测试 PNG 像素");
        }
        out
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            }
        }
        !crc
    }

    fn forged_dimensions(width: u32, height: u32) -> Vec<u8> {
        let mut png = png_from_luma(1, 1, &[255]);
        png[16..20].copy_from_slice(&width.to_be_bytes());
        png[20..24].copy_from_slice(&height.to_be_bytes());
        let ihdr_crc = crc32(&png[12..29]);
        png[29..33].copy_from_slice(&ihdr_crc.to_be_bytes());
        png
    }

    fn corrupt_ihdr_crc() -> Vec<u8> {
        let mut png = png_from_luma(1, 1, &[255]);
        png[29] ^= 1;
        png
    }

    fn candidate(format: &'static str, text: &str, points: &[(f32, f32)]) -> Candidate {
        Candidate {
            format,
            text: text.to_owned(),
            points: points
                .iter()
                .map(|(x, y)| CodePoint { x: *x, y: *y })
                .collect(),
        }
    }

    #[test]
    fn strict_png_rejects_invalid_truncated_and_trailing_data() {
        assert!(matches!(
            decode_png_to_luma(b"not png"),
            Err(CodeScanError::ImageInvalid)
        ));
        let png = png_from_luma(1, 1, &[255]);
        assert!(matches!(
            decode_png_to_luma(&png[..png.len() - 1]),
            Err(CodeScanError::ImageInvalid)
        ));
        let mut trailing = png;
        trailing.push(0);
        assert!(matches!(
            decode_png_to_luma(&trailing),
            Err(CodeScanError::ImageInvalid)
        ));
    }

    #[test]
    fn dimensions_are_checked_before_pixel_output_is_allocated() {
        let oversized = forged_dimensions(MAX_IMAGE_EDGE + 1, 1);
        assert!(matches!(
            decode_png_to_luma(&oversized),
            Err(CodeScanError::ImageTooLarge)
        ));
    }

    #[test]
    fn pixel_count_and_crc_failures_are_rejected() {
        let oversized = forged_dimensions(10_000, 10_000);
        assert!(matches!(
            decode_png_to_luma(&oversized),
            Err(CodeScanError::ImageTooLarge)
        ));
        assert!(matches!(
            decode_png_to_luma(&corrupt_ihdr_crc()),
            Err(CodeScanError::ImageInvalid)
        ));
    }

    #[test]
    fn byte_limit_is_enforced_before_png_container_parsing() {
        let oversized = vec![0; MAX_PNG_BYTES + 1];
        assert!(matches!(
            decode_png_to_luma(&oversized),
            Err(CodeScanError::ImageTooLarge)
        ));
    }

    #[test]
    fn transparent_black_is_composited_over_white_before_grayscale() {
        let png = png_from_rgba(2, 1, &[0, 0, 0, 0, 0, 0, 0, 128]);
        let decoded = decode_png_to_luma(&png).expect("应当解码");
        assert_eq!(decoded.pixels, [255, 127]);

        let direct =
            rgba_to_luma(vec![0, 0, 0, 0, 0, 0, 0, 128], 2, 1).expect("可信 RGBA 应当转换");
        assert_eq!(direct.pixels, decoded.pixels);
        assert!(matches!(
            rgba_to_luma(vec![0; 7], 2, 1),
            Err(CodeScanError::ImageInvalid)
        ));
    }

    #[test]
    fn oversized_scan_is_downscaled_and_remapped_points_stay_in_original_space() {
        let mut image = PreparedLuma {
            pixels: vec![255; 5_121],
            width: 5_121,
            height: 1,
            original_width: 5_121,
            original_height: 1,
        };
        downscale_if_needed(&mut image).expect("缩放");
        assert_eq!((image.width, image.height), (5_120, 1));
        let points = remap_points(
            &[Point::new(5_119.0, 0.0)],
            5_121.0 / 5_120.0,
            1.0,
            5_121,
            1,
        );
        assert!(points[0].x <= 5_121.0);
        assert!(points[0].x > 5_119.0);
    }

    #[test]
    fn nonfinite_and_out_of_bounds_decoder_points_are_not_exposed() {
        let points = remap_points(
            &[
                Point::new(1.0, 2.0),
                Point::new(f32::NAN, 2.0),
                Point::new(-1.0, 2.0),
                Point::new(11.0, 2.0),
            ],
            1.0,
            1.0,
            10,
            10,
        );
        assert_eq!(points, [CodePoint { x: 1.0, y: 2.0 }]);
    }

    #[test]
    fn every_ipc_error_serializes_to_its_stable_code_object_only() {
        let cases = [
            (CodeScanError::ClipNotFound, "clip_not_found"),
            (CodeScanError::NotImage, "not_image"),
            (CodeScanError::ImageMissing, "image_missing"),
            (CodeScanError::ImageTooLarge, "image_too_large"),
            (CodeScanError::ImageInvalid, "image_invalid"),
            (CodeScanError::Busy, "busy"),
            (CodeScanError::WorkerFailed, "worker_failed"),
            (CodeScanError::DecodeFailed, "decode_failed"),
            (CodeScanError::StorageFailed, "storage_failed"),
            (CodeScanError::CaptureUnavailable, "capture_unavailable"),
        ];
        for (error, code) in cases {
            assert_eq!(
                serde_json::to_value(error).expect("序列化错误对象"),
                serde_json::json!({ "code": code })
            );
        }
    }

    #[test]
    fn first_slice_allowlist_is_exactly_qr_and_code_39() {
        assert_eq!(
            possible_formats(),
            HashSet::from([BarcodeFormat::QR_CODE, BarcodeFormat::CODE_39])
        );
        assert_eq!(stable_format(&BarcodeFormat::QR_CODE), Some("qr_code"));
        assert_eq!(stable_format(&BarcodeFormat::CODE_39), Some("code_39"));
        assert_eq!(stable_format(&BarcodeFormat::MICRO_QR_CODE), None);
        assert_eq!(stable_format(&BarcodeFormat::CODE_128), None);
    }

    #[test]
    fn output_limits_sort_and_dedupe_without_merging_distinct_locations() {
        let oversized = "x".repeat(MAX_TEXT_BYTES_PER_RESULT + 1);
        let response = apply_output_limits(vec![
            candidate("qr_code", "later", &[(90.0, 20.0)]),
            candidate("qr_code", "first", &[(40.0, 10.0)]),
            candidate("qr_code", "first", &[(40.02, 10.01)]),
            candidate("qr_code", "first", &[(50.0, 10.0)]),
            candidate("qr_code", &oversized, &[(0.0, 0.0)]),
        ]);
        assert!(response.limited);
        assert_eq!(response.results.len(), 3);
        assert_eq!(response.results[0].text, "first");
        assert_eq!(response.results[1].text, "first");
        assert_eq!(response.results[2].text, "later");
        assert_eq!(
            response.results[0].points[0],
            CodePoint { x: 40.0, y: 10.0 }
        );
        assert_eq!(
            response.results[1].points[0],
            CodePoint { x: 50.0, y: 10.0 }
        );
    }

    #[test]
    fn result_anchor_uses_the_topmost_then_leftmost_point() {
        let candidate = candidate("qr_code", "payload", &[(4.0, 20.0), (9.0, 5.0), (2.0, 5.0)]);
        assert_eq!(candidate_anchor(&candidate), (5.0, 2.0));
    }

    #[test]
    fn output_caps_drop_whole_results_after_result_and_total_text_limits() {
        let response = apply_output_limits(
            (0..(MAX_RESULTS + 2))
                .map(|index| candidate("qr_code", &index.to_string(), &[(index as f32, 0.0)]))
                .collect(),
        );
        assert!(response.limited);
        assert_eq!(response.results.len(), MAX_RESULTS);

        let text = "a".repeat(MAX_TEXT_BYTES_PER_RESULT);
        let response = apply_output_limits(
            (0..5)
                .map(|index| candidate("qr_code", &text, &[(index as f32, 0.0)]))
                .collect(),
        );
        assert!(response.limited);
        assert_eq!(response.results.len(), 4);
    }

    #[test]
    fn empty_image_is_a_successful_empty_result() {
        let png = png_from_luma(64, 64, &vec![255; 64 * 64]);
        assert_eq!(scan_png(png), Ok(CodeScanResponse::default()));
    }

    fn fixture_from_modules(rows: &[&str], quiet_modules: usize, module: usize) -> Vec<u8> {
        let side = rows.len() + quiet_modules * 2;
        let mut pixels = vec![255; side * module * side * module];
        for (row, bits) in rows.iter().enumerate() {
            for (column, bit) in bits.bytes().enumerate() {
                if bit != b'#' {
                    continue;
                }
                for y in 0..module {
                    for x in 0..module {
                        let output_y = (row + quiet_modules) * module + y;
                        let output_x = (column + quiet_modules) * module + x;
                        pixels[output_y * side * module + output_x] = 0;
                    }
                }
            }
        }
        png_from_luma((side * module) as u32, (side * module) as u32, &pixels)
    }

    fn qr_fixture() -> Vec<u8> {
        let rows = [
            "#######...#.#.#######",
            "#.....#.#.#.#.#.....#",
            "#.###.#.#.##..#.###.#",
            "#.###.#.....#.#.###.#",
            "#.###.#.#####.#.###.#",
            "#.....#.###...#.....#",
            "#######.#.#.#.#######",
            "........#............",
            "##.#..##..###.###.##.",
            "..###..##.##.#....##.",
            "##.#####...#..##..#.#",
            "..#..#.##.#.#.#..#...",
            "#####.#...##.###..#.#",
            "........#..#...#.##..",
            "#######.#.###.#.#.##.",
            "#.....#..#.#.#.....#.",
            "#.###.#..#.###..###..",
            "#.###.#.###........##",
            "#.###.#....######.#.#",
            "#.....#.###....##....",
            "#######.#..####.#.##.",
        ];
        fixture_from_modules(&rows, 4, 6)
    }

    #[test]
    fn independently_encoded_qr_fixture_is_decoded() {
        // 固定 QR 版本 1 矩阵，由独立 QR 生成器产生后作为测试向量固化；测试不调用 rxing 编码器。
        let response = scan_png(qr_fixture()).expect("二维码应识别");
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].format, "qr_code");
        assert_eq!(response.results[0].text, "CLIPPY-QR-1D");
    }

    fn luma_from_png(png: &[u8]) -> PreparedLuma {
        decode_png_to_luma(png).expect("测试 fixture 应当合法")
    }

    #[test]
    fn inverted_and_ninety_degree_rotated_qr_are_decoded() {
        let original = luma_from_png(&qr_fixture());
        let inverted: Vec<u8> = original.pixels.iter().map(|value| 255 - value).collect();
        let inverted = png_from_luma(original.width, original.height, &inverted);
        assert!(scan_png(inverted)
            .expect("反色二维码应识别")
            .results
            .iter()
            .any(|result| result.text == "CLIPPY-QR-1D"));

        let mut rotated = vec![255; original.pixels.len()];
        for y in 0..original.height as usize {
            for x in 0..original.width as usize {
                let target_x = original.height as usize - 1 - y;
                let target_y = x;
                rotated[target_y * original.height as usize + target_x] =
                    original.pixels[y * original.width as usize + x];
            }
        }
        let rotated = png_from_luma(original.height, original.width, &rotated);
        assert!(scan_png(rotated)
            .expect("旋转二维码应识别")
            .results
            .iter()
            .any(|result| result.text == "CLIPPY-QR-1D"));
    }

    #[test]
    fn two_separated_qr_codes_are_reported_as_distinct_results() {
        let single = luma_from_png(&qr_fixture());
        let gap = 30usize;
        let width = single.width as usize * 2 + gap;
        let height = single.height as usize;
        let mut combined = vec![255; width * height];
        for y in 0..height {
            let source = &single.pixels[y * single.width as usize..(y + 1) * single.width as usize];
            combined[y * width..y * width + single.width as usize].copy_from_slice(source);
            let offset = y * width + single.width as usize + gap;
            combined[offset..offset + single.width as usize].copy_from_slice(source);
        }
        let response = scan_png(png_from_luma(width as u32, height as u32, &combined))
            .expect("多码图片应识别");
        assert_eq!(
            response
                .results
                .iter()
                .filter(|result| result.text == "CLIPPY-QR-1D")
                .count(),
            2
        );
    }

    fn code_39_fixture() -> Vec<u8> {
        // Code 39 的 *A*：窄/宽序列来自规范表，独立于被测解码器实现。
        let patterns = ["nwnnwnwnn", "wnnnnwnnw", "nwnnwnwnn"];
        let module = 4usize;
        let quiet = 10usize;
        let mut runs = VecDeque::new();
        for (index, pattern) in patterns.iter().enumerate() {
            for width in pattern.bytes().map(|item| if item == b'w' { 3 } else { 1 }) {
                runs.push_back(width);
            }
            if index + 1 < patterns.len() {
                runs.push_back(1);
            }
        }
        let barcode_modules: usize = runs.iter().sum();
        let width = (quiet * 2 + barcode_modules) * module;
        let height = 80usize;
        let mut pixels = vec![255; width * height];
        let mut x = quiet * module;
        let mut black = true;
        for run in runs {
            let run_width = run * module;
            if black {
                for row in 0..height {
                    pixels[row * width + x..row * width + x + run_width].fill(0);
                }
            }
            x += run_width;
            black = !black;
        }
        png_from_luma(width as u32, height as u32, &pixels)
    }

    #[test]
    fn independently_encoded_code_39_fixture_is_decoded() {
        let response = scan_png(code_39_fixture()).expect("一维码应识别");
        assert!(response
            .results
            .iter()
            .any(|result| result.format == "code_39" && result.text == "A"));
    }
}
