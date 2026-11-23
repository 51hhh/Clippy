//! schema v2 异常录屏的连续双轨恢复。
//!
//! WebM/EBML 的严格读取留在 `mux::webm_remux`；本层只负责每段 Opus decoder 生命周期、真实 PCM
//! 裁边、连续 Opus encoder 和有界 A/V interleave。这样容器校验与恢复策略各自只有一个职责。

use super::audio::{
    frames_to_ns, AudioFormat, CapturedAudioChunk, QueuedAudioChunk, AUDIO_SAMPLE_RATE_HZ,
};
use super::mux::{
    av_interleaver::AvPacketInterleaver,
    opus_webm::{AvWebmOutput, OpusPacketEncoder, OpusTrackConfig},
    webm_remux::{
        opus_head, visit_vp9_opus_packets, ParsedAvPacket, WebmAvRemuxSource, WebmRemuxSpec,
        WebmThumbnailAudioSpec, OPUS_PACKET_FRAMES,
    },
};
use opusic_c::{Channels, Decoder, SampleRate};
use std::io::{Seek, Write};

#[derive(Debug)]
pub(in crate::recording) struct WebmAvRemuxOutput<W> {
    pub writer: W,
    pub video_frame_count: u64,
    pub audio_packet_count: u64,
    pub audio_pcm_frame_count: u64,
    pub duration_ns: u64,
}

