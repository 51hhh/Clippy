//! Linux PipeWire 原始视频帧的共享格式合同。
//!
//! 冻结截图与持续录屏必须复用同一套格式协商、stride 校验和 RGBA 转换；否则同一合成器在两个
//! 入口可能得到不同颜色或对行填充作出不同解释。这里只处理可直接映射的 32 位共享内存帧，明确
//! 拒绝 DMA-BUF、缩放和不支持的像素格式。

use anyhow::{anyhow, bail, ensure, Context, Result};
use memmap2::{Mmap, MmapOptions};
use pipewire as pw;
use pw::spa;
use spa::param::format::{MediaSubtype, MediaType};
use spa::param::video::{VideoFormat, VideoInfoRaw};
use spa::pod::Pod;
use std::{fs::File, os::fd::BorrowedFd, ptr::NonNull, sync::Arc};

pub struct PipeWireRgbaFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

pub fn init_pipewire() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(pw::init);
}

pub fn parse_video_format(param: &Pod) -> Result<VideoInfoRaw> {
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

pub fn frame_from_buffer(
    buffer: &mut pw::buffer::Buffer,
    info: VideoInfoRaw,
) -> Result<PipeWireRgbaFrame> {
    decode_frame(first_plane(buffer)?, info, None)
}

pub(super) struct ScreenBuffer<'a> {
    raw: NonNull<pw::sys::pw_buffer>,
    stream: &'a pw::stream::Stream,
}

impl<'a> ScreenBuffer<'a> {
    pub fn dequeue(stream: &'a pw::stream::Stream) -> Option<Self> {
        // 安全封装底层缓冲以读帧头，归还前由此对象独占，且不跨回调持有。
        let raw = NonNull::new(unsafe { stream.dequeue_raw_buffer() })?;
        Some(Self { raw, stream })
    }

    pub fn has_frame(&mut self) -> Result<bool> {
        let chunk = self.plane()?.chunk();
        Ok(chunk.size() != 0 && !chunk.flags().contains(spa::buffer::ChunkFlags::CORRUPTED))
    }

    pub fn header(&self) -> Option<spa::sys::spa_meta_header> {
        let buffer = self.buffer().ok()?;
        if buffer.n_metas == 0 || buffer.n_metas > 32 || buffer.metas.is_null() {
            return None;
        }
        let metas = unsafe { std::slice::from_raw_parts(buffer.metas, buffer.n_metas as usize) };
        let meta = metas
            .iter()
            .find(|meta| meta.type_ == spa::sys::SPA_META_Header)?;
        if (meta.size as usize) < size_of::<spa::sys::spa_meta_header>() || meta.data.is_null() {
            return None;
        }
        Some(unsafe { std::ptr::read_unaligned(meta.data.cast::<spa::sys::spa_meta_header>()) })
    }

    pub fn region(&mut self, info: VideoInfoRaw, bounds: [u32; 4]) -> Result<PipeWireRgbaFrame> {
        decode_frame(self.plane()?, info, Some(bounds))
    }

    fn buffer(&self) -> Result<&spa::sys::spa_buffer> {
        let raw = unsafe { self.raw.as_ref().buffer.as_ref() };
        raw.context("ScreenCast buffer has no SPA data")
    }

    fn plane(&mut self) -> Result<&mut spa::buffer::Data> {
        let buffer = self.buffer()?;
        ensure!(buffer.n_datas > 0, "ScreenCast buffer has no video plane");
        let data = unsafe { buffer.datas.cast::<spa::buffer::Data>().as_mut() };
        data.context("ScreenCast buffer has no video data")
    }
}

impl Drop for ScreenBuffer<'_> {
    fn drop(&mut self) {
        unsafe { self.stream.queue_raw_buffer(self.raw.as_ptr()) };
    }
}

fn first_plane<'a>(buffer: &'a mut pw::buffer::Buffer<'_>) -> Result<&'a mut spa::buffer::Data> {
    buffer
        .datas_mut()
        .first_mut()
        .context("PipeWire 缓冲里没有数据块")
}

