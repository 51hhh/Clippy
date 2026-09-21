//! Clippy 生产 WebM 分段的受限无损 remux。
//!
//! 这里不是通用 Matroska 导入器。输入已经由恢复清单约束，并且必须恰好是 Clippy writer 生成的
//! 单 VP9 视频轨、无 lacing `SimpleBlock` 子集。读取器逐 packet 工作，避免把最长 16 GiB 分段载入
//! 内存；输出复用同一 libwebm muxer 重建 cluster、seek 与 duration 元数据。

use super::vp9_webm::{Vp9WebmError, Vp9WebmOutput, WebmPacketMux};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

const EBML: u32 = 0x1A45_DFA3;
const SEGMENT: u32 = 0x1853_8067;
const SEEK_HEAD: u32 = 0x114D_9B74;
const INFO: u32 = 0x1549_A966;
const TIMECODE_SCALE: u32 = 0x002A_D7B1;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0x0000_00AE;
const TRACK_NUMBER: u32 = 0x0000_00D7;
const TRACK_TYPE: u32 = 0x0000_0083;
const CODEC_ID: u32 = 0x0000_0086;
const VIDEO: u32 = 0x0000_00E0;
const PIXEL_WIDTH: u32 = 0x0000_00B0;
const PIXEL_HEIGHT: u32 = 0x0000_00BA;
const CLUSTER: u32 = 0x1F43_B675;
const CLUSTER_TIMECODE: u32 = 0x0000_00E7;
const SIMPLE_BLOCK: u32 = 0x0000_00A3;
const BLOCK_GROUP: u32 = 0x0000_00A0;
const CLUSTER_POSITION: u32 = 0x0000_00A7;
const PREVIOUS_CLUSTER_SIZE: u32 = 0x0000_00AB;
const CUES: u32 = 0x1C53_BB6B;
const VOID: u32 = 0x0000_00EC;

const WEBM_TIMECODE_SCALE_NS: u64 = 1_000_000;
const MAX_PACKET_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TRACK_METADATA_BYTES: u64 = 1024;
const HASH_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub(in crate::recording) struct WebmRemuxSource {
    pub path: PathBuf,
    pub byte_length: u64,
    pub sha256: String,
    pub started_at_ns: u64,
    pub duration_ns: u64,
    pub frame_count: u64,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::recording) struct WebmRemuxSpec {
    pub width: u32,
    pub height: u32,
    pub fps_numerator: u32,
    pub fps_denominator: u32,
}

#[derive(Debug)]
pub(in crate::recording) struct WebmRemuxOutput<W> {
    pub writer: W,
    pub frame_count: u64,
    pub duration_ns: u64,
}

#[derive(Debug, Clone, Copy)]
struct ElementHeader {
    id: u32,
    data_start: u64,
    data_end: u64,
}