/// 把 schema v2 的独立 VP9 + Opus 恢复分段合成为一条连续双轨 WebM。
///
/// VP9 packet 直接复用。每段 Opus 都由独立 encoder 产生，必须用独立 decoder 应用该段的
/// pre-skip/DiscardPadding，再把真实 PCM 送进一个贯穿输出的 encoder；否则中间分段会继承错误的
/// decoder state，也无法表达第二次及之后的 pre-skip。
pub(in crate::recording) fn remux_vp9_opus_segments<W: Write + Seek>(
    sources: &[WebmAvRemuxSource],
    output: W,
    spec: WebmRemuxSpec,
) -> Result<WebmAvRemuxOutput<W>, String> {
    let (expected_duration_ns, expected_video_frames, expected_audio_frames, audio_spec) =
        validate_av_sources(sources, spec)?;
    let mut audio_encoder =
        OpusPacketEncoder::new(audio_spec.channels).map_err(av_remux_opus_error)?;
    validate_output_opus_track(audio_encoder.track_config(), audio_spec)?;
    let mut interleaver = AvPacketInterleaver::new(
        output,
        spec.width,
        spec.height,
        audio_encoder.track_config(),
    )
    .map_err(av_remux_opus_error)?;
    let mut output_audio_frames = 0_u64;
    let mut observed_video_frames = 0_u64;

    for source in sources {
        let opus_channels = match source.audio.channels {
            1 => Channels::Mono,
            2 => Channels::Stereo,
            _ => return Err("录屏恢复 Opus 声道数无效".to_string()),
        };
        let mut decoder =
            Decoder::new(opus_channels, SampleRate::Hz48000).map_err(av_remux_decoder_error)?;
        let mut pre_skip_remaining = u64::from(source.audio.pre_skip_frames);
        let mut real_frames_remaining = source.audio.pcm_frame_count;
        let mut decoded_packet_count = 0_u64;
        let parsed = visit_vp9_opus_packets(spec, source, |packet| {
            let local_timestamp_ns = match packet {
                ParsedAvPacket::Video { timestamp_ns, .. }
                | ParsedAvPacket::Audio { timestamp_ns, .. } => timestamp_ns,
            };
            let scan_frontier_ns = source
                .source
                .started_at_ns
                .checked_add(local_timestamp_ns)
                .ok_or_else(|| "录屏恢复双轨 packet 时间戳溢出".to_string())?;
            match packet {
                ParsedAvPacket::Video {
                    data,
                    timestamp_ns,
                    keyframe,
                } => {
                    let global_timestamp_ns = source
                        .source
                        .started_at_ns
                        .checked_add(timestamp_ns)
                        .ok_or_else(|| "录屏恢复视频 packet 时间戳溢出".to_string())?;
                    interleaver
                        .enqueue_video(data, global_timestamp_ns, keyframe)
                        .map_err(av_remux_opus_error)?;
                    observed_video_frames = observed_video_frames
                        .checked_add(1)
                        .ok_or_else(|| "录屏恢复视频帧数溢出".to_string())?;
                }
                ParsedAvPacket::Audio {
                    data,
                    discard_padding_ns,
                    ..
                } => {
                    if discard_padding_ns > 0
                        && decoded_packet_count + 1 != source.audio.packet_count
                    {
                        return Err("录屏恢复 Opus padding 不在末包".to_string());
                    }
                    let mut decoded =
                        vec![0.0_f32; OPUS_PACKET_FRAMES * usize::from(audio_spec.channels)];
                    let decoded_frames = decoder
                        .decode_float_to_slice(data, &mut decoded, false)
                        .map_err(av_remux_decoder_error)?;
                    if decoded_frames != OPUS_PACKET_FRAMES {
                        return Err("录屏恢复 Opus packet 时长不受支持".to_string());
                    }
                    decoded_packet_count = decoded_packet_count
                        .checked_add(1)
                        .ok_or_else(|| "录屏恢复 Opus packet 数溢出".to_string())?;

                    let skip_frames = pre_skip_remaining.min(decoded_frames as u64);
                    pre_skip_remaining -= skip_frames;
                    let available_frames = decoded_frames as u64 - skip_frames;
                    let take_frames = available_frames.min(real_frames_remaining);
                    if take_frames > 0 {
                        let channels = usize::from(audio_spec.channels);
                        let sample_start = usize::try_from(skip_frames)
                            .ok()
                            .and_then(|frames| frames.checked_mul(channels))
                            .ok_or_else(|| "录屏恢复 PCM 起点溢出".to_string())?;
                        let sample_end = usize::try_from(take_frames)
                            .ok()
                            .and_then(|frames| frames.checked_mul(channels))
                            .and_then(|samples| sample_start.checked_add(samples))
                            .ok_or_else(|| "录屏恢复 PCM 长度溢出".to_string())?;
                        enqueue_recovered_pcm(
                            &mut audio_encoder,
                            &mut interleaver,
                            audio_spec.channels,
                            &decoded[sample_start..sample_end],
                            take_frames,
                            &mut output_audio_frames,
                        )?;
                        real_frames_remaining -= take_frames;
                    }
                }
            }
            let audio_frontier_ns = audio_encoder
                .next_packet_timestamp_ns(0)
                .map_err(av_remux_opus_error)?;
            interleaver
                .flush_ready(scan_frontier_ns, audio_frontier_ns)
                .map_err(av_remux_opus_error)
        })?;
        if parsed.video_frame_count != source.source.frame_count
            || parsed.audio_packet_count != source.audio.packet_count
            || decoded_packet_count != source.audio.packet_count
            || pre_skip_remaining != 0
            || real_frames_remaining != 0
        {
            return Err("录屏恢复双轨解码统计与清单不一致".to_string());
        }
        let segment_end_ns = source
            .source
            .started_at_ns
            .checked_add(source.source.duration_ns)
            .ok_or_else(|| "录屏恢复分段终点溢出".to_string())?;
        let audio_frontier_ns = audio_encoder
            .next_packet_timestamp_ns(0)
            .map_err(av_remux_opus_error)?;
        interleaver
            .flush_ready(segment_end_ns, audio_frontier_ns)
            .map_err(av_remux_opus_error)?;
    }

    if observed_video_frames != expected_video_frames
        || output_audio_frames != expected_audio_frames
    {
        return Err("录屏恢复双轨汇总与清单不一致".to_string());
    }
    let audio_finish = audio_encoder.finish().map_err(av_remux_opus_error)?;
    if audio_finish.stats.real_frames != expected_audio_frames
        || audio_finish.stats.media_start_ns != 0
    {
        return Err("录屏恢复连续 Opus 统计与清单不一致".to_string());
    }
    for packet in audio_finish.packets {
        interleaver
            .enqueue_audio(packet)
            .map_err(av_remux_opus_error)?;
    }
    let AvWebmOutput {
        writer,
        video_frame_count,
        audio_packet_count,
        duration_ns,
    } = interleaver
        .finish(expected_duration_ns)
        .map_err(av_remux_opus_error)?;
    if video_frame_count != expected_video_frames
        || audio_packet_count != audio_finish.stats.packet_count
        || duration_ns != expected_duration_ns
    {
        return Err("录屏恢复双轨 mux 统计与清单不一致".to_string());
    }
    Ok(WebmAvRemuxOutput {
        writer,
        video_frame_count,
        audio_packet_count,
        audio_pcm_frame_count: expected_audio_frames,
        duration_ns,
    })
}

