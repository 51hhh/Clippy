use super::*;
use crate::recording::mux::opus_webm::MAX_OPUS_PACKET_BYTES;

const SEEK: u32 = 0x4DBB;
const SEEK_ID: u32 = 0x53AB;
const SEEK_POSITION: u32 = 0x53AC;
const CUE_POINT: u32 = 0xBB;
const CUE_TIME: u32 = 0xB3;
const CUE_TRACK_POSITIONS: u32 = 0xB7;
const CUE_TRACK: u32 = 0xF7;
const CUE_CLUSTER_POSITION: u32 = 0xF1;

pub struct IndexedPacket {
    pub timestamp_ns: u64,
    pub discard_padding_ns: u64,
    pub data: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Cue {
    time_ns: u64,
    cluster: u64,
}

#[derive(Default)]
struct Cursor {
    offset: u64,
    cluster_end: u64,
    cluster_ticks: Option<u64>,
    first_keyframe_ns: Option<u64>,
    previous_packet_ns: Option<u64>,
}

struct Layout {
    segment: ElementHeader,
    cues: ElementHeader,
    scale: u64,
    video_track: u64,
    audio_track: Option<u64>,
    duration_ns: u64,
}

pub struct IndexedWebm {
    reader: StrictWebmReader,
    layout: Layout,
    video: Cursor,
    audio: Cursor,
}

impl IndexedWebm {
    pub fn open(
        source: &WebmRemuxSource,
        spec: WebmRemuxSpec,
        audio: Option<WebmThumbnailAudioSpec>,
        mut checkpoint: impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        let mut reader = StrictWebmReader::open_with_checkpoint(source, &mut checkpoint)?;
        let layout = Layout::read(
            &mut reader,
            source.duration_ns,
            spec,
            audio,
            &mut checkpoint,
        )?;
        let mut indexed = Self {
            reader,
            layout,
            video: Cursor::default(),
            audio: Cursor::default(),
        };
        indexed.seek(0, 0, &mut checkpoint)?;
        Ok(indexed)
    }

    pub fn seek(
        &mut self,
        position_ns: u64,
        pre_roll_ns: u64,
        checkpoint: &mut impl FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        let (video, audio) =
            self.layout
                .lookup(&mut self.reader, position_ns, pre_roll_ns, checkpoint)?;
        self.video = Cursor {
            offset: video.cluster,
            first_keyframe_ns: Some(video.time_ns),
            ..Cursor::default()
        };
        self.audio = Cursor {
            offset: audio.cluster,
            ..Cursor::default()
        };
        Ok(())
    }

    pub fn next_video(
        &mut self,
        checkpoint: &mut impl FnMut() -> Result<(), String>,
    ) -> Result<Option<IndexedPacket>, String> {
        self.layout.next_packet(
            &mut self.reader,
            &mut self.video,
            ThumbnailTrackKind::Video,
            checkpoint,
        )
    }

