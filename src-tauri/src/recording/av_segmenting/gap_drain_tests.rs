use super::*;

mod extra_tests {
    include!("gap_drain_extra_tests.rs");
}

fn complete_gap_output(
    output: SegmentedAvRecordingOutput,
    session: &std::path::Path,
    duration_ns: u64,
    video_frames: u64,
    pcm_frames: u64,
    segments: usize,
) {
    assert_eq!(output.duration_ns, duration_ns);
    assert_eq!(output.video_frame_count, video_frames);
    assert_eq!(output.audio_pcm_frame_count, pcm_frames);
    assert!(output.audio_packet_count > 32);
    output.completion.complete().unwrap();
    let value = manifest(session);
    assert_eq!(value["state"], "complete");
    assert_eq!(value["segments"].as_array().unwrap().len(), segments);
    assert_eq!(value["finalOutput"]["durationNs"], duration_ns);
    assert_eq!(value["finalOutput"]["audio"]["pcmFrameCount"], pcm_frames);
    let counted: u64 = value["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|segment| segment["audio"]["pcmFrameCount"].as_u64().unwrap())
        .sum();
    assert_eq!(counted, pcm_frames);
}

#[test]
fn long_stop_tail_drains_video_and_silence_with_bounded_packets() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut writer, session) = writer(temporary.path(), "av-long-tail", 10_000_000_000);
    writer.push_video(&rgba(1), 0).unwrap();
    writer.push_audio(pcm(0, 4_800)).unwrap();
    let output = writer
        .finish(2_000_000_000)
        .expect("长停止尾段应增量排空两轨");
    complete_gap_output(output, &session, 2_000_000_000, 20, 96_000, 1);
}

#[test]
fn long_video_gap_drains_silence_before_replacing_the_pending_frame() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut writer, session) = writer(temporary.path(), "av-long-video-gap", 10_000_000_000);
    writer.push_video(&rgba(1), 0).unwrap();
    writer.push_audio(pcm(0, 4_800)).unwrap();
    writer
        .push_video(&rgba(2), 5_000_000_000)
        .expect("长视频空洞不能突发塞满 video 队列");
    writer.push_audio(pcm(240_000, 4_800)).unwrap();
    let output = writer.finish(5_100_000_000).unwrap();
    complete_gap_output(output, &session, 5_100_000_000, 51, 244_800, 1);
}

#[test]
fn rotation_across_a_long_gap_commits_exact_audio_and_video_counts() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut writer, session) = writer(temporary.path(), "av-long-rotation-gap", 200_000_000);
    writer.push_video(&rgba(1), 0).unwrap();
    writer.push_audio(pcm(0, 4_800)).unwrap();
    writer
        .push_video(&rgba(2), 2_000_000_000)
        .expect("恢复分段边界应增量填补空洞");
    writer.push_audio(pcm(96_000, 4_800)).unwrap();
    let output = writer.finish(2_100_000_000).unwrap();
    complete_gap_output(output, &session, 2_100_000_000, 21, 100_800, 2);
}