fn validate_av_sources(
    sources: &[WebmAvRemuxSource],
    spec: WebmRemuxSpec,
) -> Result<(u64, u64, u64, WebmThumbnailAudioSpec), String> {
    if sources.is_empty()
        || spec.width == 0
        || spec.height == 0
        || spec.fps_numerator == 0
        || spec.fps_denominator == 0
    {
        return Err("录屏双轨恢复合并参数无效".to_string());
    }
    let first_audio = sources[0].audio;
    if first_audio.sample_rate_hz != AUDIO_SAMPLE_RATE_HZ
        || !matches!(first_audio.channels, 1 | 2)
        || first_audio.pre_skip_frames == 0
        || first_audio.packet_count == 0
        || first_audio.pcm_frame_count == 0
    {
        return Err("录屏双轨恢复音轨参数无效".to_string());
    }
    let mut expected_start_ns = 0_u64;
    let mut expected_video_frames = 0_u64;
    let mut expected_audio_frames = 0_u64;
    for source in sources {
        if source.source.started_at_ns != expected_start_ns
            || source.source.duration_ns == 0
            || source.audio.sample_rate_hz != first_audio.sample_rate_hz
            || source.audio.channels != first_audio.channels
            || source.audio.pre_skip_frames != first_audio.pre_skip_frames
            || source.audio.codec_delay_ns != first_audio.codec_delay_ns
            || source.audio.seek_pre_roll_ns != first_audio.seek_pre_roll_ns
            || source.audio.packet_count == 0
            || source.audio.pcm_frame_count == 0
        {
            return Err("录屏双轨恢复分段配置或时间线不连续".to_string());
        }
        expected_start_ns = expected_start_ns
            .checked_add(source.source.duration_ns)
            .ok_or_else(|| "录屏双轨恢复时间线溢出".to_string())?;
        expected_video_frames = expected_video_frames
            .checked_add(source.source.frame_count)
            .ok_or_else(|| "录屏双轨恢复视频帧数溢出".to_string())?;
        expected_audio_frames = expected_audio_frames
            .checked_add(source.audio.pcm_frame_count)
            .ok_or_else(|| "录屏双轨恢复 PCM 帧数溢出".to_string())?;
        if expected_audio_frames != timestamp_to_audio_frame(expected_start_ns)? {
            return Err("录屏双轨恢复 PCM 时间线与分段边界不一致".to_string());
        }
    }
    Ok((
        expected_start_ns,
        expected_video_frames,
        expected_audio_frames,
        first_audio,
    ))
}

fn validate_output_opus_track(
    track: &OpusTrackConfig,
    audio: WebmThumbnailAudioSpec,
) -> Result<(), String> {
    let expected_head = opus_head(audio.channels, audio.pre_skip_frames, audio.sample_rate_hz)?;
    if track.channels != audio.channels
        || track.pre_skip_frames != audio.pre_skip_frames
        || track.codec_delay_ns != audio.codec_delay_ns
        || track.seek_pre_roll_ns != audio.seek_pre_roll_ns
        || track.opus_head != expected_head
    {
        return Err("录屏恢复输出 Opus 配置与清单不一致".to_string());
    }
    Ok(())
}

fn enqueue_recovered_pcm<W: Write + Seek>(
    encoder: &mut OpusPacketEncoder,
    interleaver: &mut AvPacketInterleaver<W>,
    channels: u16,
    samples: &[f32],
    frame_count: u64,
    frame_cursor: &mut u64,
) -> Result<(), String> {
    let frame_count_u32 =
        u32::try_from(frame_count).map_err(|_| "录屏恢复 PCM 块帧数溢出".to_string())?;
    let presentation_at_ns = audio_frames_to_ns(*frame_cursor)?;
    let queued = QueuedAudioChunk {
        chunk: CapturedAudioChunk {
            sequence: *frame_cursor,
            captured_at_ns: presentation_at_ns,
            format: AudioFormat::normalized(channels),
            frame_count: frame_count_u32,
            samples: samples.to_vec().into_boxed_slice(),
        },
        presentation_at_ns,
        duration_ns: frames_to_ns(frame_count_u32)
            .map_err(|error| format!("录屏恢复 PCM 时长无效: {error}"))?,
        gap_before_ns: 0,
    };
    for packet in encoder.push(queued).map_err(av_remux_opus_error)? {
        interleaver
            .enqueue_audio(packet)
            .map_err(av_remux_opus_error)?;
    }
    *frame_cursor = frame_cursor
        .checked_add(frame_count)
        .ok_or_else(|| "录屏恢复 PCM 帧游标溢出".to_string())?;
    Ok(())
}

fn timestamp_to_audio_frame(timestamp_ns: u64) -> Result<u64, String> {
    u128::from(timestamp_ns)
        .checked_mul(u128::from(AUDIO_SAMPLE_RATE_HZ))
        .and_then(|value| value.checked_add(500_000_000))
        .and_then(|value| value.checked_div(1_000_000_000))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| "录屏恢复音频时间戳溢出".to_string())
}

fn audio_frames_to_ns(frames: u64) -> Result<u64, String> {
    u128::from(frames)
        .checked_mul(1_000_000_000)
        .and_then(|value| value.checked_div(u128::from(AUDIO_SAMPLE_RATE_HZ)))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| "录屏恢复音频帧时间溢出".to_string())
}

fn av_remux_opus_error(error: impl std::fmt::Display) -> String {
    format!("录屏双轨恢复编码或封装失败: {error}")
}

fn av_remux_decoder_error(error: opusic_c::ErrorCode) -> String {
    format!("录屏双轨恢复 Opus 解码失败: {error:?}: {}", error.message())
}
