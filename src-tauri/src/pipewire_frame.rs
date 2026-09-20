//! Linux PipeWire 原始视频帧的共享格式合同。
//!
//! 冻结截图与持续录屏必须复用同一套格式协商、stride 校验和 RGBA 转换；否则同一合成器在两个
//! 入口可能得到不同颜色或对行填充作出不同解释。这里只处理可直接映射的 32 位共享内存帧，明确
//! 拒绝 DMA-BUF、缩放和不支持的像素格式。

use anyhow::{anyhow, bail, Context, Result};
use pipewire as pw;
use pw::spa;
use spa::param::format::{MediaSubtype, MediaType};
use spa::param::video::{VideoFormat, VideoInfoRaw};
use spa::pod::Pod;
use std::sync::Arc;

pub(crate) struct PipeWireRgbaFrame {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Arc<[u8]>,
}

pub(crate) fn init_pipewire() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(pw::init);
}

pub(crate) fn parse_video_format(param: &Pod) -> Result<VideoInfoRaw> {
    let (media_type, media_subtype) = spa::param::format_utils::parse_format(param)
        .map_err(|error| anyhow!("无法解析 PipeWire 格式：{error}"))?;
    if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
        bail!("PipeWire 协商出的不是原始视频（{media_type:?}/{media_subtype:?}）");
    }
    let mut info = VideoInfoRaw::new();
    info.parse(param)
        .map_err(|error| anyhow!("无法解析视频格式：{error}"))?;
    Ok(info)
}

pub(crate) fn frame_from_buffer(
    buffer: &mut pw::buffer::Buffer,
    info: VideoInfoRaw,
) -> Result<PipeWireRgbaFrame> {
    let size = info.size();
    let layout = pixel_layout(info.format()).ok_or_else(|| {
        anyhow!(
            "PipeWire 协商出的像素格式 {:?} 不是 32 位 RGB 排列",
            info.format()
        )
    })?;
    let datas = buffer.datas_mut();
    let data = datas.first_mut().context("PipeWire 缓冲里没有数据块")?;
    let kind = data.type_();
    if kind == spa::buffer::DataType::DmaBuf {
        // EnumFormat 没有 modifier 属性，合成器应走 shm/MemFd；MAP_BUFFERS 不会替调用方
        // 映射任意 DMA-BUF，因此收到它必须失败，不能把 fd 当成像素地址。
        bail!("PipeWire 送来的是 DMA-BUF，这条路只处理共享内存");
    }
    let stride = data.chunk().stride();
    if stride <= 0 {
        bail!("PipeWire 帧的 stride 是 {stride}");
    }
    let offset = data.chunk().offset() as usize;
    let pixels = data.data().context("PipeWire 缓冲没有映射到内存")?;
    let pixels = pixels
        .get(offset..)
        .with_context(|| format!("PipeWire 帧的 offset {offset} 越过了缓冲末尾"))?;
    let rgba = repack_to_rgba(pixels, size.width, size.height, stride as usize, layout)?;
    Ok(PipeWireRgbaFrame {
        width: size.width,
        height: size.height,
        rgba: Arc::from(rgba),
    })
}

/// 报给合成器的 `EnumFormat`。不带 modifier，强制协商为能直接读取的共享内存。
pub(crate) fn enum_format_pod() -> Result<Vec<u8>> {
    use spa::pod::{object, property, Value};
    use spa::utils::{Fraction, Rectangle};

    let object = object! {
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        property!(spa::param::format::FormatProperties::MediaType, Id, MediaType::Video),
        property!(spa::param::format::FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        property!(
            spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::RGBx,
            VideoFormat::BGRA,
            VideoFormat::RGBA,
            VideoFormat::xRGB,
            VideoFormat::xBGR,
            VideoFormat::ARGB,
            VideoFormat::ABGR,
        ),
        property!(
            spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            Rectangle { width: 1920, height: 1080 },
            Rectangle { width: 1, height: 1 },
            Rectangle { width: 16384, height: 16384 }
        ),
        property!(
            spa::param::format::FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            Fraction { num: 60, denom: 1 },
            Fraction { num: 0, denom: 1 },
            Fraction { num: 1000, denom: 1 }
        ),
    };

    let bytes = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &Value::Object(object),
    )
    .map_err(|error| anyhow!("无法序列化 EnumFormat：{error}"))?
    .0
    .into_inner();
    Ok(bytes)
}

/// 4 字节像素里 R/G/B 各自的字节下标。SPA 格式名描述内存中的字节顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PixelLayout {
    r: usize,
    g: usize,
    b: usize,
}