fn decode_frame(
    data: &mut spa::buffer::Data,
    info: VideoInfoRaw,
    region: Option<[u32; 4]>,
) -> Result<PipeWireRgbaFrame> {
    let size = info.size();
    let layout = pixel_layout(info.format()).ok_or_else(|| {
        anyhow!(
            "PipeWire 协商出的像素格式 {:?} 不是 32 位 RGB 排列",
            info.format()
        )
    })?;
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
    let chunk = data.chunk();
    let offset = chunk.offset() as usize;
    let length = chunk.size() as usize;
    let mapped = if region.is_some() && kind == spa::buffer::DataType::MemFd {
        let fd = data.fd();
        ensure!(fd >= 0, "ScreenCast did not provide a shared-memory fd");
        let raw = data.as_raw();
        // 部分 wlr 后端不设置 READABLE，PipeWire 的自动映射会生成 PROT_NONE。
        // fd 由当前尚未归还的缓冲持有，主动只读映射授权流实际提供的共享内存。
        let fd = unsafe { BorrowedFd::borrow_raw(fd) };
        Some(map_readonly(fd, raw.mapoffset, raw.maxsize)?)
    } else {
        None
    };
    let pixels: &[u8] = match &mapped {
        Some(map) => map,
        None => data.data().context("PipeWire 缓冲没有映射到内存")?,
    };
    let pixels = pixels
        .get(offset..)
        .with_context(|| format!("PipeWire 帧的 offset {offset} 越过了缓冲末尾"))?;
    let (pixels, bounds) = match region {
        Some(bounds) => (
            pixels
                .get(..length)
                .context("PipeWire payload exceeds its mapped buffer")?,
            bounds,
        ),
        None => (pixels, [0, 0, size.width, size.height]),
    };
    let rgba = repack_region_to_rgba(
        pixels,
        (size.width, size.height),
        stride as usize,
        layout,
        bounds,
    )?;
    Ok(PipeWireRgbaFrame {
        width: bounds[2] - bounds[0],
        height: bounds[3] - bounds[1],
        rgba: Arc::from(rgba),
    })
}

fn map_readonly(fd: BorrowedFd<'_>, offset: u32, length: u32) -> Result<Mmap> {
    let file = File::from(fd.try_clone_to_owned()?);
    let end = u64::from(offset) + u64::from(length);
    ensure!(
        length > 0 && end <= file.metadata()?.len(),
        "ScreenCast buffer extends beyond its shared-memory file"
    );
    // PipeWire 保证缓冲归还前，来源进程不会回收或改写这块数据。
    unsafe {
        MmapOptions::new()
            .offset(u64::from(offset))
            .len(length as usize)
            .map(&file)
    }
    .context("Mapping ScreenCast shared memory")
}

/// 报给合成器的 `EnumFormat`。不带 modifier，强制协商为能直接读取的共享内存。
pub fn enum_format_pod() -> Result<Vec<u8>> {
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

pub(super) fn request_frame_header(stream: &pw::stream::Stream) -> Result<()> {
    use spa::pod::{Object, Property, Value};
    let object = Object {
        type_: spa::utils::SpaTypes::ObjectParamMeta.as_raw(),
        id: spa::param::ParamType::Meta.as_raw(),
        properties: vec![
            Property::new(
                spa::sys::SPA_PARAM_META_type,
                Value::Id(spa::utils::Id(spa::sys::SPA_META_Header)),
            ),
            Property::new(
                spa::sys::SPA_PARAM_META_size,
                Value::Int(i32::try_from(size_of::<spa::sys::spa_meta_header>())?),
            ),
        ],
    };
    let (bytes, _) = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &Value::Object(object),
    )?;
    let header = Pod::from_bytes(bytes.get_ref()).context("Invalid ScreenCast header parameter")?;
    stream
        .update_params(&mut [header])
        .context("Requesting ScreenCast frame headers")?;
    Ok(())
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
fn repack_region_to_rgba(
    src: &[u8],
    (width, height): (u32, u32),
    stride: usize,
    layout: PixelLayout,
    bounds: [u32; 4],
) -> Result<Vec<u8>> {
    let [left, top, right, bottom] = bounds.map(|value| value as usize);
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

    let cropped_row = (right - left) * 4;
    let mut out = vec![0u8; cropped_row * (bottom - top)];
    for (y, line) in out.chunks_exact_mut(cropped_row).enumerate() {
        let start = (y + top) * stride + left * 4;
        let source = &src[start..start + cropped_row];
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
    use std::{io::Write, os::fd::AsFd};

    #[test]
    fn portal_memfd_uses_its_declared_window_and_rejects_truncation() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"skipRGBA").unwrap();
        assert_eq!(&*map_readonly(file.as_fd(), 4, 4).unwrap(), b"RGBA");
        assert!(map_readonly(file.as_fd(), 4, 5).is_err());
    }

    fn repack_to_rgba(
        src: &[u8],
        width: u32,
        height: u32,
        stride: usize,
        layout: PixelLayout,
    ) -> Result<Vec<u8>> {
        repack_region_to_rgba(src, (width, height), stride, layout, [0, 0, width, height])
    }

    #[test]
    fn cropped_screen_frame_reads_only_the_selected_rows_and_columns() {
        let layout = pixel_layout(VideoFormat::BGRA).unwrap();
        let mut source = Vec::new();
        for y in 0..4 {
            for x in 0..5 {
                source.extend_from_slice(&[x, 100, y, 0]);
            }
            source.extend_from_slice(&[0xee; 8]);
        }
        let rgba = repack_region_to_rgba(&source, (5, 4), 28, layout, [2, 1, 4, 3]).unwrap();
        assert_eq!(
            rgba,
            [1, 100, 2, 255, 1, 100, 3, 255, 2, 100, 2, 255, 2, 100, 3, 255]
        );
        assert!(repack_region_to_rgba(&source[..103], (5, 4), 28, layout, [2, 1, 4, 3]).is_err());
    }

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
