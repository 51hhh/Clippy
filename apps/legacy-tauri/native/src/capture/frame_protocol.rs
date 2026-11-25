use crate::commands::AppState;
use tauri::{http, Manager, Runtime, UriSchemeContext};

const BMP_HEADER_BYTES: usize = 122;

/// 只允许覆盖层读取与自己 label 对应的冻结帧，避免其它 webview 枚举会话内容。
pub(crate) fn handle<R: Runtime>(
    context: UriSchemeContext<'_, R>,
    request: http::Request<Vec<u8>>,
) -> http::Response<Vec<u8>> {
    let label = request.uri().path().trim_start_matches('/');
    if label != context.webview_label() || super::validate_overlay_label(label).is_err() {
        return response(
            http::StatusCode::FORBIDDEN,
            "text/plain",
            b"forbidden".to_vec(),
        );
    }

    let state = context.app_handle().state::<AppState>();
    match state.capture_manager.frame_source(label) {
        Ok((rgba, width, height)) => match encode_bmp(&rgba, width, height) {
            Ok(bytes) => response(http::StatusCode::OK, "image/bmp", bytes),
            Err(message) => response(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                "text/plain",
                message.into_bytes(),
            ),
        },
        Err(error) => response(
            http::StatusCode::NOT_FOUND,
            "text/plain",
            error.to_string().into_bytes(),
        ),
    }
}

fn response(
    status: http::StatusCode,
    content_type: &'static str,
    body: Vec<u8>,
) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, content_type)
        .header(http::header::CACHE_CONTROL, "no-store")
        .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(body)
        .expect("静态截图协议响应头必须有效")
}

/// WebKit 原生支持的无损 32-bit BMP。负高度表示自顶向下，避免翻转整张 4K 图；
/// BMP 使用 BGRA，因此这里只做一次线性通道交换，不做压缩或降采样。
fn encode_bmp(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let pixel_bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "截图尺寸溢出".to_string())?;
    if width == 0 || height == 0 || rgba.len() != pixel_bytes {
        return Err(format!(
            "冻结帧尺寸不匹配: {width}x{height}, {}/{} bytes",
            rgba.len(),
            pixel_bytes
        ));
    }
    let file_bytes = BMP_HEADER_BYTES
        .checked_add(pixel_bytes)
        .ok_or_else(|| "BMP 大小溢出".to_string())?;
    let file_size = u32::try_from(file_bytes).map_err(|_| "BMP 超过 4 GiB".to_string())?;
    let signed_width = i32::try_from(width).map_err(|_| "BMP 宽度溢出".to_string())?;
    let signed_height = i32::try_from(height).map_err(|_| "BMP 高度溢出".to_string())?;

    let mut bmp = Vec::with_capacity(file_bytes);
    bmp.extend_from_slice(b"BM");
    push_u32(&mut bmp, file_size);
    push_u32(&mut bmp, 0);
    push_u32(&mut bmp, BMP_HEADER_BYTES as u32);
    push_u32(&mut bmp, 108); // BITMAPV4HEADER
    push_i32(&mut bmp, signed_width);
    push_i32(&mut bmp, -signed_height); // top-down
    push_u16(&mut bmp, 1);
    push_u16(&mut bmp, 32);
    push_u32(&mut bmp, 3); // BI_BITFIELDS
    push_u32(&mut bmp, pixel_bytes as u32);
    push_i32(&mut bmp, 2835);
    push_i32(&mut bmp, 2835);
    push_u32(&mut bmp, 0);
    push_u32(&mut bmp, 0);
    push_u32(&mut bmp, 0x00ff_0000);
    push_u32(&mut bmp, 0x0000_ff00);
    push_u32(&mut bmp, 0x0000_00ff);
    push_u32(&mut bmp, 0xff00_0000);
    push_u32(&mut bmp, 0x5769_6e20); // LCS_WINDOWS_COLOR_SPACE ("Win ")
    bmp.resize(BMP_HEADER_BYTES, 0);
    let (pixels, remainder) = rgba.as_chunks::<4>();
    debug_assert!(remainder.is_empty());
    for pixel in pixels {
        bmp.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    Ok(bmp)
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bmp_is_top_down_lossless_bgra() {
        let bmp = encode_bmp(&[0x11, 0x22, 0x33, 0x44], 1, 1).unwrap();
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(u32::from_le_bytes(bmp[10..14].try_into().unwrap()), 122);
        assert_eq!(i32::from_le_bytes(bmp[18..22].try_into().unwrap()), 1);
        assert_eq!(i32::from_le_bytes(bmp[22..26].try_into().unwrap()), -1);
        assert_eq!(&bmp[122..], &[0x33, 0x22, 0x11, 0x44]);
    }

    #[test]
    fn bmp_rejects_invalid_frame_length() {
        assert!(encode_bmp(&[0; 3], 1, 1).is_err());
        assert!(encode_bmp(&[], 0, 0).is_err());
    }
}