fn pixel_layout(format: VideoFormat) -> Option<PixelLayout> {
    const RGB: PixelLayout = PixelLayout { r: 0, g: 1, b: 2 };
    const BGR: PixelLayout = PixelLayout { r: 2, g: 1, b: 0 };
    const ARGB: PixelLayout = PixelLayout { r: 1, g: 2, b: 3 };
    const ABGR: PixelLayout = PixelLayout { r: 3, g: 2, b: 1 };

    if format == VideoFormat::RGBx || format == VideoFormat::RGBA {
        Some(RGB)
    } else if format == VideoFormat::BGRx || format == VideoFormat::BGRA {
        Some(BGR)
    } else if format == VideoFormat::xRGB || format == VideoFormat::ARGB {
        Some(ARGB)
    } else if format == VideoFormat::xBGR || format == VideoFormat::ABGR {
        Some(ABGR)
    } else {
        None
    }
}

/// 去除行尾填充并换成紧排 RGBA8。桌面画面没有透明度，alpha 固定为 255。
fn repack_to_rgba(
    src: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    layout: PixelLayout,
) -> Result<Vec<u8>> {
    let width = width as usize;
    let height = height as usize;
    if width == 0 || height == 0 {
        bail!("PipeWire 帧的尺寸是 {width}x{height}");
    }
    let row = width.checked_mul(4).context("帧宽度溢出")?;
    if stride < row {
        bail!("PipeWire 帧的 stride {stride} 装不下一行 {row} 字节");
    }
    let minimum = stride
        .checked_mul(height - 1)
        .and_then(|value| value.checked_add(row))
        .context("帧尺寸溢出")?;
    if src.len() < minimum {
        bail!(
            "PipeWire 帧只有 {} 字节，装不下 {width}x{height}（stride {stride}）",
            src.len()
        );
    }

    let mut out = vec![0u8; row * height];
    for (y, line) in out.chunks_exact_mut(row).enumerate() {
        let source = &src[y * stride..y * stride + row];
        let (targets, _) = line.as_chunks_mut::<4>();
        let (pixels, _) = source.as_chunks::<4>();
        for (target, pixel) in targets.iter_mut().zip(pixels) {
            target[0] = pixel[layout.r];
            target[1] = pixel[layout.g];
            target[2] = pixel[layout.b];
            target[3] = 0xff;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_advertised_format_has_a_layout() {
        for format in [
            VideoFormat::RGBx,
            VideoFormat::RGBA,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::xRGB,
            VideoFormat::ARGB,
            VideoFormat::xBGR,
            VideoFormat::ABGR,
        ] {
            assert!(pixel_layout(format).is_some());
        }
        assert!(pixel_layout(VideoFormat::NV12).is_none());
        assert!(pixel_layout(VideoFormat::RGB).is_none());
    }

    #[test]
    fn bgrx_becomes_rgba_with_opaque_alpha() {
        let layout = pixel_layout(VideoFormat::BGRx).unwrap();
        assert_eq!(
            repack_to_rgba(&[1, 2, 3, 0x7f], 1, 1, 4, layout).unwrap(),
            vec![3, 2, 1, 0xff]
        );
    }

    #[test]
    fn row_padding_is_dropped_and_last_row_needs_no_padding() {
        let layout = pixel_layout(VideoFormat::RGBx).unwrap();
        let mut source = Vec::new();
        for row in 0..2u8 {
            source.extend_from_slice(&[row, row, row, 0]);
            if row == 0 {
                source.extend_from_slice(&[0xee; 8]);
            }
        }
        assert_eq!(
            repack_to_rgba(&source, 1, 2, 12, layout).unwrap(),
            vec![0, 0, 0, 0xff, 1, 1, 1, 0xff]
        );
        assert!(repack_to_rgba(&source[..15], 1, 2, 12, layout).is_err());
    }

    #[test]
    fn impossible_shapes_are_rejected() {
        let layout = PixelLayout { r: 0, g: 1, b: 2 };
        assert!(repack_to_rgba(&[0; 16], 0, 1, 4, layout).is_err());
        assert!(repack_to_rgba(&[0; 16], 2, 1, 4, layout).is_err());
        assert!(repack_to_rgba(&[0; 3], 1, 1, 4, layout).is_err());
    }

    #[test]
    fn the_enum_format_pod_is_valid() {
        let bytes = enum_format_pod().unwrap();
        assert!(Pod::from_bytes(&bytes).is_some());
    }
}
