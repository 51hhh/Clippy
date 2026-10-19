use super::*;
use crate::recording::mux::webm_remux::{
    visit_vp9_opus_packets, ParsedAvPacket, WebmAvRemuxSource, WebmRemuxSource, WebmRemuxSpec,
    WebmThumbnailAudioSpec,
};

fn cfr_writer(
    root: &std::path::Path,
    id: &str,
    fps: u32,
    segment_ns: u64,
) -> (SegmentedAvRecordingWriter, PathBuf) {
    let audio = OpusPacketEncoder::new(2).unwrap();
    let mut config = journal_config(id, audio.track_config());
    config.target_fps_numerator = fps;
    let journal = RecordingJournal::create(root, config).unwrap();
    let session = journal.session_directory().to_path_buf();
    (
        SegmentedAvRecordingWriter::new(
            journal,
            Arc::new(RecordingPipeline::default()),
            2,
            2,
            fps,
            2,
            segment_ns,
            audio,
        )
        .unwrap(),
        session,
    )
}

fn read_artifact(session: &std::path::Path, value: &Value, artifact: &Value, fps: u32) {
    let track = &value["audio"];
    let audio = &artifact["audio"];
    let source = WebmAvRemuxSource {
        source: WebmRemuxSource {
            path: session.join(artifact["fileName"].as_str().unwrap()),
            byte_length: artifact["byteLength"].as_u64().unwrap(),
            sha256: artifact["sha256"].as_str().unwrap().to_string(),
            started_at_ns: artifact["startedAtNs"].as_u64().unwrap_or(0),
            duration_ns: artifact["durationNs"].as_u64().unwrap(),
            frame_count: artifact["frameCount"].as_u64().unwrap(),
        },
        audio: WebmThumbnailAudioSpec {
            sample_rate_hz: 48_000,
            channels: 2,
            pre_skip_frames: track["preSkipFrames"].as_u64().unwrap() as u16,
            codec_delay_ns: track["codecDelayNs"].as_u64().unwrap(),
            seek_pre_roll_ns: track["seekPreRollNs"].as_u64().unwrap(),
            packet_count: audio["packetCount"].as_u64().unwrap(),
            pcm_frame_count: audio["pcmFrameCount"].as_u64().unwrap(),
        },
    };
    let mut first_video = None;
    let stats = visit_vp9_opus_packets(
        WebmRemuxSpec {
            width: 2,
            height: 2,
            fps_numerator: fps,
            fps_denominator: 1,
        },
        &source,
        |packet| {
            if let ParsedAvPacket::Video {
                timestamp_ns,
                keyframe,
                ..
            } = packet
            {
                first_video.get_or_insert((timestamp_ns, keyframe));
            }
            Ok(())
        },
    )
    .expect("真实分段必须通过原有严格 WebM packet reader");
    assert_eq!(first_video, Some((0, true)));
    assert_eq!(stats.video_frame_count, source.source.frame_count);
    assert_eq!(stats.audio_packet_count, source.audio.packet_count);
    let mut forged = source.clone();
    forged.source.frame_count += 1;
    assert!(visit_vp9_opus_packets(
        WebmRemuxSpec {
            width: 2,
            height: 2,
            fps_numerator: fps,
            fps_denominator: 1
        },
        &forged,
        |_| Ok(())
    )
    .is_err());
    let mut forged_audio = source.clone();
    forged_audio.audio.pcm_frame_count += 1;
    assert!(visit_vp9_opus_packets(
        WebmRemuxSpec {
            width: 2,
            height: 2,
            fps_numerator: fps,
            fps_denominator: 1
        },
        &forged_audio,
        |_| Ok(())
    )
    .is_err());
}

