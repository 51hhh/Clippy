//! 解码经过产物校验的 VP9 首帧，窗口端无需再次编码 PNG。

use super::manifest::RecordingThumbnailSource;

pub const MAX_THUMBNAIL_EDGE: u32 = 320;
pub(super) const MAX_DECODE_PIXELS: u64 = 16_777_216;

pub fn decode_thumbnail(source: &RecordingThumbnailSource) -> Result<image::RgbaImage, String> {
    use super::mux::webm_remux::{
        first_vp9_keyframe, WebmRemuxSource, WebmRemuxSpec, WebmThumbnailAudioSpec,
    };
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
        source.audio.map(|audio| WebmThumbnailAudioSpec {
            sample_rate_hz: audio.sample_rate_hz,
            channels: audio.channels,
            pre_skip_frames: audio.pre_skip_frames,
            codec_delay_ns: audio.codec_delay_ns,
            seek_pre_roll_ns: audio.seek_pre_roll_ns,
            packet_count: audio.packet_count,
            pcm_frame_count: audio.pcm_frame_count,
        }),
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
    Ok(thumbnail)
}

fn require_first_frame<T>(frame: Option<T>) -> Result<T, String> {
    frame.ok_or_else(|| "VP9 缩略图 packet 没有输出画面".to_string())
}

fn reject_additional_frame<T>(frame: Option<T>, message: &'static str) -> Result<(), String> {
    if frame.is_some() {
        Err(message.to_string())
    } else {
        Ok(())
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
