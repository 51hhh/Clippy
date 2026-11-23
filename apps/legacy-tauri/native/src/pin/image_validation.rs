//! Pin 图片在分配 RGBA 与进入运行时状态之前的统一资源边界。

use image::RgbaImage;
use std::io::Cursor;

const MAX_IMAGE_DIMENSION: u32 = 32_768;
const MAX_IMAGE_PIXELS: u64 = 64 * 1024 * 1024;
const MAX_DECODER_WORK_BYTES: usize = 128 * 1024 * 1024;

fn validate_byte_length(length: usize, byte_limit: usize, name: &str) -> Result<(), String> {
    if length > byte_limit {
        return Err(format!("{name}超过 {} MiB 上限", byte_limit / 1024 / 1024));
    }
    Ok(())
}

/// `png::Reader::finish` 会校验到 IEND，但底层 `BufReader` 可能提前读入尾随字节，无法用
/// Cursor 位置判断文件是否恰好结束。先做一次零分配 chunk 边界扫描，拒绝 IEND 后载荷。
fn validate_container_layout(png: &[u8], name: &str) -> Result<(), String> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if !png.starts_with(SIGNATURE) {
        return Err(format!("{name}不是合法 PNG"));
    }

    let mut cursor = SIGNATURE.len();
    loop {
        if png.len().saturating_sub(cursor) < 12 {
            return Err(format!("{name}的 PNG chunk 被截断"));
        }
        let length = u32::from_be_bytes(
            png[cursor..cursor + 4]
                .try_into()
                .map_err(|_| format!("{name}的 PNG chunk 长度无效"))?,
        ) as usize;
        let end = cursor
            .checked_add(12)
            .and_then(|value| value.checked_add(length))
            .ok_or_else(|| format!("{name}的 PNG chunk 长度溢出"))?;
        if end > png.len() {
            return Err(format!("{name}的 PNG chunk 被截断"));
        }
        let chunk_type = &png[cursor + 4..cursor + 8];
        cursor = end;
        if chunk_type == b"IEND" {
            if length != 0 || cursor != png.len() {
                return Err(format!("{name}的 PNG IEND 无效"));
            }
            return Ok(());
        }
    }
}

fn validate_dimensions(width: u32, height: u32, name: &str) -> Result<usize, String> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| format!("{name}尺寸超过安全上限"))?;
    if width == 0
        || height == 0
        || width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || pixels > MAX_IMAGE_PIXELS
    {
        return Err(format!("{name}尺寸超过安全上限"));
    }
    usize::try_from(pixels).map_err(|_| format!("{name}尺寸超过平台上限"))
}

fn reserve_zeroed(length: usize, name: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| format!("{name}像素缓冲区分配失败"))?;
    bytes.resize(length, 0);
    Ok(bytes)
}