#[derive(Debug, Default)]
struct ParsedTrack {
    number: Option<u64>,
    track_type: Option<u64>,
    codec_id: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

struct StrictWebmReader {
    reader: BufReader<File>,
    file_len: u64,
}

pub(in crate::recording) fn remux_vp9_segments<W: Write + Seek>(
    sources: &[WebmRemuxSource],
    output: W,
    spec: WebmRemuxSpec,
) -> Result<WebmRemuxOutput<W>, String> {
    if sources.is_empty()
        || spec.width == 0
        || spec.height == 0
        || spec.fps_numerator == 0
        || spec.fps_denominator == 0
    {
        return Err("录屏恢复合并参数无效".to_string());
    }

    let mut expected_start = 0_u64;
    let mut expected_frames = 0_u64;
    for source in sources {
        if source.started_at_ns != expected_start || source.duration_ns == 0 {
            return Err("录屏恢复分段时间线不连续".to_string());
        }
        expected_start = expected_start
            .checked_add(source.duration_ns)
            .ok_or_else(|| "录屏恢复分段时间线溢出".to_string())?;
        expected_frames = expected_frames
            .checked_add(source.frame_count)
            .ok_or_else(|| "录屏恢复分段帧数溢出".to_string())?;
    }

    let mut mux = WebmPacketMux::new(output, spec.width, spec.height).map_err(remux_mux_error)?;
    let mut written_frames = 0_u64;
    for source in sources {
        let mut input = StrictWebmReader::open_verified(source)?;
        let parsed = input.read_packets(spec, source, |packet, local_timestamp_ns, keyframe| {
            let global_timestamp_ns = source
                .started_at_ns
                .checked_add(local_timestamp_ns)
                .ok_or_else(|| "录屏恢复 packet 时间戳溢出".to_string())?;
            mux.add_frame(packet, global_timestamp_ns, keyframe)
                .map_err(remux_mux_error)
        })?;
        written_frames = written_frames
            .checked_add(parsed)
            .ok_or_else(|| "录屏恢复输出帧数溢出".to_string())?;
    }
    if written_frames != expected_frames {
        return Err("录屏恢复输出帧数与清单不一致".to_string());
    }
    let Vp9WebmOutput {
        writer,
        frame_count,
    } = mux.finish(expected_start).map_err(remux_mux_error)?;
    if frame_count != expected_frames {
        return Err("录屏恢复 mux 帧数与清单不一致".to_string());
    }
    Ok(WebmRemuxOutput {
        writer,
        frame_count,
        duration_ns: expected_start,
    })
}

fn remux_mux_error(error: Vp9WebmError) -> String {
    format!("录屏恢复 WebM 封装失败: {error}")
}

impl StrictWebmReader {
    fn open_verified(source: &WebmRemuxSource) -> Result<Self, String> {
        let path_metadata = fs::symlink_metadata(&source.path)
            .map_err(|error| format!("读取录屏恢复分段失败: {error}"))?;
        if path_metadata.file_type().is_symlink() || !path_metadata.is_file() {
            return Err("录屏恢复分段不是普通文件".to_string());
        }
        if path_metadata.len() != source.byte_length {
            return Err("录屏恢复分段大小与清单不一致".to_string());
        }
        let mut file =
            File::open(&source.path).map_err(|error| format!("打开录屏恢复分段失败: {error}"))?;
        let opened_metadata = file
            .metadata()
            .map_err(|error| format!("读取已打开录屏恢复分段失败: {error}"))?;
        if !opened_metadata.is_file() || opened_metadata.len() != source.byte_length {
            return Err("已打开录屏恢复分段与清单不一致".to_string());
        }
        let mut hasher = Sha256::new();
        let mut observed = 0_u64;
        let mut buffer = [0_u8; HASH_BUFFER_BYTES];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|error| format!("校验录屏恢复分段失败: {error}"))?;
            if read == 0 {
                break;
            }
            observed = observed
                .checked_add(read as u64)
                .ok_or_else(|| "录屏恢复分段大小溢出".to_string())?;
            if observed > source.byte_length {
                return Err("录屏恢复分段大小与清单不一致".to_string());
            }
            hasher.update(&buffer[..read]);
        }
        if observed != source.byte_length || format!("{:x}", hasher.finalize()) != source.sha256 {
            return Err("录屏恢复分段与清单哈希不一致".to_string());
        }
        file.seek(SeekFrom::Start(0))
            .map_err(|error| format!("重置录屏恢复分段失败: {error}"))?;
        Ok(Self {
            reader: BufReader::new(file),
            file_len: source.byte_length,
        })
    }

    fn read_packets<F>(
        &mut self,
        spec: WebmRemuxSpec,
        source: &WebmRemuxSource,
        mut on_packet: F,
    ) -> Result<u64, String>
    where
        F: FnMut(&[u8], u64, bool) -> Result<(), String>,
    {
        let ebml = self.read_header(self.file_len, false)?;
        if ebml.id != EBML {
            return Err("录屏恢复分段缺少 EBML 头".to_string());
        }
        self.seek_to(ebml.data_end)?;

        let segment = loop {
            if self.position()? >= self.file_len {
                return Err("录屏恢复分段缺少 Segment".to_string());
            }
            let header = self.read_header(self.file_len, true)?;
            if header.id == SEGMENT {
                break header;
            }
            if header.id != VOID {
                return Err("录屏恢复分段根元素无效".to_string());
            }
            self.seek_to(header.data_end)?;
        };
        if segment.data_end != self.file_len {
            return Err("录屏恢复分段包含 Segment 外数据".to_string());
        }

        let mut timecode_scale = WEBM_TIMECODE_SCALE_NS;
        let mut info_seen = false;
        let mut track: Option<ParsedTrack> = None;
        let mut frame_index = 0_u64;
        let mut cluster_seen = false;
        while self.position()? < segment.data_end {
            let header = self.read_header(segment.data_end, false)?;
            match header.id {
                INFO => {
                    if info_seen || cluster_seen {
                        return Err("录屏恢复分段包含重复 Info".to_string());
                    }
                    timecode_scale = self.read_info(header.data_end)?;
                    info_seen = true;
                }
                TRACKS => {
                    if track.is_some() || cluster_seen {
                        return Err("录屏恢复分段包含重复 Tracks".to_string());
                    }
                    track = Some(self.read_tracks(header.data_end)?);
                }
                CLUSTER => {
                    cluster_seen = true;
                    let track = track
                        .as_ref()
                        .ok_or_else(|| "录屏恢复分段在轨道之前出现画面".to_string())?;
                    self.validate_track(track, spec)?;
                    frame_index = self.read_cluster(
                        header.data_end,
                        timecode_scale,
                        track.number.expect("轨道已校验"),
                        spec,
                        source,
                        frame_index,
                        &mut on_packet,
                    )?;
                }
                SEEK_HEAD | CUES | VOID => self.seek_to(header.data_end)?,
                _ => return Err("录屏恢复分段包含非生产顶层元素".to_string()),
            }
        }
        let track = track.ok_or_else(|| "录屏恢复分段缺少视频轨".to_string())?;
        self.validate_track(&track, spec)?;
        if !info_seen || !cluster_seen {
            return Err("录屏恢复分段缺少生产容器元数据或画面".to_string());
        }
        if timecode_scale != WEBM_TIMECODE_SCALE_NS {
            return Err("录屏恢复分段时间基不受支持".to_string());
        }
        if frame_index != source.frame_count {
            return Err("录屏恢复分段帧数与清单不一致".to_string());
        }
        Ok(frame_index)
    }

    fn read_info(&mut self, end: u64) -> Result<u64, String> {
        let mut scale = WEBM_TIMECODE_SCALE_NS;
        let mut scale_seen = false;
        while self.position()? < end {
            let header = self.read_header(end, false)?;
            if header.id == TIMECODE_SCALE {
                if scale_seen {
                    return Err("录屏恢复分段包含重复时间基".to_string());
                }
                scale = self.read_uint(header)?;
                scale_seen = true;
            } else {
                self.seek_to(header.data_end)?;
            }
        }
        Ok(scale)
    }

    fn read_tracks(&mut self, end: u64) -> Result<ParsedTrack, String> {
        let mut track = None;
        while self.position()? < end {
            let header = self.read_header(end, false)?;
            if header.id == TRACK_ENTRY {
                if track.is_some() {
                    return Err("录屏恢复分段不是单轨视频".to_string());
                }
                track = Some(self.read_track_entry(header.data_end)?);
            } else {
                self.seek_to(header.data_end)?;
            }
        }
        track.ok_or_else(|| "录屏恢复分段缺少 TrackEntry".to_string())
    }

    fn read_track_entry(&mut self, end: u64) -> Result<ParsedTrack, String> {
        let mut track = ParsedTrack::default();
        while self.position()? < end {
            let header = self.read_header(end, false)?;
            match header.id {
                TRACK_NUMBER => track.number = Some(self.read_uint(header)?),
                TRACK_TYPE => track.track_type = Some(self.read_uint(header)?),
                CODEC_ID => track.codec_id = Some(self.read_string(header)?),
                VIDEO => self.read_video(header.data_end, &mut track)?,
                _ => self.seek_to(header.data_end)?,
            }
        }
        Ok(track)
    }

    fn read_video(&mut self, end: u64, track: &mut ParsedTrack) -> Result<(), String> {
        while self.position()? < end {
            let header = self.read_header(end, false)?;
            match header.id {
                PIXEL_WIDTH => {
                    track.width = Some(
                        u32::try_from(self.read_uint(header)?)
                            .map_err(|_| "录屏恢复视频宽度溢出".to_string())?,
                    );
                }
                PIXEL_HEIGHT => {
                    track.height = Some(
                        u32::try_from(self.read_uint(header)?)
                            .map_err(|_| "录屏恢复视频高度溢出".to_string())?,
                    );
                }
                _ => self.seek_to(header.data_end)?,
            }
        }
        Ok(())
    }

    fn validate_track(&self, track: &ParsedTrack, spec: WebmRemuxSpec) -> Result<(), String> {
        if track.number != Some(1)
            || track.track_type != Some(1)
            || track.codec_id.as_deref() != Some("V_VP9")
            || track.width != Some(spec.width)
            || track.height != Some(spec.height)
        {
            return Err("录屏恢复视频轨与清单不一致".to_string());
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn read_cluster<F>(
        &mut self,
        end: u64,
        timecode_scale: u64,
        track_number: u64,
        spec: WebmRemuxSpec,
        source: &WebmRemuxSource,
        mut frame_index: u64,
        on_packet: &mut F,
    ) -> Result<u64, String>
    where
        F: FnMut(&[u8], u64, bool) -> Result<(), String>,
    {
        let mut cluster_timecode = None;
        while self.position()? < end {
            let header = self.read_header(end, false)?;
            match header.id {
                CLUSTER_TIMECODE => {
                    if cluster_timecode.is_some() {
                        return Err("录屏恢复 cluster 包含重复时间戳".to_string());
                    }
                    cluster_timecode = Some(self.read_uint(header)?);
                }
                SIMPLE_BLOCK => {
                    let cluster_timecode = cluster_timecode
                        .ok_or_else(|| "录屏恢复 block 缺少 cluster 时间戳".to_string())?;
                    let body = self.read_bounded_body(header, MAX_PACKET_BYTES + 16)?;
                    let parsed = parse_simple_block(&body, cluster_timecode, timecode_scale)?;
                    if parsed.track_number != track_number {
                        return Err("录屏恢复 block 引用了未知轨道".to_string());
                    }
                    if frame_index == 0 && !parsed.keyframe {
                        return Err("录屏恢复分段没有从关键帧开始".to_string());
                    }
                    validate_frame_timestamp(parsed.timestamp_ns, frame_index, spec, source)?;
                    on_packet(
                        &body[parsed.payload_offset..],
                        parsed.timestamp_ns,
                        parsed.keyframe,
                    )?;
                    frame_index = frame_index
                        .checked_add(1)
                        .ok_or_else(|| "录屏恢复分段帧数溢出".to_string())?;
                    if frame_index > source.frame_count {
                        return Err("录屏恢复分段帧数超过清单".to_string());
                    }
                }
                BLOCK_GROUP => return Err("录屏恢复分段包含不受支持的 BlockGroup".to_string()),
                CLUSTER_POSITION | PREVIOUS_CLUSTER_SIZE | VOID => self.seek_to(header.data_end)?,
                _ => return Err("录屏恢复 cluster 包含非生产元素".to_string()),
            }
        }
        Ok(frame_index)
    }

    fn read_header(
        &mut self,
        parent_end: u64,
        allow_unknown_segment: bool,
    ) -> Result<ElementHeader, String> {
        let start = self.position()?;
        if start >= parent_end {
            return Err("录屏恢复 EBML 元素越过父边界".to_string());
        }
        let (id, _) = self.read_vint(true, 4)?;
        let (size, unknown) = self.read_vint(false, 8)?;
        let data_start = self.position()?;
        let data_end = if unknown {
            if allow_unknown_segment && id == u64::from(SEGMENT) {
                parent_end
            } else {
                return Err("录屏恢复 EBML 未知长度元素不受支持".to_string());
            }
        } else {
            data_start
                .checked_add(size)
                .filter(|end| *end <= parent_end)
                .ok_or_else(|| "录屏恢复 EBML 元素长度越界".to_string())?
        };
        Ok(ElementHeader {
            id: u32::try_from(id).map_err(|_| "录屏恢复 EBML ID 溢出".to_string())?,
            data_start,
            data_end,
        })
    }

    fn read_vint(&mut self, keep_marker: bool, max_bytes: u32) -> Result<(u64, bool), String> {
        let mut first = [0_u8; 1];
        self.reader
            .read_exact(&mut first)
            .map_err(|_| "录屏恢复 EBML VINT 截断".to_string())?;
        if first[0] == 0 {
            return Err("录屏恢复 EBML VINT 无效".to_string());
        }
        let length = first[0].leading_zeros() + 1;
        if length > max_bytes {
            return Err("录屏恢复 EBML VINT 过长".to_string());
        }
        let marker = 1_u8 << (8 - length);
        let mut value = if keep_marker {
            u64::from(first[0])
        } else {
            u64::from(first[0] & !marker)
        };
        for _ in 1..length {
            let mut byte = [0_u8; 1];
            self.reader
                .read_exact(&mut byte)
                .map_err(|_| "录屏恢复 EBML VINT 截断".to_string())?;
            value = value
                .checked_shl(8)
                .map(|shifted| shifted | u64::from(byte[0]))
                .ok_or_else(|| "录屏恢复 EBML VINT 溢出".to_string())?;
        }
        let unknown = !keep_marker && value == ((1_u64 << (7 * length)) - 1);
        Ok((value, unknown))
    }

    fn read_uint(&mut self, header: ElementHeader) -> Result<u64, String> {
        let length = header.data_end - header.data_start;
        if length == 0 || length > 8 {
            return Err("录屏恢复 EBML 整数长度无效".to_string());
        }
        let mut bytes = [0_u8; 8];
        let offset = 8 - length as usize;
        self.reader
            .read_exact(&mut bytes[offset..])
            .map_err(|error| format!("读取录屏恢复 EBML 整数失败: {error}"))?;
        Ok(u64::from_be_bytes(bytes))
    }

    fn read_string(&mut self, header: ElementHeader) -> Result<String, String> {
        let body = self.read_bounded_body(header, MAX_TRACK_METADATA_BYTES)?;
        String::from_utf8(body).map_err(|_| "录屏恢复轨道字符串不是 UTF-8".to_string())
    }

    fn read_bounded_body(&mut self, header: ElementHeader, limit: u64) -> Result<Vec<u8>, String> {
        let length = header.data_end - header.data_start;
        if length == 0 || length > limit {
            return Err("录屏恢复 EBML 数据长度超限".to_string());
        }
        let length =
            usize::try_from(length).map_err(|_| "录屏恢复 EBML 数据长度溢出".to_string())?;
        let mut body = vec![0_u8; length];
        self.reader
            .read_exact(&mut body)
            .map_err(|error| format!("读取录屏恢复 EBML 数据失败: {error}"))?;
        Ok(body)
    }

    fn position(&mut self) -> Result<u64, String> {
        self.reader
            .stream_position()
            .map_err(|error| format!("读取录屏恢复位置失败: {error}"))
    }

    fn seek_to(&mut self, position: u64) -> Result<(), String> {
        if position > self.file_len {
            return Err("录屏恢复 seek 越过文件边界".to_string());
        }
        self.reader
            .seek(SeekFrom::Start(position))
            .map_err(|error| format!("跳过录屏恢复元数据失败: {error}"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct ParsedSimpleBlock {
    track_number: u64,
    timestamp_ns: u64,
    keyframe: bool,
    payload_offset: usize,
}

fn parse_simple_block(
    body: &[u8],
    cluster_timecode: u64,
    timecode_scale: u64,
) -> Result<ParsedSimpleBlock, String> {
    let Some(&first) = body.first() else {
        return Err("录屏恢复 SimpleBlock 为空".to_string());
    };
    if first == 0 {
        return Err("录屏恢复 SimpleBlock 轨道 VINT 无效".to_string());
    }
    let length = (first.leading_zeros() + 1) as usize;
    if length > 8 || body.len() <= length + 3 {
        return Err("录屏恢复 SimpleBlock 截断".to_string());
    }
    let marker = 1_u8 << (8 - length);
    let mut track_number = u64::from(first & !marker);
    for byte in &body[1..length] {
        track_number = track_number
            .checked_shl(8)
            .map(|shifted| shifted | u64::from(*byte))
            .ok_or_else(|| "录屏恢复 SimpleBlock 轨道溢出".to_string())?;
    }
    if track_number == 0 {
        return Err("录屏恢复 SimpleBlock 轨道无效".to_string());
    }
    let relative = i16::from_be_bytes([body[length], body[length + 1]]);
    let flags = body[length + 2];
    if flags & 0x06 != 0 {
        return Err("录屏恢复不接受 lacing block".to_string());
    }
    let ticks = i128::from(cluster_timecode) + i128::from(relative);
    if ticks < 0 {
        return Err("录屏恢复 block 时间戳为负".to_string());
    }
    let timestamp_ns = u128::try_from(ticks)
        .ok()
        .and_then(|ticks| ticks.checked_mul(u128::from(timecode_scale)))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| "录屏恢复 block 时间戳溢出".to_string())?;
    Ok(ParsedSimpleBlock {
        track_number,
        timestamp_ns,
        keyframe: flags & 0x80 != 0,
        payload_offset: length + 3,
    })
}

fn validate_frame_timestamp(
    timestamp_ns: u64,
    frame_index: u64,
    spec: WebmRemuxSpec,
    source: &WebmRemuxSource,
) -> Result<(), String> {
    let expected_frames = u128::from(source.duration_ns)
        .checked_mul(u128::from(spec.fps_numerator))
        .and_then(|value| {
            value.checked_add(u128::from(1_000_000_000_u64) * u128::from(spec.fps_denominator) - 1)
        })
        .and_then(|value| {
            value.checked_div(u128::from(1_000_000_000_u64) * u128::from(spec.fps_denominator))
        })
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| "录屏恢复预期帧数溢出".to_string())?;
    if expected_frames != source.frame_count || timestamp_ns >= source.duration_ns {
        return Err("录屏恢复分段时长或帧数与固定帧率不一致".to_string());
    }
    let expected_timestamp = u128::from(frame_index)
        .checked_mul(u128::from(1_000_000_000_u64))
        .and_then(|value| value.checked_mul(u128::from(spec.fps_denominator)))
        .and_then(|value| value.checked_div(u128::from(spec.fps_numerator)))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| "录屏恢复预期时间戳溢出".to_string())?;
    if timestamp_ns.abs_diff(expected_timestamp) >= WEBM_TIMECODE_SCALE_NS {
        return Err("录屏恢复分段时间戳与固定帧率不一致".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::mux::vp9_webm::Vp9WebmWriter;
    use std::io::Cursor;
    use std::process::Command;

    fn solid_rgba(width: u32, height: u32, color: [u8; 3]) -> Vec<u8> {
        (0..width * height)
            .flat_map(|_| [color[0], color[1], color[2], 255])
            .collect()
    }

    fn write_segment(path: &std::path::Path, color: [u8; 3]) -> WebmRemuxSource {
        let mut writer = Vp9WebmWriter::new(Cursor::new(Vec::new()), 64, 48, 10, 1).unwrap();
        writer.push_rgba(&solid_rgba(64, 48, color), 0).unwrap();
        writer
            .push_rgba(
                &solid_rgba(64, 48, [color[2], color[0], color[1]]),
                100_000_000,
            )
            .unwrap();
        let output = writer.finish_with_stats(200_000_000).unwrap();
        let bytes = output.writer.into_inner();
        fs::write(path, &bytes).unwrap();
        WebmRemuxSource {
            path: path.to_path_buf(),
            byte_length: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            started_at_ns: 0,
            duration_ns: 200_000_000,
            frame_count: 2,
        }
    }

    fn packets(source: &WebmRemuxSource) -> Vec<Vec<u8>> {
        let mut reader = StrictWebmReader::open_verified(source).unwrap();
        let mut packets = Vec::new();
        reader
            .read_packets(
                WebmRemuxSpec {
                    width: 64,
                    height: 48,
                    fps_numerator: 10,
                    fps_denominator: 1,
                },
                source,
                |packet, _, _| {
                    packets.push(packet.to_vec());
                    Ok(())
                },
            )
            .unwrap();
        packets
    }

    #[test]
    fn remuxes_two_production_segments_without_reencoding() {
        let directory = tempfile::tempdir().unwrap();
        let first_path = directory.path().join("first.webm");
        let second_path = directory.path().join("second.webm");
        let first = write_segment(&first_path, [16, 32, 64]);
        let single = remux_vp9_segments(
            std::slice::from_ref(&first),
            Cursor::new(Vec::new()),
            WebmRemuxSpec {
                width: 64,
                height: 48,
                fps_numerator: 10,
                fps_denominator: 1,
            },
        )
        .unwrap();
        assert_eq!(single.frame_count, 2);
        assert_eq!(single.duration_ns, 200_000_000);

        let mut second = write_segment(&second_path, [200, 180, 160]);
        second.started_at_ns = 200_000_000;
        let expected_packets = packets(&first)
            .into_iter()
            .chain(packets(&second))
            .collect::<Vec<_>>();
        let sources = vec![first, second];
        let output = remux_vp9_segments(
            &sources,
            Cursor::new(Vec::new()),
            WebmRemuxSpec {
                width: 64,
                height: 48,
                fps_numerator: 10,
                fps_denominator: 1,
            },
        )
        .unwrap();
        assert_eq!(output.frame_count, 4);
        assert_eq!(output.duration_ns, 400_000_000);

        let output_path = directory.path().join("recovered.webm");
        let output_bytes = output.writer.into_inner();
        fs::write(&output_path, &output_bytes).unwrap();
        let recovered = WebmRemuxSource {
            path: output_path.clone(),
            byte_length: output_bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&output_bytes)),
            started_at_ns: 0,
            duration_ns: 400_000_000,
            frame_count: 4,
        };
        assert_eq!(packets(&recovered), expected_packets);
        if Command::new("ffprobe").arg("-version").output().is_ok() {
            let probe = Command::new("ffprobe")
                .args([
                    "-v",
                    "error",
                    "-count_frames",
                    "-select_streams",
                    "v:0",
                    "-show_entries",
                    "stream=codec_name,nb_read_frames:format=duration",
                    "-of",
                    "json",
                ])
                .arg(output_path)
                .output()
                .unwrap();
            assert!(
                probe.status.success(),
                "{}",
                String::from_utf8_lossy(&probe.stderr)
            );
            let payload: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
            assert_eq!(payload["streams"][0]["codec_name"], "vp9");
            assert_eq!(payload["streams"][0]["nb_read_frames"], "4");
            assert_eq!(payload["format"]["duration"], "0.400000");
        }
    }

    #[test]
    fn rejects_hash_timeline_geometry_and_frame_count_mismatches() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("segment.webm");
        let source = write_segment(&path, [16, 32, 64]);
        let spec = WebmRemuxSpec {
            width: 64,
            height: 48,
            fps_numerator: 10,
            fps_denominator: 1,
        };
        let mut bad = source.clone();
        bad.sha256 = "0".repeat(64);
        assert!(remux_vp9_segments(&[bad], Cursor::new(Vec::new()), spec).is_err());

        let mut bad = source.clone();
        bad.started_at_ns = 1;
        assert!(remux_vp9_segments(&[bad], Cursor::new(Vec::new()), spec).is_err());

        assert!(remux_vp9_segments(
            std::slice::from_ref(&source),
            Cursor::new(Vec::new()),
            WebmRemuxSpec { width: 66, ..spec },
        )
        .is_err());

        let mut bad = source;
        bad.frame_count = 3;
        assert!(remux_vp9_segments(&[bad], Cursor::new(Vec::new()), spec).is_err());
    }

    #[test]
    fn rejects_lacing_truncated_and_negative_simple_blocks() {
        assert!(parse_simple_block(&[0x81, 0, 0, 0x02, 1], 0, WEBM_TIMECODE_SCALE_NS).is_err());
        assert!(parse_simple_block(&[0x81, 0, 0], 0, WEBM_TIMECODE_SCALE_NS).is_err());
        assert!(
            parse_simple_block(&[0x81, 0xff, 0xff, 0x80, 1], 0, WEBM_TIMECODE_SCALE_NS,).is_err()
        );
    }
}
