use super::{
    audio::{AUDIO_SAMPLE_RATE_HZ, DEFAULT_OPUS_FRAME_MS},
    manifest::{self, RecordingThumbnailSource},
    mux::webm_remux::{
        expected_opus_padding_ns,
        playback_index::{IndexedPacket, IndexedWebm},
        validate_audio_timestamp, WebmRemuxSource, WebmRemuxSpec, WebmThumbnailAudioSpec,
        OPUS_PACKET_FRAMES,
    },
    thumbnail::{vp9_frame_rgba, MAX_DECODE_PIXELS},
};
use image::RgbaImage;
use opusic_c::{Channels, Decoder as OpusDecoder, SampleRate};
use shiguredo_libvpx::{Decoder, DecoderCodec, DecoderConfig};
use std::path::Path;

const NS_PER_SECOND: u64 = 1_000_000_000;
const PACKET_NS: u64 = DEFAULT_OPUS_FRAME_MS as u64 * 1_000_000;

pub struct VideoFrame {
    pub timestamp_ns: u64,
    pub image: RgbaImage,
}

pub struct PcmBlock {
    pub first_frame: u64,
    pub samples: Box<[f32]>,
}

pub struct NativeWebmDecoder {
    media: IndexedWebm,
    video: Decoder,
    pending_video: Option<IndexedPacket>,
    audio: Option<OpusStream>,
    duration_ns: u64,
    width: u32,
    height: u32,
}

impl NativeWebmDecoder {
    pub fn open_library_artifact(
        data_root: &Path,
        session: &str,
        artifact: &str,
        checkpoint: impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        let source = manifest::resolve_library_playback_source(data_root, session, artifact)?;
        Self::open(source, checkpoint)
    }

    fn open(
        source: RecordingThumbnailSource,
        mut checkpoint: impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        let RecordingThumbnailSource {
            artifact,
            width,
            height,
            target_fps_numerator,
            target_fps_denominator,
            duration_ns,
            frame_count,
            audio,
        } = source;
        if u64::from(width) * u64::from(height) > MAX_DECODE_PIXELS {
            return Err("录屏回放超过解码像素上限".into());
        }
        let audio_spec = audio.map(|audio| WebmThumbnailAudioSpec {
            channels: audio.channels,
            sample_rate_hz: audio.sample_rate_hz,
            pre_skip_frames: audio.pre_skip_frames,
            codec_delay_ns: audio.codec_delay_ns,
            seek_pre_roll_ns: audio.seek_pre_roll_ns,
            packet_count: audio.packet_count,
            pcm_frame_count: audio.pcm_frame_count,
        });
        let mut media = IndexedWebm::open(
            &WebmRemuxSource {
                path: artifact.path,
                byte_length: artifact.byte_length,
                sha256: artifact.sha256,
                started_at_ns: 0,
                duration_ns,
                frame_count,
            },
            WebmRemuxSpec {
                width,
                height,
                fps_numerator: target_fps_numerator,
                fps_denominator: target_fps_denominator,
            },
            audio_spec,
            &mut checkpoint,
        )?;
        let pending_video = media.next_video(&mut checkpoint)?;
        Ok(Self {
            media,
            pending_video,
            video: vp9_decoder()?,
            duration_ns,
            width,
            height,
            audio: audio_spec
                .map(|audio| OpusStream::new(audio, 0))
                .transpose()?,
        })
    }

    pub fn duration_ns(&self) -> u64 {
        self.duration_ns
    }

    pub fn audio_channels(&self) -> Option<u16> {
        self.audio.as_ref().map(|audio| audio.spec.channels)
    }

    pub fn seek(
        &mut self,
        position_ns: u64,
        mut checkpoint: impl FnMut() -> Result<(), String>,
    ) -> Result<u64, String> {
        let position_ns = position_ns.min(self.duration_ns - 1);
        let pre_roll_ns = self
            .audio
            .as_ref()
            .map_or(0, |audio| audio.spec.seek_pre_roll_ns + PACKET_NS);
        self.media.seek(position_ns, pre_roll_ns, &mut checkpoint)?;
        self.pending_video = self.media.next_video(&mut checkpoint)?;
        self.video = vp9_decoder()?;
        if let Some(audio) = self.audio.as_mut() {
            *audio = OpusStream::new(audio.spec, position_ns)?;
        }
        Ok(position_ns)
    }