fn into_rgba(
    mut decoded: Vec<u8>,
    info: png::OutputInfo,
    pixel_count: usize,
    name: &str,
) -> Result<RgbaImage, String> {
    if info.bit_depth != png::BitDepth::Eight {
        return Err(format!("{name}无法规范化为 8 位像素"));
    }

    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err(format!("{name}调色板展开失败")),
    };
    let source_len = pixel_count
        .checked_mul(channels)
        .ok_or_else(|| format!("{name}像素长度溢出"))?;
    if info.buffer_size() != source_len || decoded.len() < source_len {
        return Err(format!("{name}解码像素长度不匹配"));
    }
    decoded.truncate(source_len);

    let rgba_len = pixel_count
        .checked_mul(4)
        .ok_or_else(|| format!("{name}RGBA 长度溢出"))?;
    if decoded.len() < rgba_len {
        decoded
            .try_reserve_exact(rgba_len - decoded.len())
            .map_err(|_| format!("{name}RGBA 缓冲区分配失败"))?;
        decoded.resize(rgba_len, 0);
    }

    match info.color_type {
        png::ColorType::Grayscale => {
            for index in (0..pixel_count).rev() {
                let gray = decoded[index];
                let output = index * 4;
                decoded[output..output + 4].copy_from_slice(&[gray, gray, gray, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for index in (0..pixel_count).rev() {
                let input = index * 2;
                let gray = decoded[input];
                let alpha = decoded[input + 1];
                let output = index * 4;
                decoded[output..output + 4].copy_from_slice(&[gray, gray, gray, alpha]);
            }
        }
        png::ColorType::Rgb => {
            for index in (0..pixel_count).rev() {
                let input = index * 3;
                let red = decoded[input];
                let green = decoded[input + 1];
                let blue = decoded[input + 2];
                let output = index * 4;
                decoded[output..output + 4].copy_from_slice(&[red, green, blue, 255]);
            }
        }
        png::ColorType::Rgba => {}
        png::ColorType::Indexed => unreachable!("调色板已在前面拒绝"),
    }

    RgbaImage::from_raw(info.width, info.height, decoded)
        .ok_or_else(|| format!("{name}RGBA 像素长度不匹配"))
}

/// 严格校验调用方给出的原始 PNG 字节，不剥离或容忍任何辅助块。
pub(super) fn decode_strict_png(
    png: &[u8],
    byte_limit: usize,
    name: &str,
) -> Result<RgbaImage, String> {
    validate_byte_length(png.len(), byte_limit, name)?;
    validate_container_layout(png, name)?;

    let mut options = png::DecodeOptions::default();
    options.set_ignore_adler32(false);
    options.set_ignore_crc(false);
    options.set_skip_ancillary_crc_failures(false);
    let mut decoder = png::Decoder::new_with_options(Cursor::new(png), options);
    decoder.set_limits(png::Limits {
        bytes: MAX_DECODER_WORK_BYTES,
    });
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|_| format!("{name}不是合法 PNG"))?;
    let (width, height) = reader.info().size();
    let pixel_count = validate_dimensions(width, height, name)?;
    let rgba_len = pixel_count
        .checked_mul(4)
        .ok_or_else(|| format!("{name}RGBA 长度溢出"))?;
    let output_len = reader.output_buffer_size();
    if output_len > rgba_len {
        return Err(format!("{name}解码缓冲区超过安全上限"));
    }
    let mut decoded = reserve_zeroed(output_len, name)?;
    let info = reader
        .next_frame(&mut decoded)
        .map_err(|_| format!("{name}无法完整解码"))?;
    reader.finish().map_err(|_| format!("{name}无法完整解码"))?;
    if (info.width, info.height) != (width, height) {
        return Err(format!("{name}解码尺寸不匹配"));
    }
    into_rgba(decoded, info, pixel_count, name)
}

pub(super) fn validate_strict_png(
    png: &[u8],
    byte_limit: usize,
    name: &str,
) -> Result<(u32, u32), String> {
    Ok(decode_strict_png(png, byte_limit, name)?.dimensions())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded_png(color: png::ColorType, pixels: &[u8]) -> Vec<u8> {
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(color);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("测试 PNG 头");
            writer.write_image_data(pixels).expect("测试像素");
        }
        png
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
        let mut png = crate::screenshot::encode_png(&[1, 2, 3, 255], 1, 1).expect("测试 PNG");
        png[16..20].copy_from_slice(&width.to_be_bytes());
        png[20..24].copy_from_slice(&height.to_be_bytes());
        let ihdr_crc = crc32(&png[12..29]);
        png[29..33].copy_from_slice(&ihdr_crc.to_be_bytes());
        png
    }

    fn corrupt_chunk_crc(mut png: Vec<u8>, wanted: &[u8; 4]) -> Vec<u8> {
        let mut cursor = 8usize;
        while cursor < png.len() {
            let length = u32::from_be_bytes(png[cursor..cursor + 4].try_into().unwrap()) as usize;
            let end = cursor + length + 12;
            if &png[cursor + 4..cursor + 8] == wanted {
                png[end - 1] ^= 1;
                return png;
            }
            cursor = end;
        }
        panic!("测试 PNG 缺少 {wanted:?}");
    }

    fn corrupt_project_chunk_crc() -> Vec<u8> {
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("测试 PNG 头");
            let chunk =
                png::text_metadata::ITXtChunk::new(super::super::project::PROJECT_KEYWORD, "{}");
            writer.write_text_chunk(&chunk).expect("测试 iTXt");
            writer.write_image_data(&[1, 2, 3, 255]).expect("测试像素");
        }
        corrupt_chunk_crc(png, b"iTXt")
    }

    #[test]
    fn accepts_a_fully_decodable_png_and_returns_its_dimensions() {
        let png = crate::screenshot::encode_png(&[1, 2, 3, 255], 1, 1).expect("测试 PNG");
        assert_eq!(
            validate_strict_png(
                &png,
                super::super::project::MAX_RENDERED_PNG_BYTES,
                "截图 PNG"
            ),
            Ok((1, 1))
        );
    }

    #[test]
    fn rejects_truncated_non_png_and_corrupt_auxiliary_chunks() {
        let png = crate::screenshot::encode_png(&[1, 2, 3, 255], 1, 1).expect("测试 PNG");
        assert!(validate_strict_png(&png[..png.len() / 2], usize::MAX, "截图 PNG").is_err());
        assert!(validate_strict_png(b"not a png", usize::MAX, "截图 PNG").is_err());

        let corrupt = corrupt_project_chunk_crc();
        assert!(
            super::super::project::validate_rendered_png(&corrupt).is_ok(),
            "工程容器继续容忍损坏的工程辅助块"
        );
        assert!(validate_strict_png(&corrupt, usize::MAX, "截图 PNG").is_err());
    }

    #[test]
    fn validates_image_data_iend_and_exact_container_end() {
        let png = crate::screenshot::encode_png(&[1, 2, 3, 255], 1, 1).expect("测试 PNG");
        assert!(validate_strict_png(
            &corrupt_chunk_crc(png.clone(), b"IDAT"),
            usize::MAX,
            "截图 PNG"
        )
        .is_err());
        assert!(validate_strict_png(
            &corrupt_chunk_crc(png.clone(), b"IEND"),
            usize::MAX,
            "截图 PNG"
        )
        .is_err());

        let mut trailing = png;
        trailing.extend_from_slice(b"trailing bytes");
        assert!(validate_strict_png(&trailing, usize::MAX, "截图 PNG").is_err());
    }

    #[test]
    fn normalizes_supported_eight_bit_color_types_to_rgba() {
        for (color, source, expected) in [
            (png::ColorType::Grayscale, &[7][..], [7, 7, 7, 255]),
            (png::ColorType::GrayscaleAlpha, &[8, 9][..], [8, 8, 8, 9]),
            (png::ColorType::Rgb, &[10, 11, 12][..], [10, 11, 12, 255]),
            (
                png::ColorType::Rgba,
                &[13, 14, 15, 16][..],
                [13, 14, 15, 16],
            ),
        ] {
            let image = decode_strict_png(&encoded_png(color, source), usize::MAX, "截图 PNG")
                .expect("颜色类型应规范化");
            assert_eq!(image.as_raw(), &expected);
        }
    }

    #[test]
    fn rejects_oversized_bytes_without_allocating_the_payload() {
        let limit = super::super::project::MAX_RENDERED_PNG_BYTES;
        assert!(validate_byte_length(limit, limit, "截图 PNG").is_ok());
        assert!(validate_byte_length(limit + 1, limit, "截图 PNG").is_err());
    }

    #[test]
    fn rejects_dimensions_before_decoding_the_forged_pixel_stream() {
        for (width, height) in [(32_769, 1), (32_768, 2_049)] {
            let png = forged_dimensions(width, height);
            assert_eq!(
                crate::screenshot::png_dimensions(&png).expect("IHDR 应可读取"),
                (width, height)
            );
            assert!(validate_strict_png(&png, usize::MAX, "截图 PNG").is_err());
        }
    }
}
