//! Clippy 生产 WebM 分段的受限无损 remux。
//!
//! 这里不是通用 Matroska 导入器。输入已经由恢复清单约束，并且必须恰好是 Clippy writer 生成的
//! 单 VP9 视频轨、无 lacing `SimpleBlock` 子集。schema v2 恢复另接受固定的 VP9 + Opus 双轨
//! 形状：视频 packet 原样重封装，每段 Opus 用独立 decoder 去掉 pre-skip/尾 padding 后，交给一个
//! 连续 encoder。读取器逐 packet 工作，避免把最长 16 GiB 分段载入内存；输出复用现有 muxer 重建
//! cluster、seek 与 duration 元数据。

#[cfg(feature = "recording-opus-webm")]
use super::super::audio::{AUDIO_SAMPLE_RATE_HZ, DEFAULT_OPUS_FRAME_MS};
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
const CODEC_PRIVATE: u32 = 0x0000_63A2;
const CODEC_DELAY: u32 = 0x0000_56AA;
const SEEK_PRE_ROLL: u32 = 0x0000_56BB;
const VIDEO: u32 = 0x0000_00E0;
const PIXEL_WIDTH: u32 = 0x0000_00B0;
const PIXEL_HEIGHT: u32 = 0x0000_00BA;
const AUDIO: u32 = 0x0000_00E1;
const SAMPLING_FREQUENCY: u32 = 0x0000_00B5;
const CHANNELS: u32 = 0x0000_009F;
const CLUSTER: u32 = 0x1F43_B675;
const CLUSTER_TIMECODE: u32 = 0x0000_00E7;
const SIMPLE_BLOCK: u32 = 0x0000_00A3;
const BLOCK_GROUP: u32 = 0x0000_00A0;
const BLOCK: u32 = 0x0000_00A1;
const DISCARD_PADDING: u32 = 0x0000_75A2;
const CLUSTER_POSITION: u32 = 0x0000_00A7;
const PREVIOUS_CLUSTER_SIZE: u32 = 0x0000_00AB;
const CUES: u32 = 0x1C53_BB6B;
const VOID: u32 = 0x0000_00EC;

const WEBM_TIMECODE_SCALE_NS: u64 = 1_000_000;
const AV_WEBM_TIMECODE_SCALE_NS: u64 = 500_000;
const MAX_PACKET_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TRACK_METADATA_BYTES: u64 = 1024;
const HASH_BUFFER_BYTES: usize = 64 * 1024;
#[cfg(feature = "recording-opus-webm")]
pub(in crate::recording) const OPUS_PACKET_FRAMES: usize =
    AUDIO_SAMPLE_RATE_HZ as usize * DEFAULT_OPUS_FRAME_MS as usize / 1_000;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::recording) struct WebmThumbnailAudioSpec {
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub pre_skip_frames: u16,
    pub codec_delay_ns: u64,
    pub seek_pre_roll_ns: u64,
    pub packet_count: u64,
    pub pcm_frame_count: u64,
}

#[cfg(feature = "recording-opus-webm")]
#[derive(Debug, Clone)]
pub(in crate::recording) struct WebmAvRemuxSource {
    pub source: WebmRemuxSource,
    pub audio: WebmThumbnailAudioSpec,
}

#[derive(Debug)]
pub(in crate::recording) struct WebmRemuxOutput<W> {
    pub writer: W,
    pub frame_count: u64,
    pub duration_ns: u64,
}

#[cfg(feature = "recording-opus-webm")]
pub(in crate::recording) enum ParsedAvPacket<'a> {
    Video {
        data: &'a [u8],
        timestamp_ns: u64,
        keyframe: bool,
    },
    Audio {
        data: &'a [u8],
        timestamp_ns: u64,
        discard_padding_ns: u64,
    },
}

#[cfg(feature = "recording-opus-webm")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::recording) struct ParsedAvStats {
    pub video_frame_count: u64,
    pub audio_packet_count: u64,
}

#[derive(Debug, Clone, Copy)]
struct ElementHeader {
    id: u32,
    data_start: u64,
    data_end: u64,
}

#[derive(Debug, Clone, Default)]
struct ParsedTrack {
    number: Option<u64>,
    track_type: Option<u64>,
    codec_id: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    codec_private: Option<Vec<u8>>,
    codec_delay_ns: Option<u64>,
    seek_pre_roll_ns: Option<u64>,
    sample_rate_hz: Option<f64>,
    channels: Option<u16>,
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

#[cfg(feature = "recording-opus-webm")]
pub(in crate::recording) fn visit_vp9_opus_packets<F>(
    spec: WebmRemuxSpec,
    source: &WebmAvRemuxSource,
    on_packet: F,
) -> Result<ParsedAvStats, String>
where
    F: for<'packet> FnMut(ParsedAvPacket<'packet>) -> Result<(), String>,
{
    StrictWebmReader::open_verified(&source.source)?.read_av_packets(spec, source, on_packet)
}

#[cfg(feature = "recording-opus-webm")]
fn expected_opus_padding_ns(audio: WebmThumbnailAudioSpec) -> Result<u64, String> {
    let encoded_frames = audio
        .packet_count
        .checked_mul(OPUS_PACKET_FRAMES as u64)
        .ok_or_else(|| "录屏恢复 Opus 编码帧数溢出".to_string())?;
    let consumed_frames = u64::from(audio.pre_skip_frames)
        .checked_add(audio.pcm_frame_count)
        .ok_or_else(|| "录屏恢复 Opus 真实帧数溢出".to_string())?;
    let padding_frames = encoded_frames
        .checked_sub(consumed_frames)
        .filter(|padding| *padding < OPUS_PACKET_FRAMES as u64)
        .ok_or_else(|| "录屏恢复 Opus padding 与清单不一致".to_string())?;
    audio_frames_to_ns(padding_frames)
}

#[cfg(feature = "recording-opus-webm")]
fn audio_frames_to_ns(frames: u64) -> Result<u64, String> {
    u128::from(frames)
        .checked_mul(1_000_000_000)
        .and_then(|value| value.checked_div(u128::from(AUDIO_SAMPLE_RATE_HZ)))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| "录屏恢复音频帧时间溢出".to_string())
}

