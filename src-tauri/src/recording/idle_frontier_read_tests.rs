use crate::recording::mux::webm_remux::{
    visit_vp9_opus_packets, ParsedAvPacket, WebmAvRemuxSource, WebmRemuxSource, WebmRemuxSpec,
    WebmThumbnailAudioSpec,
};

fn read_idle_artifact(session: &std::path::Path, manifest: &Value, artifact: &Value, fps: u32) {
    let track = &manifest["audio"];
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
    let mut first = None;
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
                first.get_or_insert((timestamp_ns, keyframe));
            }
            Ok(())
        },
    )
    .expect("空闲排空的真实文件必须通过原严格 reader 与哈希/顺序/统计检查");
    assert_eq!(first, Some((0, true)));
    assert_eq!(stats.video_frame_count, source.source.frame_count);
    assert_eq!(stats.audio_packet_count, source.audio.packet_count);
}

fn read_idle_complete(session: &std::path::Path, value: &Value, fps: u32, duration: u64) {
    assert_eq!(value["state"], "complete");
    let mut start = 0;
    let mut frames = 0;
    let mut pcm = 0;
    for segment in value["segments"].as_array().unwrap() {
        assert_eq!(segment["startedAtNs"], start);
        start += segment["durationNs"].as_u64().unwrap();
        frames += segment["frameCount"].as_u64().unwrap();
        pcm += segment["audio"]["pcmFrameCount"].as_u64().unwrap();
        read_idle_artifact(session, value, segment, fps);
    }
    assert_eq!(start, duration);
    assert_eq!(value["finalOutput"]["frameCount"], frames);
    assert_eq!(value["finalOutput"]["audio"]["pcmFrameCount"], pcm);
    read_idle_artifact(session, value, &value["finalOutput"], fps);
}
