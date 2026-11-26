//! 解码经过产物校验的 VP9 首帧，窗口端无需再次编码 PNG。

use super::manifest::RecordingThumbnailSource;

pub const MAX_THUMBNAIL_EDGE: u32 = 320;
pub(super) const MAX_DECODE_PIXELS: u64 = 16_777_216;

pub fn decode_thumbnail(source: &RecordingThumbnailSource) -> Result<image::RgbaImage, String> {
    use super::mux::webm_remux::{
        first_vp9_keyframe, WebmRemuxSource, WebmRemuxSpec, WebmThumbnailAudioSpec,
    };
    use shiguredo_libvpx::{Decoder, DecoderCodec, DecoderConfig};

    u64::from(source.width)
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
    let image = {
        let frame = require_first_frame(
            decoder
                .next_frame()
                .map_err(|error| format!("读取 VP9 缩略图帧失败: {error}"))?,
        )?;
        vp9_frame_rgba(&frame, source.width, source.height)?
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

    let thumbnail = image::DynamicImage::ImageRgba8(image)
        .thumbnail(MAX_THUMBNAIL_EDGE, MAX_THUMBNAIL_EDGE)
        .to_rgba8();
    Ok(thumbnail)
}

pub(super) fn vp9_frame_rgba(
    frame: &shiguredo_libvpx::DecodedFrame<'_>,
    width: u32,
    height: u32,
) -> Result<image::RgbaImage, String> {
    let (y_stride, u_stride, v_stride) = (frame.y_stride(), frame.u_stride(), frame.v_stride());
    validate_decoded_layout(
        frame.is_high_depth(),
        frame.width(),
        frame.height(),
        y_stride,
        u_stride,
        v_stride,
        width,
        height,
    )?;
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 || pixels > MAX_DECODE_PIXELS {
        return Err("VP9 帧超过解码像素上限".to_string());
    }
    let planar = yuv::YuvPlanarImage {
        y_plane: frame.y_plane(),
        y_stride: y_stride as u32,
        u_plane: frame.u_plane(),
        u_stride: u_stride as u32,
        v_plane: frame.v_plane(),
        v_stride: v_stride as u32,
        width,
        height,
    };
    let mut rgba = vec![0_u8; pixels as usize * 4];
    yuv::yuv420_to_rgba(
        &planar,
        &mut rgba,
        width * 4,
        yuv::YuvRange::Limited,
        yuv::YuvStandardMatrix::Bt709,
    )
    .map_err(|error| format!("转换 VP9 颜色失败: {error}"))?;
    image::RgbaImage::from_raw(width, height, rgba).ok_or_else(|| "构造 VP9 帧像素失败".to_string())
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
        return Err("VP9 帧格式或尺寸与清单不一致".to_string());
    }
    let width = expected_width as usize;
    let chroma_width = width.div_ceil(2);
    // libvpx 的边框和对齐对窄帧也占固定空间，2×2 录像实际返回 96/48/48。
    let luma_limit = width.saturating_mul(4).max(128);
    let chroma_limit = width.saturating_mul(2).max(64);
    if !(width..=luma_limit).contains(&y_stride)
        || !(chroma_width..=chroma_limit).contains(&u_stride)
        || !(chroma_width..=chroma_limit).contains(&v_stride)
    {
        return Err("VP9 帧的 plane stride 越界".to_string());
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
        assert!(validate_decoded_layout(false, 2, 2, 96, 48, 48, 2, 2).is_ok());
        assert!(validate_decoded_layout(true, 640, 360, 640, 320, 320, 640, 360).is_err());
        assert!(validate_decoded_layout(false, 639, 360, 640, 320, 320, 640, 360).is_err());
        assert!(validate_decoded_layout(false, 640, 359, 640, 320, 320, 640, 360).is_err());
        assert!(validate_decoded_layout(false, 640, 360, 639, 320, 320, 640, 360).is_err());
        assert!(validate_decoded_layout(false, 640, 360, 2_561, 320, 320, 640, 360).is_err());
        assert!(validate_decoded_layout(false, 640, 360, 640, 319, 320, 640, 360).is_err());
        assert!(validate_decoded_layout(false, 640, 360, 640, 320, 1_281, 640, 360).is_err());
    }
}
