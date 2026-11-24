use super::*;
include!("../idle_frontier_read_tests.rs");

fn idle_writer(
    root: &std::path::Path,
    id: &str,
    fps: u32,
    segment: u64,
) -> (SegmentedAvRecordingWriter, PathBuf) {
    let audio = OpusPacketEncoder::new(2).unwrap();
    let mut settings = journal_config(id, audio.track_config());
    settings.target_fps_numerator = fps;
    let journal = RecordingJournal::create(root, settings).unwrap();
    let path = journal.session_directory().to_path_buf();
    (
        SegmentedAvRecordingWriter::new(
            journal,
            Arc::new(RecordingPipeline::default()),
            2,
            2,
            fps,
            2,
            segment,
            audio,
        )
        .unwrap(),
        path,
    )
}

#[test]
fn fractional_idle_slots_rotate_on_shared_cfr_cuts_and_keep_next_native_slot() {
    for fps in [15, 30] {
        let temporary = tempfile::tempdir().unwrap();
        let (mut writer, path) = idle_writer(temporary.path(), "idle-fractional", fps, 50_000_000);
        writer.push_video(&rgba(1), 0).unwrap();
        while writer.idle_video_step(240_000_000).unwrap().is_some() {
            writer.push_idle_video(240_000_000).unwrap();
        }
        let before = manifest(&path);
        assert!(before["segments"].as_array().unwrap().len() >= 2);
        for segment in before["segments"].as_array().unwrap() {
            read_idle_artifact(&path, &before, segment, fps);
        }
        writer.push_video(&rgba(200), 240_000_000).unwrap();
        let output = writer.finish(300_000_000).unwrap();
        assert_eq!(output.video_frame_count, (u64::from(fps) * 3).div_ceil(10));
        assert_eq!(output.audio_pcm_frame_count, 14_400);
        output.completion.complete().unwrap();
        read_idle_complete(&path, &manifest(&path), fps, 300_000_000);
    }
}

#[test]
fn one_fps_bound_inside_slot_cannot_emit_or_rotate_replaceable_frame() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut writer, path) = idle_writer(temporary.path(), "idle-one-slot", 1, 200_000_000);
    writer.push_video(&rgba(1), 0).unwrap();
    assert_eq!(writer.idle_video_step(250_000_000).unwrap(), None);
    assert!(manifest(&path)["segments"].as_array().unwrap().is_empty());
    writer.push_video(&rgba(2), 250_000_000).unwrap();
    writer.push_idle_video(1_250_000_000).unwrap();
    assert_eq!(writer.idle_video_step(1_250_000_000).unwrap(), None);
    writer.push_video(&rgba(3), 1_250_000_000).unwrap();
    let output = writer.finish(2_000_000_000).unwrap();
    assert_eq!(output.video_frame_count, 2);
    output.completion.complete().unwrap();
    read_idle_complete(&path, &manifest(&path), 1, 2_000_000_000);
}
