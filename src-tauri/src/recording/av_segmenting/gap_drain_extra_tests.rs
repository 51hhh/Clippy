use super::*;

fn writer_at_fps(
    root: &std::path::Path,
    session_id: &str,
    fps: u32,
    segment_ns: u64,
) -> (SegmentedAvRecordingWriter, PathBuf) {
    let audio = OpusPacketEncoder::new(2).unwrap();
    let mut config = journal_config(session_id, audio.track_config());
    config.target_fps_numerator = fps;
    let journal = RecordingJournal::create(root, config).unwrap();
    let session = journal.session_directory().to_path_buf();
    let writer = SegmentedAvRecordingWriter::new(
        journal,
        Arc::new(RecordingPipeline::default()),
        2,
        2,
        fps,
        2,
        segment_ns,
        audio,
    )
    .unwrap();
    (writer, session)
}

#[test]
fn one_fps_stop_tail_remains_bounded_without_an_extra_pending_frame() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut writer, session) =
        writer_at_fps(temporary.path(), "av-one-fps-tail", 1, 10_000_000_000);
    writer.push_video(&rgba(1), 0).unwrap();
    writer.push_audio(pcm(0, 4_800)).unwrap();
    let output = writer.finish(2_000_000_000).unwrap();
    complete_gap_output(output, &session, 2_000_000_000, 2, 96_000, 1);
}

#[test]
fn one_fps_rotation_keeps_boundary_video_out_of_the_previous_segment() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut writer, session) =
        writer_at_fps(temporary.path(), "av-one-fps-rotation", 1, 200_000_000);
    writer.push_video(&rgba(1), 0).unwrap();
    writer.push_audio(pcm(0, 4_800)).unwrap();
    writer.push_video(&rgba(2), 2_000_000_000).unwrap();
    writer.push_audio(pcm(96_000, 4_800)).unwrap();
    let output = writer.finish(2_100_000_000).unwrap();
    complete_gap_output(output, &session, 2_100_000_000, 3, 100_800, 2);
}