    pub fn next_audio(
        &mut self,
        checkpoint: &mut impl FnMut() -> Result<(), String>,
    ) -> Result<Option<IndexedPacket>, String> {
        if self.layout.audio_track.is_none() {
            return Ok(None);
        }
        self.layout.next_packet(
            &mut self.reader,
            &mut self.audio,
            ThumbnailTrackKind::Audio,
            checkpoint,
        )
    }
}

impl Layout {
    fn read(
        reader: &mut StrictWebmReader,
        duration_ns: u64,
        spec: WebmRemuxSpec,
        audio: Option<WebmThumbnailAudioSpec>,
        checkpoint: &mut impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        let header = reader.read_header(reader.file_len, false)?;
        if header.id != EBML {
            return Err("录屏回放缺少 EBML 头".to_string());
        }
        reader.seek_to(header.data_end)?;
        let segment = loop {
            checkpoint()?;
            let header = reader.read_header(reader.file_len, true)?;
            match header.id {
                SEGMENT if header.data_end == reader.file_len => break header,
                VOID => reader.seek_to(header.data_end)?,
                _ => return Err("录屏回放 Segment 无效".to_string()),
            }
        };
        let mut scale = None;
        let mut tracks = None;
        let mut cues_offset = None;
        while reader.position()? < segment.data_end {
            checkpoint()?;
            let header = reader.read_header(segment.data_end, false)?;
            match header.id {
                SEEK_HEAD => {
                    if let Some(offset) = cue_index_offset(reader, header.data_end, checkpoint)? {
                        set_once(&mut cues_offset, offset)?;
                    }
                }
                INFO => set_once(&mut scale, reader.read_info(header.data_end)?)?,
                TRACKS => set_once(&mut tracks, reader.read_tracks(header.data_end)?)?,
                CLUSTER => break,
                CUES | VOID => reader.seek_to(header.data_end)?,
                _ => return Err("录屏回放遇到非生产顶层元素".to_string()),
            }
        }
        let scale = scale.ok_or("录屏回放缺少时间基")?;
        let expected = if audio.is_some() {
            AV_WEBM_TIMECODE_SCALE_NS
        } else {
            WEBM_TIMECODE_SCALE_NS
        };
        if scale != expected {
            return Err("录屏回放时间基不受支持".into());
        }
        let (video_track, audio_track) =
            validate_thumbnail_tracks(tracks.as_deref().ok_or("录屏回放缺少轨道")?, spec, audio)?;
        let target = segment_position(segment, cues_offset.ok_or("录屏回放缺少 Cues 索引")?)?;
        reader.seek_to(target)?;
        let cues = reader.read_header(segment.data_end, false)?;
        if cues.id != CUES {
            return Err("录屏回放 SeekHead 未指向 Cues".into());
        }
        Ok(Self {
            segment,
            cues,
            scale,
            video_track,
            audio_track,
            duration_ns,
        })
    }

    fn lookup(
        &self,
        reader: &mut StrictWebmReader,
        position_ns: u64,
        pre_roll_ns: u64,
        checkpoint: &mut impl FnMut() -> Result<(), String>,
    ) -> Result<(Cue, Cue), String> {
        let earliest_audio = position_ns.saturating_sub(pre_roll_ns);
        reader.seek_to(self.cues.data_start)?;
        let mut video = None;
        let mut audio = None;
        // 索引保留在磁盘上，长录屏的定位也只扫 CuePoint，不累积关键帧表。
        while reader.position()? < self.cues.data_end {
            checkpoint()?;
            let header = reader.read_header(self.cues.data_end, false)?;
            if header.id != CUE_POINT {
                reader.seek_to(header.data_end)?;
                continue;
            }
            let Some(cue) = self.read_cue(reader, header.data_end)? else {
                continue;
            };
            if let Some(previous) = video {
                let previous: Cue = previous;
                if cue.time_ns <= previous.time_ns {
                    return Err("录屏回放关键帧索引顺序无效".into());
                }
            } else if cue.time_ns != 0 {
                return Err("录屏回放索引缺少第一帧".into());
            }
            if cue.time_ns > position_ns {
                break;
            }
            video = Some(cue);
            if cue.time_ns <= earliest_audio {
                audio = Some(cue);
            }
        }
        let video = video.ok_or("录屏回放缺少可定位的关键帧")?;
        let audio = audio.expect("首个 CuePoint 覆盖零时刻");
        for cluster in [video.cluster, audio.cluster] {
            reader.seek_to(cluster)?;
            if reader.read_header(self.segment.data_end, false)?.id != CLUSTER {
                return Err("录屏回放 CuePoint 未指向 Cluster".into());
            }
            if video.cluster == audio.cluster {
                break;
            }
        }
        Ok((video, audio))
    }