fn complete_cfr(
    output: SegmentedAvRecordingOutput,
    session: &std::path::Path,
    fps: u32,
    duration_ns: u64,
    video_frames: u64,
    pcm_frames: u64,
    boundaries: &[u64],
) {
    assert_eq!(output.duration_ns, duration_ns);
    assert_eq!(output.video_frame_count, video_frames);
    assert_eq!(output.audio_pcm_frame_count, pcm_frames);
    output.completion.complete().unwrap();
    let value = manifest(session);
    assert_eq!(value["state"], "complete");
    let segments = value["segments"].as_array().unwrap();
    assert_eq!(segments.len(), boundaries.len() + 1);
    let mut start = 0;
    for (segment, end) in segments
        .iter()
        .zip(boundaries.iter().copied().chain([duration_ns]))
    {
        assert_eq!(segment["startedAtNs"], start);
        assert_eq!(segment["durationNs"], end - start);
        read_artifact(session, &value, segment, fps);
        start = end;
    }
    assert_eq!(
        segments
            .iter()
            .map(|s| s["frameCount"].as_u64().unwrap())
            .sum::<u64>(),
        video_frames
    );
    assert_eq!(
        segments
            .iter()
            .map(|s| s["audio"]["pcmFrameCount"].as_u64().unwrap())
            .sum::<u64>(),
        pcm_frames
    );
    read_artifact(session, &value, &value["finalOutput"], fps);
}

#[test]
fn native_220ms_frame_rotates_at_200ms_cfr_boundary() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut writer, session) = cfr_writer(temporary.path(), "cfr-ten", 10, 200_000_000);
    writer.push_video(&rgba(1), 0).unwrap();
    writer.push_audio(pcm(0, 4_800)).unwrap();
    writer.push_video(&rgba(2), 110_000_000).unwrap();
    writer.push_audio(pcm(5_280, 4_320)).unwrap();
    writer.push_video(&rgba(3), 220_000_000).unwrap();
    writer.push_audio(pcm(9_600, 4_800)).unwrap();
    writer.push_video(&rgba(4), 330_000_000).unwrap();
    writer.push_audio(pcm(15_840, 3_360)).unwrap();
    let output = writer.finish(440_000_000).unwrap();
    complete_cfr(output, &session, 10, 440_000_000, 5, 21_120, &[200_000_000]);
}

#[test]
fn native_105ms_frame_at_30fps_keeps_audio_and_video_segment_origin_equal() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut writer, session) = cfr_writer(temporary.path(), "cfr-thirty", 30, 100_000_000);
    writer.push_video(&rgba(1), 0).unwrap();
    writer.push_audio(pcm(0, 4_800)).unwrap();
    writer.push_video(&rgba(2), 105_000_000).unwrap();
    writer.push_audio(pcm(4_800, 4_800)).unwrap();
    let output = writer.finish(205_000_000).unwrap();
    complete_cfr(output, &session, 30, 205_000_000, 7, 9_840, &[100_000_000]);
}

#[test]
fn fifteen_fps_adjacent_cfr_cuts_remain_strictly_readable() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut writer, session) = cfr_writer(temporary.path(), "cfr-fifteen", 15, 50_000_000);
    writer.push_video(&rgba(1), 0).unwrap();
    writer.push_audio(pcm(0, 3_200)).unwrap();
    writer.push_video(&rgba(2), 70_000_000).unwrap();
    writer.push_audio(pcm(3_200, 3_200)).unwrap();
    writer.push_video(&rgba(3), 140_000_000).unwrap();
    writer.push_audio(pcm(6_400, 3_200)).unwrap();
    writer.push_video(&rgba(4), 210_000_000).unwrap();
    writer.push_audio(pcm(9_600, 1_920)).unwrap();
    let output = writer.finish(240_000_000).unwrap();
    complete_cfr(
        output,
        &session,
        15,
        240_000_000,
        4,
        11_520,
        &[66_666_666, 133_333_333, 200_000_000],
    );
}

#[test]
fn same_slot_native_frame_defers_rotation_until_cfr_progress_exists() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut writer, session) = cfr_writer(temporary.path(), "cfr-one", 1, 200_000_000);
    writer.push_video(&rgba(1), 0).unwrap();
    writer.push_audio(pcm(0, 4_800)).unwrap();
    writer.push_video(&rgba(2), 250_000_000).unwrap();
    writer.push_audio(pcm(12_000, 4_800)).unwrap();
    writer.push_video(&rgba(3), 1_250_000_000).unwrap();
    writer.push_audio(pcm(48_000, 4_800)).unwrap();
    let output = writer.finish(2_000_000_000).unwrap();
    complete_cfr(
        output,
        &session,
        1,
        2_000_000_000,
        2,
        96_000,
        &[1_000_000_000],
    );
}