    pub fn video_through(
        &mut self,
        position_ns: u64,
        mut checkpoint: impl FnMut() -> Result<(), String>,
    ) -> Result<Option<VideoFrame>, String> {
        while self
            .pending_video
            .as_ref()
            .is_some_and(|packet| packet.timestamp_ns <= position_ns)
        {
            checkpoint()?;
            let packet = self.pending_video.take().expect("待解码录像帧已就绪");
            self.pending_video = self.media.next_video(&mut checkpoint)?;
            let last_due = self
                .pending_video
                .as_ref()
                .is_none_or(|next| next.timestamp_ns > position_ns);
            self.video
                .decode(&packet.data)
                .map_err(|error| error.to_string())?;
            let decoded = self
                .video
                .next_frame()
                .map_err(|error| error.to_string())?
                .ok_or("VP9 packet 未输出录像画面")?;
            let image = last_due
                .then(|| vp9_frame_rgba(&decoded, self.width, self.height))
                .transpose()?;
            if self
                .video
                .next_frame()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return Err("VP9 packet 输出了额外录像画面".into());
            }
            if let Some(image) = image {
                return Ok(Some(VideoFrame {
                    timestamp_ns: packet.timestamp_ns,
                    image,
                }));
            }
        }
        Ok(None)
    }

    pub fn next_pcm(
        &mut self,
        mut checkpoint: impl FnMut() -> Result<(), String>,
    ) -> Result<Option<PcmBlock>, String> {
        let Some(audio) = self.audio.as_mut() else {
            return Ok(None);
        };
        loop {
            checkpoint()?;
            let Some(packet) = self.media.next_audio(&mut checkpoint)? else {
                if audio.next_packet_index == Some(audio.spec.packet_count) {
                    return Ok(None);
                }
                return Err("录屏回放 Opus 数据早于清单结束".into());
            };
            if let Some(block) = audio.decode(packet)? {
                return Ok(Some(block));
            }
        }
    }
}

fn vp9_decoder() -> Result<Decoder, String> {
    Decoder::new(DecoderConfig::new(DecoderCodec::Vp9)).map_err(|error| error.to_string())
}

struct OpusStream {
    spec: WebmThumbnailAudioSpec,
    decoder: OpusDecoder,
    next_packet_index: Option<u64>,
    first_frame: u64,
    padding_ns: u64,
    decoded: Vec<f32>,
}

impl OpusStream {
    fn new(spec: WebmThumbnailAudioSpec, position_ns: u64) -> Result<Self, String> {
        let channels = match spec.channels {
            1 => Channels::Mono,
            2 => Channels::Stereo,
            _ => return Err("录屏回放 Opus 声道数不受支持".into()),
        };
        let first_frame = (u128::from(position_ns) * u128::from(AUDIO_SAMPLE_RATE_HZ))
            .div_ceil(u128::from(NS_PER_SECOND)) as u64;
        Ok(Self {
            spec,
            decoder: OpusDecoder::new(channels, SampleRate::Hz48000)
                .map_err(|error| format!("创建回放 Opus 解码器失败: {error:?}"))?,
            first_frame,
            next_packet_index: None,
            padding_ns: expected_opus_padding_ns(spec)?,
            decoded: vec![0.0; OPUS_PACKET_FRAMES * usize::from(spec.channels)],
        })
    }

    fn decode(&mut self, packet: IndexedPacket) -> Result<Option<PcmBlock>, String> {
        // 双轨 WebM 以 0.5 毫秒量化 PTS，采样位置按固定 20 毫秒包序号恢复。
        let approximate = packet.timestamp_ns.saturating_sub(self.spec.codec_delay_ns);
        let index = approximate / PACKET_NS + u64::from(approximate % PACKET_NS >= PACKET_NS / 2);
        validate_audio_timestamp(packet.timestamp_ns, index, self.spec)?;
        if index >= self.spec.packet_count
            || self.next_packet_index.is_some_and(|next| next != index)
        {
            return Err("录屏回放 Opus packet 序号不连续".into());
        }
        self.next_packet_index = Some(index + 1);
        let padding = if index + 1 == self.spec.packet_count {
            self.padding_ns
        } else {
            0
        };
        if packet.discard_padding_ns != padding {
            return Err("录屏回放 Opus 尾填充与清单不一致".into());
        }
        let count = self
            .decoder
            .decode_float_to_slice(&packet.data, &mut self.decoded, false)
            .map_err(|error| format!("解码回放 Opus packet 失败: {error:?}"))?;
        if count != OPUS_PACKET_FRAMES {
            return Err("录屏回放 Opus packet 采样数不正确".into());
        }
        let start = index * OPUS_PACKET_FRAMES as u64;
        let pre_skip = u64::from(self.spec.pre_skip_frames);
        let trim_left = start.max(pre_skip + self.first_frame);
        let trim_right = (start + count as u64).min(pre_skip + self.spec.pcm_frame_count);
        if trim_left >= trim_right {
            return Ok(None);
        }
        let channels = usize::from(self.spec.channels);
        let samples = &self.decoded
            [(trim_left - start) as usize * channels..(trim_right - start) as usize * channels];
        Ok(Some(PcmBlock {
            first_frame: trim_left - pre_skip,
            samples: samples.into(),
        }))
    }
}
