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

fn feed_reserved(writer: &mut SegmentedAvRecordingWriter, end_frame: u64) {
    while writer.audio_frame_cursor < end_frame {
        let (end_ns, slot) = writer
            .pending_audio_window()
            .unwrap()
            .expect("既有包槽应足够");
        let available = timestamp_to_audio_frame(end_ns).unwrap() - writer.audio_frame_cursor;
        let frames = (end_frame - writer.audio_frame_cursor)
            .min(available)
            .min(4_800) as u32;
        writer
            .push_audio_before(pcm(writer.audio_frame_cursor, frames), slot)
            .unwrap();
    }
}

#[test]
fn pending_slot_exhausts_only_original_packets_and_accepts_later_native_replacement() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut writer, path) = idle_writer(temporary.path(), "pending-replacement", 1, 200_000_000);
    writer.push_video(&rgba(1), 0).unwrap();
    feed_reserved(&mut writer, 30_720);
    assert_eq!(
        writer.final_mux.as_ref().unwrap().available_audio_packets(),
        0
    );
    assert_eq!(
        writer
            .segment_mux
            .as_ref()
            .unwrap()
            .available_audio_packets(),
        0
    );
    assert_eq!(writer.pending_audio_window().unwrap(), None);
    assert_eq!(writer.video_encoder.next_timestamp_ns().unwrap(), 0);
    assert!(manifest(&path)["segments"].as_array().unwrap().is_empty());
    writer.push_video(&rgba(200), 250_000_000).unwrap();
    let output = writer.finish(650_000_000).unwrap();
    assert_eq!(output.video_frame_count, 1);
    assert_eq!(output.audio_pcm_frame_count, 31_200);
    output.completion.complete().unwrap();
    let value = manifest(&path);
    read_idle_complete(&path, &value, 1, 650_000_000);
    // 真实后到帧必须成为尚未编码的 slot，而不是占位图像已被抢先固化。
    let mut expected = Vp9PacketEncoder::new(2, 2, 1, 1).unwrap();
    let mut packets = Vec::new();
    expected
        .push_rgba(&rgba(200), 0, &mut |data, _, _| {
            packets.push(data.to_vec());
            Ok(())
        })
        .unwrap();
    expected
        .finish(650_000_000, &mut |data, _, _| {
            packets.push(data.to_vec());
            Ok(())
        })
        .unwrap();
    let bytes = fs::read(path.join(value["finalOutput"]["fileName"].as_str().unwrap())).unwrap();
    assert_eq!(packets.len(), 1);
    assert!(bytes
        .windows(packets[0].len())
        .any(|bytes| bytes == packets[0]));
}

#[test]
fn exact_boundary_stop_does_not_create_empty_reserved_tail() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut writer, path) = idle_writer(temporary.path(), "pending-exact-stop", 1, 200_000_000);
    writer.push_video(&rgba(1), 0).unwrap();
    feed_reserved(&mut writer, 30_720);
    writer.push_idle_video(1_000_000_000).unwrap();
    writer
        .push_audio_before(pcm(30_720, 4_800), 1_000_000_000)
        .unwrap();
    writer
        .push_audio_before(pcm(35_520, 4_800), 1_000_000_000)
        .unwrap();
    writer
        .push_audio_before(pcm(40_320, 4_800), 1_000_000_000)
        .unwrap();
    writer
        .push_audio_before(pcm(45_120, 2_880), 1_000_000_000)
        .unwrap();
    let output = writer.finish(1_000_000_000).unwrap();
    output.completion.complete().unwrap();
    let value = manifest(&path);
    assert_eq!(value["segments"].as_array().unwrap().len(), 1);
    read_idle_complete(&path, &value, 1, 1_000_000_000);
}

#[test]
fn next_slot_audio_rotates_before_pcm_and_stop_emits_reserved_slot() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut writer, path) = idle_writer(temporary.path(), "pending-rotated-stop", 1, 200_000_000);
    writer.push_video(&rgba(1), 0).unwrap();
    feed_reserved(&mut writer, 30_720);
    writer.push_idle_video(1_000_000_000).unwrap();
    while writer.audio_frame_cursor < 48_000 {
        let frames = (48_000 - writer.audio_frame_cursor).min(4_800) as u32;
        writer
            .push_audio_before(pcm(writer.audio_frame_cursor, frames), 1_000_000_000)
            .unwrap();
    }
    feed_reserved(&mut writer, 48_048);
    assert_eq!(writer.segment_started_at_ns, 1_000_000_000);
    assert!(!writer.segment_has_video);
    assert_eq!(
        writer.video_encoder.next_timestamp_ns().unwrap(),
        1_000_000_000
    );
    let prefix = manifest(&path);
    assert_eq!(prefix["segments"][0]["audio"]["pcmFrameCount"], 48_000);
    read_idle_artifact(&path, &prefix, &prefix["segments"][0], 1);
    let output = writer.finish(1_001_000_000).unwrap();
    output.completion.complete().unwrap();
    let value = manifest(&path);
    assert_eq!(value["segments"][1]["audio"]["pcmFrameCount"], 48);
    read_idle_complete(&path, &value, 1, 1_001_000_000);
}

#[test]
fn fractional_reserved_slots_preserve_sample_cuts_and_strict_files() {
    for fps in [15, 30] {
        let temporary = tempfile::tempdir().unwrap();
        let (mut writer, path) =
            idle_writer(temporary.path(), "pending-fractional", fps, 50_000_000);
        writer.push_video(&rgba(1), 0).unwrap();
        while writer.audio_frame_cursor < 14_400 {
            let slot_end =
                timestamp_to_audio_frame(writer.video_encoder.next_frame_end_ns().unwrap())
                    .unwrap();
            feed_reserved(&mut writer, slot_end.min(14_400));
            if writer.audio_frame_cursor == slot_end && slot_end < 14_400 {
                let bound = writer.video_encoder.next_frame_end_ns().unwrap() + 1;
                writer.push_idle_video(bound).unwrap();
            }
        }
        let output = writer.finish(300_000_000).unwrap();
        assert_eq!(output.audio_pcm_frame_count, 14_400);
        output.completion.complete().unwrap();
        read_idle_complete(&path, &manifest(&path), fps, 300_000_000);
    }
}

#[test]
fn confirmed_audio_gap_uses_same_capacity_before_native_boundary() {
    let temporary = tempfile::tempdir().unwrap();
    let (mut writer, path) = idle_writer(temporary.path(), "pending-gap", 1, 200_000_000);
    writer.push_video(&rgba(1), 0).unwrap();
    let (end, slot) = writer.pending_audio_window().unwrap().unwrap();
    assert_eq!(end, 640_000_000);
    writer.pad_pending_audio_until(end, slot).unwrap();
    assert_eq!(writer.pending_audio_window().unwrap(), None);
    assert_eq!(writer.video_encoder.next_timestamp_ns().unwrap(), 0);
    writer.push_video(&rgba(200), 1_000_000_000).unwrap();
    feed_reserved(&mut writer, 52_800);
    let output = writer.finish(1_100_000_000).unwrap();
    assert_eq!(output.audio_pcm_frame_count, 52_800);
    output.completion.complete().unwrap();
    read_idle_complete(&path, &manifest(&path), 1, 1_100_000_000);
}