    fn read_cue(&self, reader: &mut StrictWebmReader, end: u64) -> Result<Option<Cue>, String> {
        let mut ticks = None;
        let mut cluster = None;
        while reader.position()? < end {
            let header = reader.read_header(end, false)?;
            match header.id {
                CUE_TIME => set_once(&mut ticks, reader.read_uint(header)?)?,
                CUE_TRACK_POSITIONS => {
                    if let Some(action) = cue_position(reader, header.data_end, self.video_track)? {
                        set_once(&mut cluster, action)?;
                    }
                }
                _ => reader.seek_to(header.data_end)?,
            }
        }
        let Some(cluster) = cluster else {
            return Ok(None);
        };
        let time_ns = ticks
            .ok_or("录屏回放 CuePoint 缺少时间")?
            .checked_mul(self.scale)
            .filter(|time| *time < self.duration_ns)
            .ok_or("录屏回放 CueTime 越界")?;
        Ok(Some(Cue {
            time_ns,
            cluster: segment_position(self.segment, cluster)?,
        }))
    }

    fn next_packet(
        &self,
        reader: &mut StrictWebmReader,
        cursor: &mut Cursor,
        wanted: ThumbnailTrackKind,
        checkpoint: &mut impl FnMut() -> Result<(), String>,
    ) -> Result<Option<IndexedPacket>, String> {
        reader.seek_to(cursor.offset)?;
        while cursor.offset < self.segment.data_end {
            checkpoint()?;
            if cursor.offset >= cursor.cluster_end {
                let header = reader.read_header(self.segment.data_end, false)?;
                if header.id == CLUSTER {
                    cursor.cluster_end = header.data_end;
                    cursor.cluster_ticks = None;
                    cursor.offset = header.data_start;
                } else {
                    cursor.offset = header.data_end;
                    reader.seek_to(header.data_end)?;
                }
                continue;
            }
            let header = reader.read_header(cursor.cluster_end, false)?;
            cursor.offset = header.data_end;
            match header.id {
                CLUSTER_TIMECODE => {
                    set_once(&mut cursor.cluster_ticks, reader.read_uint(header)?)?;
                }
                SIMPLE_BLOCK => {
                    let ticks = cursor
                        .cluster_ticks
                        .ok_or("录屏回放 Block 缺少 Cluster 时间")?;
                    let block = read_block_prefix(reader, header, ticks, self.scale)?;
                    let kind = thumbnail_track_kind(
                        block.track_number,
                        self.video_track,
                        self.audio_track,
                    )?;
                    if kind == wanted && cursor.accepts(&block)? {
                        let limit = if kind == ThumbnailTrackKind::Audio {
                            if !block.keyframe {
                                return Err("录屏回放 Opus SimpleBlock 标志无效".into());
                            }
                            MAX_OPUS_PACKET_BYTES as u64
                        } else {
                            MAX_PACKET_BYTES
                        };
                        let payload = ElementHeader {
                            data_start: header.data_start + block.payload_offset as u64,
                            ..header
                        };
                        reader.seek_to(payload.data_start)?;
                        let data = reader.read_bounded_body(payload, limit)?;
                        return Ok(Some(IndexedPacket {
                            timestamp_ns: block.timestamp_ns,
                            discard_padding_ns: 0,
                            data,
                        }));
                    }
                    reader.seek_to(header.data_end)?;
                }
                BLOCK_GROUP if self.audio_track.is_some() => {
                    if wanted == ThumbnailTrackKind::Audio {
                        let ticks = cursor
                            .cluster_ticks
                            .ok_or("录屏回放 BlockGroup 缺少 Cluster 时间")?;
                        let (mut body, discard_padding_ns) = reader.read_opus_block_group(
                            header.data_end,
                            MAX_OPUS_PACKET_BYTES as u64 + 16,
                        )?;
                        let block = parse_simple_block(&body, ticks, self.scale)?;
                        if block.keyframe || Some(block.track_number) != self.audio_track {
                            return Err("录屏回放 BlockGroup 不是 Opus 尾包".into());
                        }
                        cursor.accepts(&block)?;
                        let data = body.split_off(block.payload_offset);
                        return Ok(Some(IndexedPacket {
                            timestamp_ns: block.timestamp_ns,
                            discard_padding_ns,
                            data,
                        }));
                    }
                    reader.seek_to(header.data_end)?;
                }
                CLUSTER_POSITION | PREVIOUS_CLUSTER_SIZE | VOID => {
                    reader.seek_to(header.data_end)?
                }
                _ => return Err("录屏回放 Cluster 含非生产元素".into()),
            }
        }
        if cursor.first_keyframe_ns.is_some() {
            return Err("录屏回放 CuePoint 找不到关键帧".into());
        }
        Ok(None)
    }
}

impl Cursor {
    fn accepts(&mut self, block: &ParsedSimpleBlock) -> Result<bool, String> {
        if let Some(keyframe) = self.first_keyframe_ns {
            if block.timestamp_ns < keyframe {
                return Ok(false);
            }
            if block.timestamp_ns != keyframe || !block.keyframe {
                return Err("录屏回放 CuePoint 与关键帧不一致".into());
            }
            self.first_keyframe_ns = None;
        }
        if self
            .previous_packet_ns
            .is_some_and(|previous| block.timestamp_ns <= previous)
        {
            return Err("录屏回放轨道时间倒退或重复".into());
        }
        self.previous_packet_ns = Some(block.timestamp_ns);
        Ok(true)
    }
}

fn read_block_prefix(
    reader: &mut StrictWebmReader,
    header: ElementHeader,
    cluster_ticks: u64,
    scale: u64,
) -> Result<ParsedSimpleBlock, String> {
    // 预读最多 8 字节轨道号、3 字节时间和标志、1 字节 payload，不给另一轨分配 packet。
    let mut prefix = [0; 12];
    let count = (header.data_end - header.data_start).min(prefix.len() as u64) as usize;
    reader
        .reader
        .read_exact(&mut prefix[..count])
        .map_err(|error| error.to_string())?;
    parse_simple_block(&prefix[..count], cluster_ticks, scale)
}

fn segment_position(segment: ElementHeader, relative: u64) -> Result<u64, String> {
    segment
        .data_start
        .checked_add(relative)
        .filter(|position| *position < segment.data_end)
        .ok_or_else(|| "录屏回放索引位置越界".to_string())
}

fn cue_index_offset(
    reader: &mut StrictWebmReader,
    end: u64,
    checkpoint: &mut impl FnMut() -> Result<(), String>,
) -> Result<Option<u64>, String> {
    let mut offset = None;
    while reader.position()? < end {
        checkpoint()?;
        let header = reader.read_header(end, false)?;
        if header.id != SEEK {
            reader.seek_to(header.data_end)?;
            continue;
        }
        let (id, location) = pair(reader, header.data_end, SEEK_ID, SEEK_POSITION)?;
        if id == Some(u64::from(CUES)) {
            set_once(
                &mut offset,
                location.ok_or("录屏回放 Cues 缺少 SeekPosition")?,
            )?;
        }
    }
    Ok(offset)
}

fn cue_position(
    reader: &mut StrictWebmReader,
    end: u64,
    video_track: u64,
) -> Result<Option<u64>, String> {
    let (track, cluster) = pair(reader, end, CUE_TRACK, CUE_CLUSTER_POSITION)?;
    if track == Some(video_track) {
        return cluster
            .map(Some)
            .ok_or_else(|| "录屏回放视频 CuePoint 缺少位置".to_string());
    }
    Ok(None)
}

fn pair(
    reader: &mut StrictWebmReader,
    end: u64,
    first: u32,
    second: u32,
) -> Result<(Option<u64>, Option<u64>), String> {
    let mut values = (None, None);
    while reader.position()? < end {
        let header = reader.read_header(end, false)?;
        match header.id {
            id if id == first => set_once(&mut values.0, reader.read_uint(header)?)?,
            id if id == second => set_once(&mut values.1, reader.read_uint(header)?)?,
            _ => reader.seek_to(header.data_end)?,
        }
    }
    Ok(values)
}