/// 校验源产物并只取第一个 VP9 关键帧 packet，供结果库生成持久缩略图。
///
/// 文件长度与 SHA-256 仍完整校验；容器只解析到第一帧，避免为了一个缩略图遍历数小时的 block。
pub(in crate::recording) fn first_vp9_keyframe(
    source: &WebmRemuxSource,
    spec: WebmRemuxSpec,
    audio: Option<WebmThumbnailAudioSpec>,
) -> Result<Vec<u8>, String> {
    if source.duration_ns == 0
        || source.frame_count == 0
        || spec.width == 0
        || spec.height == 0
        || spec.fps_numerator == 0
        || spec.fps_denominator == 0
    {
        return Err("录屏缩略图源参数无效".to_string());
    }
    if audio.is_some_and(|audio| {
        audio.sample_rate_hz != 48_000
            || !matches!(audio.channels, 1 | 2)
            || audio.pre_skip_frames == 0
            || audio.packet_count == 0
            || audio.pcm_frame_count == 0
    }) {
        return Err("录屏缩略图音轨参数无效".to_string());
    }
    StrictWebmReader::open_verified(source)?.read_first_keyframe(spec, source, audio)
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
        let mut tracks: Option<Vec<ParsedTrack>> = None;
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
                    if tracks.is_some() || cluster_seen {
                        return Err("录屏恢复分段包含重复 Tracks".to_string());
                    }
                    tracks = Some(self.read_tracks(header.data_end)?);
                }
                CLUSTER => {
                    cluster_seen = true;
                    let tracks = tracks
                        .as_ref()
                        .ok_or_else(|| "录屏恢复分段在轨道之前出现画面".to_string())?;
                    let track = validate_video_only_tracks(tracks, spec)?;
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
        let tracks = tracks.ok_or_else(|| "录屏恢复分段缺少视频轨".to_string())?;
        validate_video_only_tracks(&tracks, spec)?;
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

    #[cfg(feature = "recording-opus-webm")]
    fn read_av_packets<F>(
        &mut self,
        spec: WebmRemuxSpec,
        source: &WebmAvRemuxSource,
        mut on_packet: F,
    ) -> Result<ParsedAvStats, String>
    where
        F: for<'packet> FnMut(ParsedAvPacket<'packet>) -> Result<(), String>,
    {
        let ebml = self.read_header(self.file_len, false)?;
        if ebml.id != EBML {
            return Err("录屏双轨恢复分段缺少 EBML 头".to_string());
        }
        self.seek_to(ebml.data_end)?;
        let segment = loop {
            if self.position()? >= self.file_len {
                return Err("录屏双轨恢复分段缺少 Segment".to_string());
            }
            let header = self.read_header(self.file_len, true)?;
            if header.id == SEGMENT {
                break header;
            }
            if header.id != VOID {
                return Err("录屏双轨恢复分段根元素无效".to_string());
            }
            self.seek_to(header.data_end)?;
        };
        if segment.data_end != self.file_len {
            return Err("录屏双轨恢复分段包含 Segment 外数据".to_string());
        }

        let mut info_seen = false;
        let mut timecode_scale = WEBM_TIMECODE_SCALE_NS;
        let mut tracks: Option<Vec<ParsedTrack>> = None;
        let mut cluster_seen = false;
        let mut stats = ParsedAvStats::default();
        let mut last_packet = None;
        let mut observed_discard_padding_ns = None;
        while self.position()? < segment.data_end {
            let header = self.read_header(segment.data_end, false)?;
            match header.id {
                INFO => {
                    if info_seen || cluster_seen {
                        return Err("录屏双轨恢复分段包含重复 Info".to_string());
                    }
                    timecode_scale = self.read_info(header.data_end)?;
                    info_seen = true;
                }
                TRACKS => {
                    if tracks.is_some() || cluster_seen {
                        return Err("录屏双轨恢复分段包含重复 Tracks".to_string());
                    }
                    tracks = Some(self.read_tracks(header.data_end)?);
                }
                CLUSTER => {
                    if !info_seen || timecode_scale != AV_WEBM_TIMECODE_SCALE_NS {
                        return Err("录屏双轨恢复分段时间基不受支持".to_string());
                    }
                    cluster_seen = true;
                    let tracks = tracks
                        .as_ref()
                        .ok_or_else(|| "录屏双轨恢复分段在轨道之前出现 packet".to_string())?;
                    let (video_track, audio_track) =
                        validate_thumbnail_tracks(tracks, spec, Some(source.audio))?;
                    self.read_av_cluster(
                        header.data_end,
                        timecode_scale,
                        video_track,
                        audio_track.expect("双轨校验保证 Opus 轨存在"),
                        spec,
                        source,
                        &mut stats,
                        &mut last_packet,
                        &mut observed_discard_padding_ns,
                        &mut on_packet,
                    )?;
                }
                SEEK_HEAD | CUES | VOID => self.seek_to(header.data_end)?,
                _ => return Err("录屏双轨恢复分段包含非生产顶层元素".to_string()),
            }
        }
        let tracks = tracks.ok_or_else(|| "录屏双轨恢复分段缺少轨道".to_string())?;
        validate_thumbnail_tracks(&tracks, spec, Some(source.audio))?;
        if !info_seen || !cluster_seen {
            return Err("录屏双轨恢复分段缺少生产容器元数据或 packet".to_string());
        }
        let expected_padding_ns = expected_opus_padding_ns(source.audio)?;
        if stats.video_frame_count != source.source.frame_count
            || stats.audio_packet_count != source.audio.packet_count
            || observed_discard_padding_ns.unwrap_or(0) != expected_padding_ns
            || (expected_padding_ns == 0 && observed_discard_padding_ns.is_some())
        {
            return Err("录屏双轨恢复分段统计或尾 padding 与清单不一致".to_string());
        }
        Ok(stats)
    }

    #[cfg(feature = "recording-opus-webm")]
    #[allow(clippy::too_many_arguments)]
    fn read_av_cluster<F>(
        &mut self,
        end: u64,
        timecode_scale: u64,
        video_track: u64,
        audio_track: u64,
        spec: WebmRemuxSpec,
        source: &WebmAvRemuxSource,
        stats: &mut ParsedAvStats,
        last_packet: &mut Option<(u64, ThumbnailTrackKind)>,
        observed_discard_padding_ns: &mut Option<u64>,
        on_packet: &mut F,
    ) -> Result<(), String>
    where
        F: for<'packet> FnMut(ParsedAvPacket<'packet>) -> Result<(), String>,
    {
        let mut cluster_timecode = None;
        while self.position()? < end {
            let header = self.read_header(end, false)?;
            match header.id {
                CLUSTER_TIMECODE => {
                    if cluster_timecode.is_some() {
                        return Err("录屏双轨恢复 cluster 包含重复时间戳".to_string());
                    }
                    cluster_timecode = Some(self.read_uint(header)?);
                }
                SIMPLE_BLOCK => {
                    let cluster_timecode = cluster_timecode
                        .ok_or_else(|| "录屏双轨恢复 block 缺少 cluster 时间戳".to_string())?;
                    let body = self.read_bounded_body(header, MAX_PACKET_BYTES + 16)?;
                    let parsed = parse_simple_block(&body, cluster_timecode, timecode_scale)?;
                    let kind =
                        thumbnail_track_kind(parsed.track_number, video_track, Some(audio_track))?;
                    match kind {
                        ThumbnailTrackKind::Video => {
                            if stats.video_frame_count == 0 && !parsed.keyframe {
                                return Err("录屏双轨恢复分段没有从关键帧开始".to_string());
                            }
                            validate_frame_timestamp(
                                parsed.timestamp_ns,
                                stats.video_frame_count,
                                spec,
                                &source.source,
                            )?;
                        }
                        ThumbnailTrackKind::Audio => {
                            if !parsed.keyframe || observed_discard_padding_ns.is_some() {
                                return Err(
                                    "录屏双轨恢复 Opus SimpleBlock 标志或顺序无效".to_string()
                                );
                            }
                            validate_audio_timestamp(
                                parsed.timestamp_ns,
                                stats.audio_packet_count,
                                source.audio,
                            )?;
                        }
                    }
                    validate_av_packet_order(last_packet, parsed.timestamp_ns, kind)?;
                    match kind {
                        ThumbnailTrackKind::Video => {
                            on_packet(ParsedAvPacket::Video {
                                data: &body[parsed.payload_offset..],
                                timestamp_ns: parsed.timestamp_ns,
                                keyframe: parsed.keyframe,
                            })?;
                            stats.video_frame_count = stats
                                .video_frame_count
                                .checked_add(1)
                                .ok_or_else(|| "录屏双轨恢复视频帧数溢出".to_string())?;
                        }
                        ThumbnailTrackKind::Audio => {
                            on_packet(ParsedAvPacket::Audio {
                                data: &body[parsed.payload_offset..],
                                timestamp_ns: parsed.timestamp_ns,
                                discard_padding_ns: 0,
                            })?;
                            stats.audio_packet_count = stats
                                .audio_packet_count
                                .checked_add(1)
                                .ok_or_else(|| "录屏双轨恢复 Opus packet 数溢出".to_string())?;
                        }
                    }
                }
                BLOCK_GROUP => {
                    if observed_discard_padding_ns.is_some() {
                        return Err("录屏双轨恢复包含多个尾 padding".to_string());
                    }
                    let cluster_timecode = cluster_timecode
                        .ok_or_else(|| "录屏双轨恢复 BlockGroup 缺少 cluster 时间戳".to_string())?;
                    let (body, discard_padding_ns) = self.read_opus_block_group(header.data_end)?;
                    let parsed = parse_simple_block(&body, cluster_timecode, timecode_scale)?;
                    if parsed.track_number != audio_track || parsed.keyframe {
                        return Err("录屏双轨恢复 BlockGroup 不是生产 Opus 尾包".to_string());
                    }
                    validate_audio_timestamp(
                        parsed.timestamp_ns,
                        stats.audio_packet_count,
                        source.audio,
                    )?;
                    validate_av_packet_order(
                        last_packet,
                        parsed.timestamp_ns,
                        ThumbnailTrackKind::Audio,
                    )?;
                    on_packet(ParsedAvPacket::Audio {
                        data: &body[parsed.payload_offset..],
                        timestamp_ns: parsed.timestamp_ns,
                        discard_padding_ns,
                    })?;
                    stats.audio_packet_count = stats
                        .audio_packet_count
                        .checked_add(1)
                        .ok_or_else(|| "录屏双轨恢复 Opus packet 数溢出".to_string())?;
                    *observed_discard_padding_ns = Some(discard_padding_ns);
                }
                CLUSTER_POSITION | PREVIOUS_CLUSTER_SIZE | VOID => self.seek_to(header.data_end)?,
                _ => return Err("录屏双轨恢复 cluster 包含非生产元素".to_string()),
            }
            if stats.video_frame_count > source.source.frame_count
                || stats.audio_packet_count > source.audio.packet_count
            {
                return Err("录屏双轨恢复 packet 数超过清单".to_string());
            }
        }
        Ok(())
    }

    #[cfg(feature = "recording-opus-webm")]
    fn read_opus_block_group(&mut self, end: u64) -> Result<(Vec<u8>, u64), String> {
        let mut block = None;
        let mut discard_padding = None;
        while self.position()? < end {
            let header = self.read_header(end, false)?;
            match header.id {
                BLOCK => {
                    if block.is_some() {
                        return Err("录屏双轨恢复 BlockGroup 包含重复 Block".to_string());
                    }
                    block = Some(self.read_bounded_body(header, MAX_PACKET_BYTES + 16)?);
                }
                DISCARD_PADDING => {
                    if discard_padding.is_some() {
                        return Err("录屏双轨恢复 BlockGroup 包含重复 DiscardPadding".to_string());
                    }
                    let value = self.read_signed_int(header)?;
                    discard_padding = Some(
                        u64::try_from(value)
                            .ok()
                            .filter(|value| *value > 0)
                            .ok_or_else(|| "录屏双轨恢复 DiscardPadding 无效".to_string())?,
                    );
                }
                _ => return Err("录屏双轨恢复 BlockGroup 包含非生产元素".to_string()),
            }
        }
        Ok((
            block.ok_or_else(|| "录屏双轨恢复 BlockGroup 缺少 Block".to_string())?,
            discard_padding
                .ok_or_else(|| "录屏双轨恢复 BlockGroup 缺少 DiscardPadding".to_string())?,
        ))
    }

    fn read_first_keyframe(
        &mut self,
        spec: WebmRemuxSpec,
        source: &WebmRemuxSource,
        audio: Option<WebmThumbnailAudioSpec>,
    ) -> Result<Vec<u8>, String> {
        let ebml = self.read_header(self.file_len, false)?;
        if ebml.id != EBML {
            return Err("录屏缩略图源缺少 EBML 头".to_string());
        }
        self.seek_to(ebml.data_end)?;
        let segment = loop {
            if self.position()? >= self.file_len {
                return Err("录屏缩略图源缺少 Segment".to_string());
            }
            let header = self.read_header(self.file_len, true)?;
            if header.id == SEGMENT {
                break header;
            }
            if header.id != VOID {
                return Err("录屏缩略图源根元素无效".to_string());
            }
            self.seek_to(header.data_end)?;
        };
        if segment.data_end != self.file_len {
            return Err("录屏缩略图源包含 Segment 外数据".to_string());
        }

        let mut info_seen = false;
        let mut timecode_scale = WEBM_TIMECODE_SCALE_NS;
        let mut tracks: Option<Vec<ParsedTrack>> = None;
        let expected_timecode_scale = if audio.is_some() {
            AV_WEBM_TIMECODE_SCALE_NS
        } else {
            WEBM_TIMECODE_SCALE_NS
        };
        while self.position()? < segment.data_end {
            let header = self.read_header(segment.data_end, false)?;
            match header.id {
                INFO => {
                    if info_seen {
                        return Err("录屏缩略图源包含重复 Info".to_string());
                    }
                    timecode_scale = self.read_info(header.data_end)?;
                    info_seen = true;
                }
                TRACKS => {
                    if tracks.is_some() {
                        return Err("录屏缩略图源包含重复 Tracks".to_string());
                    }
                    tracks = Some(self.read_tracks(header.data_end)?);
                }
                CLUSTER => {
                    if !info_seen || timecode_scale != expected_timecode_scale {
                        return Err(format!(
                            "录屏缩略图源时间基不受支持: scale={timecode_scale}"
                        ));
                    }
                    let tracks = tracks
                        .as_ref()
                        .ok_or_else(|| "录屏缩略图源在轨道之前出现画面".to_string())?;
                    let (video_track_number, audio_track_number) =
                        validate_thumbnail_tracks(tracks, spec, audio)?;
                    if let Some(packet) = self.read_first_keyframe_in_cluster(
                        header.data_end,
                        timecode_scale,
                        video_track_number,
                        audio_track_number,
                        spec,
                        source,
                    )? {
                        return Ok(packet);
                    }
                }
                SEEK_HEAD | CUES | VOID => self.seek_to(header.data_end)?,
                _ => return Err("录屏缩略图源包含非生产顶层元素".to_string()),
            }
        }
        let tracks = tracks.ok_or_else(|| "录屏缩略图源缺少轨道".to_string())?;
        validate_thumbnail_tracks(&tracks, spec, audio)?;
        Err("录屏缩略图源没有画面".to_string())
    }

    fn read_first_keyframe_in_cluster(
        &mut self,
        end: u64,
        timecode_scale: u64,
        video_track_number: u64,
        audio_track_number: Option<u64>,
        spec: WebmRemuxSpec,
        source: &WebmRemuxSource,
    ) -> Result<Option<Vec<u8>>, String> {
        let mut cluster_timecode = None;
        while self.position()? < end {
            let header = self.read_header(end, false)?;
            match header.id {
                CLUSTER_TIMECODE => {
                    if cluster_timecode.is_some() {
                        return Err("录屏缩略图 cluster 包含重复时间戳".to_string());
                    }
                    cluster_timecode = Some(self.read_uint(header)?);
                }
                SIMPLE_BLOCK => {
                    let cluster_timecode = cluster_timecode
                        .ok_or_else(|| "录屏缩略图 block 缺少 cluster 时间戳".to_string())?;
                    let body = self.read_bounded_body(header, MAX_PACKET_BYTES + 16)?;
                    let parsed = parse_simple_block(&body, cluster_timecode, timecode_scale)?;
                    match validate_thumbnail_block(
                        parsed.track_number,
                        parsed.keyframe,
                        video_track_number,
                        audio_track_number,
                    )? {
                        ThumbnailTrackKind::Audio => continue,
                        ThumbnailTrackKind::Video => {}
                    }
                    validate_frame_timestamp(parsed.timestamp_ns, 0, spec, source)?;
                    return Ok(Some(body[parsed.payload_offset..].to_vec()));
                }
                BLOCK_GROUP => return Err("录屏缩略图源包含不受支持的 BlockGroup".to_string()),
                CLUSTER_POSITION | PREVIOUS_CLUSTER_SIZE | VOID => self.seek_to(header.data_end)?,
                _ => return Err("录屏缩略图 cluster 包含非生产元素".to_string()),
            }
        }
        Ok(None)
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

    fn read_tracks(&mut self, end: u64) -> Result<Vec<ParsedTrack>, String> {
        let mut tracks = Vec::with_capacity(2);
        while self.position()? < end {
            let header = self.read_header(end, false)?;
            if header.id == TRACK_ENTRY {
                if tracks.len() >= 2 {
                    return Err("录屏恢复分段包含额外轨道".to_string());
                }
                tracks.push(self.read_track_entry(header.data_end)?);
            } else {
                self.seek_to(header.data_end)?;
            }
        }
        if tracks.is_empty() {
            return Err("录屏恢复分段缺少 TrackEntry".to_string());
        }
        Ok(tracks)
    }

    fn read_track_entry(&mut self, end: u64) -> Result<ParsedTrack, String> {
        let mut track = ParsedTrack::default();
        while self.position()? < end {
            let header = self.read_header(end, false)?;
            match header.id {
                TRACK_NUMBER => set_once(&mut track.number, self.read_uint(header)?)?,
                TRACK_TYPE => set_once(&mut track.track_type, self.read_uint(header)?)?,
                CODEC_ID => set_once(&mut track.codec_id, self.read_string(header)?)?,
                CODEC_PRIVATE => set_once(
                    &mut track.codec_private,
                    self.read_bounded_body(header, MAX_TRACK_METADATA_BYTES)?,
                )?,
                CODEC_DELAY => set_once(&mut track.codec_delay_ns, self.read_uint(header)?)?,
                SEEK_PRE_ROLL => set_once(&mut track.seek_pre_roll_ns, self.read_uint(header)?)?,
                VIDEO => self.read_video(header.data_end, &mut track)?,
                AUDIO => self.read_audio(header.data_end, &mut track)?,
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
                    set_once(
                        &mut track.width,
                        u32::try_from(self.read_uint(header)?)
                            .map_err(|_| "录屏恢复视频宽度溢出".to_string())?,
                    )?;
                }
                PIXEL_HEIGHT => {
                    set_once(
                        &mut track.height,
                        u32::try_from(self.read_uint(header)?)
                            .map_err(|_| "录屏恢复视频高度溢出".to_string())?,
                    )?;
                }
                _ => self.seek_to(header.data_end)?,
            }
        }
        Ok(())
    }

    fn read_audio(&mut self, end: u64, track: &mut ParsedTrack) -> Result<(), String> {
        while self.position()? < end {
            let header = self.read_header(end, false)?;
            match header.id {
                SAMPLING_FREQUENCY => {
                    let sample_rate = self.read_float(header)?;
                    if !sample_rate.is_finite() {
                        return Err("录屏恢复音频采样率无效".to_string());
                    }
                    set_once(&mut track.sample_rate_hz, sample_rate)?;
                }
                CHANNELS => {
                    let channels = u16::try_from(self.read_uint(header)?)
                        .map_err(|_| "录屏恢复音频声道数溢出".to_string())?;
                    set_once(&mut track.channels, channels)?;
                }
                _ => self.seek_to(header.data_end)?,
            }
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

    #[cfg(feature = "recording-opus-webm")]
    fn read_signed_int(&mut self, header: ElementHeader) -> Result<i64, String> {
        let length = header.data_end - header.data_start;
        if length == 0 || length > 8 {
            return Err("录屏双轨恢复 EBML 有符号整数长度无效".to_string());
        }
        let mut bytes = [0_u8; 8];
        let offset = 8 - length as usize;
        self.reader
            .read_exact(&mut bytes[offset..])
            .map_err(|error| format!("读取录屏双轨恢复 EBML 有符号整数失败: {error}"))?;
        if bytes[offset] & 0x80 != 0 {
            bytes[..offset].fill(0xff);
        }
        Ok(i64::from_be_bytes(bytes))
    }

    fn read_float(&mut self, header: ElementHeader) -> Result<f64, String> {
        let length = header.data_end - header.data_start;
        match length {
            4 => {
                let mut bytes = [0_u8; 4];
                self.reader
                    .read_exact(&mut bytes)
                    .map_err(|error| format!("读取录屏恢复 EBML 浮点数失败: {error}"))?;
                Ok(f64::from(f32::from_be_bytes(bytes)))
            }
            8 => {
                let mut bytes = [0_u8; 8];
                self.reader
                    .read_exact(&mut bytes)
                    .map_err(|error| format!("读取录屏恢复 EBML 浮点数失败: {error}"))?;
                Ok(f64::from_be_bytes(bytes))
            }
            _ => Err("录屏恢复 EBML 浮点数长度无效".to_string()),
        }
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

fn validate_video_track(track: &ParsedTrack, spec: WebmRemuxSpec) -> Result<(), String> {
    if track.number != Some(1)
        || track.track_type != Some(1)
        || track.codec_id.as_deref() != Some("V_VP9")
        || track.width != Some(spec.width)
        || track.height != Some(spec.height)
        || track.codec_private.is_some()
        || track.codec_delay_ns.is_some()
        || track.seek_pre_roll_ns.is_some()
        || track.sample_rate_hz.is_some()
        || track.channels.is_some()
    {
        return Err("录屏恢复视频轨与清单不一致".to_string());
    }
    Ok(())
}

fn validate_video_only_tracks(
    tracks: &[ParsedTrack],
    spec: WebmRemuxSpec,
) -> Result<&ParsedTrack, String> {
    if tracks.len() != 1 {
        return Err("录屏恢复分段不是单轨视频".to_string());
    }
    let track = &tracks[0];
    validate_video_track(track, spec)?;
    Ok(track)
}

fn validate_thumbnail_tracks(
    tracks: &[ParsedTrack],
    spec: WebmRemuxSpec,
    audio: Option<WebmThumbnailAudioSpec>,
) -> Result<(u64, Option<u64>), String> {
    let video = tracks
        .iter()
        .find(|track| track.number == Some(1))
        .ok_or_else(|| "录屏缩略图源缺少视频轨".to_string())?;
    validate_video_track(video, spec)?;
    let Some(audio) = audio else {
        if tracks.len() != 1 {
            return Err("录屏缩略图单轨清单与容器不一致".to_string());
        }
        return Ok((1, None));
    };
    if tracks.len() != 2 {
        return Err("录屏缩略图双轨容器形状无效".to_string());
    }
    let audio_track = tracks
        .iter()
        .find(|track| track.number == Some(2))
        .ok_or_else(|| "录屏缩略图源缺少 Opus 轨".to_string())?;
    validate_opus_track(audio_track, audio)?;
    Ok((1, Some(2)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThumbnailTrackKind {
    Video,
    Audio,
}

fn thumbnail_track_kind(
    track_number: u64,
    video_track_number: u64,
    audio_track_number: Option<u64>,
) -> Result<ThumbnailTrackKind, String> {
    if track_number == video_track_number {
        Ok(ThumbnailTrackKind::Video)
    } else if Some(track_number) == audio_track_number {
        Ok(ThumbnailTrackKind::Audio)
    } else {
        Err("录屏缩略图 block 引用了未知轨道".to_string())
    }
}

fn validate_thumbnail_block(
    track_number: u64,
    keyframe: bool,
    video_track_number: u64,
    audio_track_number: Option<u64>,
) -> Result<ThumbnailTrackKind, String> {
    let kind = thumbnail_track_kind(track_number, video_track_number, audio_track_number)?;
    match (kind, keyframe) {
        (ThumbnailTrackKind::Audio, true) => Ok(kind),
        (ThumbnailTrackKind::Audio, false) => Err("录屏缩略图 Opus block 标志无效".to_string()),
        (ThumbnailTrackKind::Video, true) => Ok(kind),
        (ThumbnailTrackKind::Video, false) => Err("录屏缩略图源没有从关键帧开始".to_string()),
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T) -> Result<(), String> {
    if slot.is_some() {
        return Err("录屏恢复轨道元数据重复".to_string());
    }
    *slot = Some(value);
    Ok(())
}

fn validate_opus_track(track: &ParsedTrack, audio: WebmThumbnailAudioSpec) -> Result<(), String> {
    let expected_delay_ns = u64::from(audio.pre_skip_frames)
        .checked_mul(1_000_000_000)
        .and_then(|value| value.checked_div(u64::from(audio.sample_rate_hz)))
        .ok_or_else(|| "录屏缩略图 Opus 延迟溢出".to_string())?;
    let expected_head = opus_head(audio.channels, audio.pre_skip_frames, audio.sample_rate_hz)?;
    if audio.codec_delay_ns != expected_delay_ns
        || track.number != Some(2)
        || track.track_type != Some(2)
        || track.codec_id.as_deref() != Some("A_OPUS")
        || track.codec_private.as_deref() != Some(expected_head.as_slice())
        || track.codec_delay_ns != Some(audio.codec_delay_ns)
        || track.seek_pre_roll_ns != Some(audio.seek_pre_roll_ns)
        || track.sample_rate_hz != Some(f64::from(audio.sample_rate_hz))
        || track.channels != Some(audio.channels)
        || track.width.is_some()
        || track.height.is_some()
    {
        return Err("录屏缩略图 Opus 轨与清单不一致".to_string());
    }
    Ok(())
}

pub(in crate::recording) fn opus_head(
    channels: u16,
    pre_skip_frames: u16,
    sample_rate_hz: u32,
) -> Result<[u8; 19], String> {
    let channels = u8::try_from(channels).map_err(|_| "录屏缩略图 Opus 声道数无效".to_string())?;
    if !matches!(channels, 1 | 2) || pre_skip_frames == 0 || sample_rate_hz != 48_000 {
        return Err("录屏缩略图 Opus 参数无效".to_string());
    }
    let mut head = [0_u8; 19];
    head[0..8].copy_from_slice(b"OpusHead");
    head[8] = 1;
    head[9] = channels;
    head[10..12].copy_from_slice(&pre_skip_frames.to_le_bytes());
    head[12..16].copy_from_slice(&sample_rate_hz.to_le_bytes());
    head[16..18].copy_from_slice(&0_i16.to_le_bytes());
    head[18] = 0;
    Ok(head)
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
    if flags & 0x06 != 0 || flags & !0x80 != 0 {
        return Err("录屏恢复不接受非生产 block 标志或 lacing".to_string());
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

#[cfg(feature = "recording-opus-webm")]
fn validate_audio_timestamp(
    timestamp_ns: u64,
    packet_index: u64,
    audio: WebmThumbnailAudioSpec,
) -> Result<(), String> {
    let expected = packet_index
        .checked_mul(u64::from(DEFAULT_OPUS_FRAME_MS) * 1_000_000)
        .and_then(|offset| audio.codec_delay_ns.checked_add(offset))
        .ok_or_else(|| "录屏双轨恢复 Opus 时间戳溢出".to_string())?;
    if timestamp_ns.abs_diff(expected) >= AV_WEBM_TIMECODE_SCALE_NS {
        return Err("录屏双轨恢复 Opus 时间戳与 packet 序号不一致".to_string());
    }
    Ok(())
}

#[cfg(feature = "recording-opus-webm")]
fn validate_av_packet_order(
    last: &mut Option<(u64, ThumbnailTrackKind)>,
    timestamp_ns: u64,
    kind: ThumbnailTrackKind,
) -> Result<(), String> {
    if let Some((last_timestamp_ns, last_kind)) = *last {
        if timestamp_ns < last_timestamp_ns
            || (timestamp_ns == last_timestamp_ns
                && !matches!(
                    (last_kind, kind),
                    (ThumbnailTrackKind::Video, ThumbnailTrackKind::Audio)
                ))
        {
            return Err("录屏双轨恢复 packet 顺序无效".to_string());
        }
    }
    *last = Some((timestamp_ns, kind));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "recording-opus-webm")]
    use crate::recording::audio::{
        frames_to_ns, AudioFormat, CapturedAudioChunk, QueuedAudioChunk,
    };
    #[cfg(feature = "recording-opus-webm")]
    use crate::recording::av_recovery::remux_vp9_opus_segments;
    #[cfg(feature = "recording-opus-webm")]
    use crate::recording::mux::opus_webm::{AvWebmPacketMux, EncodedOpusPacket, OpusPacketEncoder};
    use crate::recording::mux::vp9_webm::{Vp9PacketEncoder, Vp9WebmWriter};
    #[cfg(feature = "recording-opus-webm")]
    use opusic_c::{Channels, Decoder, SampleRate};
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

    #[cfg(feature = "recording-opus-webm")]
    enum OwnedAvPacket {
        Video {
            data: Vec<u8>,
            timestamp_ns: u64,
            keyframe: bool,
        },
        Audio(EncodedOpusPacket),
    }

    #[cfg(feature = "recording-opus-webm")]
    impl OwnedAvPacket {
        fn order_key(&self) -> (u64, u8) {
            match self {
                Self::Video { timestamp_ns, .. } => (*timestamp_ns, 0),
                Self::Audio(packet) => (packet.timestamp_ns, 1),
            }
        }
    }

    #[cfg(feature = "recording-opus-webm")]
    fn queued_pcm(start_frame: u64, frame_count: u32, channels: u16) -> QueuedAudioChunk {
        let presentation_at_ns = audio_frames_to_ns(start_frame).unwrap();
        QueuedAudioChunk {
            chunk: CapturedAudioChunk {
                sequence: start_frame,
                captured_at_ns: presentation_at_ns,
                format: AudioFormat::normalized(channels),
                frame_count,
                samples: (0..frame_count as usize * usize::from(channels))
                    .map(|index| ((index % 97) as f32 / 96.0 - 0.5) * 0.2)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            },
            presentation_at_ns,
            duration_ns: frames_to_ns(frame_count).unwrap(),
            gap_before_ns: 0,
        }
    }

    #[cfg(feature = "recording-opus-webm")]
    fn write_av_segment(
        path: &std::path::Path,
        color: [u8; 3],
        started_at_ns: u64,
    ) -> WebmAvRemuxSource {
        let duration_ns = 50_000_000;
        let frame_count = 3_u64;
        let channels = 2_u16;
        let mut packets = Vec::new();
        let mut video = Vp9PacketEncoder::new(64, 48, 60, 1).unwrap();
        let mut emit = |data: &[u8], timestamp_ns: u64, keyframe: bool| {
            packets.push(OwnedAvPacket::Video {
                data: data.to_vec(),
                timestamp_ns,
                keyframe,
            });
            Ok(())
        };
        for index in 0..frame_count {
            let timestamp_ns = u128::from(index)
                .checked_mul(1_000_000_000)
                .and_then(|value| value.checked_div(60))
                .and_then(|value| u64::try_from(value).ok())
                .unwrap();
            video
                .push_rgba(
                    &solid_rgba(
                        64,
                        48,
                        [color[0].wrapping_add(index as u8), color[1], color[2]],
                    ),
                    timestamp_ns,
                    &mut emit,
                )
                .unwrap();
        }
        video.finish(duration_ns, &mut emit).unwrap();

        let mut audio = OpusPacketEncoder::new(channels).unwrap();
        let track = audio.track_config().clone();
        let mut audio_packets = audio.push(queued_pcm(0, 2_400, channels)).unwrap();
        let audio_finish = audio.finish().unwrap();
        audio_packets.extend(audio_finish.packets);
        packets.extend(audio_packets.into_iter().map(OwnedAvPacket::Audio));
        packets.sort_by_key(OwnedAvPacket::order_key);

        let mut mux = AvWebmPacketMux::new(Cursor::new(Vec::new()), 64, 48, &track).unwrap();
        for packet in &packets {
            match packet {
                OwnedAvPacket::Video {
                    data,
                    timestamp_ns,
                    keyframe,
                } => mux
                    .add_video_packet(data, *timestamp_ns, *keyframe)
                    .unwrap(),
                OwnedAvPacket::Audio(packet) => mux.add_audio_packet(packet).unwrap(),
            }
        }
        let output = mux.finish(duration_ns).unwrap();
        let bytes = output.writer.into_inner();
        fs::write(path, &bytes).unwrap();
        WebmAvRemuxSource {
            source: WebmRemuxSource {
                path: path.to_path_buf(),
                byte_length: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                started_at_ns,
                duration_ns,
                frame_count,
            },
            audio: WebmThumbnailAudioSpec {
                sample_rate_hz: AUDIO_SAMPLE_RATE_HZ,
                channels,
                pre_skip_frames: track.pre_skip_frames,
                codec_delay_ns: track.codec_delay_ns,
                seek_pre_roll_ns: track.seek_pre_roll_ns,
                packet_count: audio_finish.stats.packet_count,
                pcm_frame_count: audio_finish.stats.real_frames,
            },
        }
    }

    #[cfg(feature = "recording-opus-webm")]
    fn av_payloads(
        source: &WebmAvRemuxSource,
        spec: WebmRemuxSpec,
    ) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        let mut input = StrictWebmReader::open_verified(&source.source).unwrap();
        let mut video = Vec::new();
        let mut audio = Vec::new();
        input
            .read_av_packets(spec, source, |packet| {
                match packet {
                    ParsedAvPacket::Video { data, .. } => video.push(data.to_vec()),
                    ParsedAvPacket::Audio { data, .. } => audio.push(data.to_vec()),
                }
                Ok(())
            })
            .unwrap();
        (video, audio)
    }

    #[test]
    fn remuxes_two_production_segments_without_reencoding() {
        let directory = tempfile::tempdir().unwrap();
        let first_path = directory.path().join("first.webm");
        let second_path = directory.path().join("second.webm");
        let first = write_segment(&first_path, [16, 32, 64]);
        let spec = WebmRemuxSpec {
            width: 64,
            height: 48,
            fps_numerator: 10,
            fps_denominator: 1,
        };
        assert_eq!(
            first_vp9_keyframe(&first, spec, None).unwrap(),
            packets(&first)[0]
        );
        let single =
            remux_vp9_segments(std::slice::from_ref(&first), Cursor::new(Vec::new()), spec)
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
        let output = remux_vp9_segments(&sources, Cursor::new(Vec::new()), spec).unwrap();
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

    #[cfg(feature = "recording-opus-webm")]
    #[test]
    fn recovers_independent_opus_segments_and_keeps_vp9_payloads() {
        let directory = tempfile::tempdir().unwrap();
        let spec = WebmRemuxSpec {
            width: 64,
            height: 48,
            fps_numerator: 60,
            fps_denominator: 1,
        };
        let first = write_av_segment(&directory.path().join("first-av.webm"), [16, 32, 64], 0);
        let second = write_av_segment(
            &directory.path().join("second-av.webm"),
            [160, 120, 80],
            50_000_000,
        );
        assert_eq!(first.audio.pcm_frame_count, 2_400);
        assert_eq!(second.audio.pcm_frame_count, 2_400);
        assert_eq!(expected_opus_padding_ns(first.audio).unwrap(), 3_500_000);
        let expected_video = av_payloads(&first, spec)
            .0
            .into_iter()
            .chain(av_payloads(&second, spec).0)
            .collect::<Vec<_>>();

        let output =
            remux_vp9_opus_segments(&[first, second], Cursor::new(Vec::new()), spec).unwrap();
        assert_eq!(output.video_frame_count, 6);
        assert_eq!(output.audio_pcm_frame_count, 4_800);
        assert_eq!(output.duration_ns, 100_000_000);
        let output_bytes = output.writer.into_inner();
        let output_path = directory.path().join("recovered-av.webm");
        fs::write(&output_path, &output_bytes).unwrap();
        let recovered = WebmAvRemuxSource {
            source: WebmRemuxSource {
                path: output_path.clone(),
                byte_length: output_bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&output_bytes)),
                started_at_ns: 0,
                duration_ns: 100_000_000,
                frame_count: 6,
            },
            audio: WebmThumbnailAudioSpec {
                sample_rate_hz: AUDIO_SAMPLE_RATE_HZ,
                channels: 2,
                pre_skip_frames: 312,
                codec_delay_ns: 6_500_000,
                seek_pre_roll_ns: 80_000_000,
                packet_count: output.audio_packet_count,
                pcm_frame_count: output.audio_pcm_frame_count,
            },
        };
        let (actual_video, actual_audio) = av_payloads(&recovered, spec);
        assert_eq!(actual_video, expected_video);
        assert_eq!(actual_audio.len() as u64, output.audio_packet_count);

        let mut decoder = Decoder::new(Channels::Stereo, SampleRate::Hz48000).unwrap();
        let mut decoded_frames = 0_u64;
        for packet in actual_audio {
            let mut pcm = vec![0.0_f32; OPUS_PACKET_FRAMES * 2];
            decoded_frames += decoder
                .decode_float_to_slice(&packet, &mut pcm, false)
                .unwrap() as u64;
        }
        let real_frames = decoded_frames
            .checked_sub(u64::from(recovered.audio.pre_skip_frames))
            .and_then(|frames| {
                let encoded = recovered.audio.packet_count * OPUS_PACKET_FRAMES as u64;
                let padding = encoded
                    - u64::from(recovered.audio.pre_skip_frames)
                    - recovered.audio.pcm_frame_count;
                frames.checked_sub(padding)
            })
            .unwrap();
        assert_eq!(real_frames, recovered.audio.pcm_frame_count);
    }

    #[cfg(feature = "recording-opus-webm")]
    #[test]
    fn rejects_dual_track_statistics_codec_and_padding_mismatches() {
        let directory = tempfile::tempdir().unwrap();
        let spec = WebmRemuxSpec {
            width: 64,
            height: 48,
            fps_numerator: 60,
            fps_denominator: 1,
        };
        let source = write_av_segment(&directory.path().join("strict-av.webm"), [12, 34, 56], 0);

        let mut wrong_packets = source.clone();
        wrong_packets.audio.packet_count += 1;
        assert!(remux_vp9_opus_segments(&[wrong_packets], Cursor::new(Vec::new()), spec,).is_err());

        let mut wrong_pcm = source.clone();
        wrong_pcm.audio.pcm_frame_count += 1;
        assert!(remux_vp9_opus_segments(&[wrong_pcm], Cursor::new(Vec::new()), spec).is_err());

        let mut wrong_codec = source.clone();
        let mut bytes = fs::read(&wrong_codec.source.path).unwrap();
        let codec = bytes
            .windows(6)
            .position(|window| window == b"A_OPUS")
            .unwrap();
        bytes[codec + 5] = b'X';
        fs::write(&wrong_codec.source.path, &bytes).unwrap();
        wrong_codec.source.sha256 = format!("{:x}", Sha256::digest(&bytes));
        assert!(remux_vp9_opus_segments(&[wrong_codec], Cursor::new(Vec::new()), spec,).is_err());

        let mut wrong_padding = write_av_segment(
            &directory.path().join("strict-padding.webm"),
            [78, 90, 12],
            0,
        );
        let mut bytes = fs::read(&wrong_padding.source.path).unwrap();
        let element = bytes
            .windows(2)
            .position(|window| window == [0x75, 0xa2])
            .unwrap();
        let size_offset = element + 2;
        let size_length = (bytes[size_offset].leading_zeros() + 1) as usize;
        let payload_length = usize::from(bytes[size_offset] & (0xff >> size_length));
        assert!(payload_length > 0);
        let last_payload_byte = size_offset + size_length + payload_length - 1;
        bytes[last_payload_byte] ^= 1;
        fs::write(&wrong_padding.source.path, &bytes).unwrap();
        wrong_padding.source.sha256 = format!("{:x}", Sha256::digest(&bytes));
        assert!(remux_vp9_opus_segments(&[wrong_padding], Cursor::new(Vec::new()), spec,).is_err());
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

    #[test]
    fn dual_track_thumbnail_contract_rejects_extra_tracks_and_unknown_blocks() {
        let spec = WebmRemuxSpec {
            width: 64,
            height: 48,
            fps_numerator: 10,
            fps_denominator: 1,
        };
        let audio = WebmThumbnailAudioSpec {
            sample_rate_hz: 48_000,
            channels: 2,
            pre_skip_frames: 312,
            codec_delay_ns: 6_500_000,
            seek_pre_roll_ns: 80_000_000,
            packet_count: 6,
            pcm_frame_count: 4_800,
        };
        let video = ParsedTrack {
            number: Some(1),
            track_type: Some(1),
            codec_id: Some("V_VP9".to_string()),
            width: Some(64),
            height: Some(48),
            ..ParsedTrack::default()
        };
        let opus = ParsedTrack {
            number: Some(2),
            track_type: Some(2),
            codec_id: Some("A_OPUS".to_string()),
            codec_private: Some(opus_head(2, 312, 48_000).unwrap().to_vec()),
            codec_delay_ns: Some(6_500_000),
            seek_pre_roll_ns: Some(80_000_000),
            sample_rate_hz: Some(48_000.0),
            channels: Some(2),
            ..ParsedTrack::default()
        };
        assert_eq!(
            validate_thumbnail_tracks(&[video.clone(), opus.clone()], spec, Some(audio)).unwrap(),
            (1, Some(2))
        );
        assert!(validate_thumbnail_tracks(
            &[video.clone(), opus.clone(), ParsedTrack::default()],
            spec,
            Some(audio),
        )
        .is_err());
        let mut wrong_head = opus;
        wrong_head.codec_private.as_mut().unwrap()[9] = 1;
        assert!(validate_thumbnail_tracks(&[video, wrong_head], spec, Some(audio)).is_err());
        assert_eq!(
            thumbnail_track_kind(1, 1, Some(2)).unwrap(),
            ThumbnailTrackKind::Video
        );
        assert_eq!(
            thumbnail_track_kind(2, 1, Some(2)).unwrap(),
            ThumbnailTrackKind::Audio
        );
        assert!(thumbnail_track_kind(3, 1, Some(2)).is_err());
        assert!(validate_thumbnail_block(3, true, 1, Some(2)).is_err());
        assert!(validate_thumbnail_block(1, false, 1, Some(2)).is_err());
        assert!(validate_thumbnail_block(2, false, 1, Some(2)).is_err());
    }
}
