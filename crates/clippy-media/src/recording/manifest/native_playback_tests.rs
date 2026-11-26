use super::*;
use crate::recording::native_playback::{NativeWebmDecoder, VideoFrame};

fn color(frame: VideoFrame, at_ns: u64, expected: [u8; 3]) {
    assert_eq!(frame.timestamp_ns, at_ns);
    let actual = frame.image.get_pixel(0, 0).0;
    assert!(
        actual[..3]
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual.abs_diff(expected) <= 3),
        "实际颜色 {actual:?}，预期 {expected:?}"
    );
}

#[test]
fn indexed_playback_seeks_a_selected_segment_and_recovered_output() {
    let root = tempfile::tempdir().unwrap();
    let audio = OpusPacketEncoder::new(2).unwrap();
    let mut config = av_recovery_journal_config("indexed-av", audio.track_config());
    config.width = 64;
    config.height = 48;
    let journal = RecordingJournal::create(root.path(), config).unwrap();
    let mut writer = SegmentedAvRecordingWriter::new(
        journal,
        Arc::new(RecordingPipeline::default()),
        64,
        48,
        10,
        2,
        200_000_000,
        audio,
    )
    .unwrap();
    let red = [190, 70, 55];
    let blue = [50, 100, 195];
    for (frame, rgb) in [red, red, blue, blue, blue].into_iter().enumerate() {
        let rgba = [rgb[0], rgb[1], rgb[2], 255].repeat(64 * 48);
        writer
            .push_video(&rgba, frame as u64 * 100_000_000)
            .unwrap();
        if frame < 4 {
            writer
                .push_audio(av_recovery_pcm(frame as u32 * 4_800))
                .unwrap();
        }
    }
    drop(writer);

    let mut segment = NativeWebmDecoder::open_library_artifact(
        root.path(),
        "indexed-av",
        "segment-000001",
        || Ok(()),
    )
    .unwrap();
    assert_eq!(segment.duration_ns(), 200_000_000);
    color(
        segment.video_through(0, || Ok(())).unwrap().unwrap(),
        0,
        blue,
    );
    assert_eq!(segment.next_pcm(|| Ok(())).unwrap().unwrap().first_frame, 0);
    drop(segment);

    merge_interrupted_vp9_session(root.path(), "indexed-av").unwrap();
    let thumbnail = resolve_library_thumbnail_source(root.path(), "indexed-av")
        .unwrap()
        .unwrap();
    let image = crate::recording::thumbnail::decode_thumbnail(&thumbnail).unwrap();
    color(
        VideoFrame {
            timestamp_ns: 0,
            image,
        },
        0,
        red,
    );
    let mut final_output =
        NativeWebmDecoder::open_library_artifact(root.path(), "indexed-av", "final", || Ok(()))
            .unwrap();
    assert_eq!(final_output.duration_ns(), 400_000_000);
    assert_eq!(final_output.audio_channels(), Some(2));
    let requested = 312_345_678;
    final_output.seek(requested, || Ok(())).unwrap();
    color(
        final_output
            .video_through(requested, || Ok(()))
            .unwrap()
            .unwrap(),
        300_000_000,
        blue,
    );
    let mut next_frame = 14_993;
    while let Some(block) = final_output.next_pcm(|| Ok(())).unwrap() {
        assert_eq!(block.first_frame, next_frame);
        assert!(!block.samples.is_empty() && block.samples.len().is_multiple_of(2));
        assert!(block.samples.iter().all(|sample| sample.is_finite()));
        next_frame += (block.samples.len() / 2) as u64;
    }
    assert_eq!(next_frame, 19_200);

    assert_eq!(final_output.seek(u64::MAX, || Ok(())).unwrap(), 399_999_999);
    color(
        final_output
            .video_through(399_999_999, || Ok(()))
            .unwrap()
            .unwrap(),
        300_000_000,
        blue,
    );
    assert!(final_output.next_pcm(|| Ok(())).unwrap().is_none());
    final_output.seek(0, || Ok(())).unwrap();
    color(
        final_output.video_through(0, || Ok(())).unwrap().unwrap(),
        0,
        red,
    );
    assert_eq!(
        final_output
            .next_pcm(|| Ok(()))
            .unwrap()
            .unwrap()
            .first_frame,
        0
    );
}

#[test]
fn indexed_video_only_uses_the_last_frame_and_checks_the_file_hash() {
    let root = tempfile::tempdir().unwrap();
    let mut journal =
        RecordingJournal::create(root.path(), vp9_journal_config("indexed-video")).unwrap();
    let expected = [160, 84, 120];
    commit_vp9_segment(&mut journal, expected);
    journal.interrupt().unwrap();
    let open = || {
        NativeWebmDecoder::open_library_artifact(
            root.path(),
            "indexed-video",
            "segment-000000",
            || Ok(()),
        )
    };
    let mut player = open().unwrap();
    let position = player.seek(u64::MAX, || Ok(())).unwrap();
    color(
        player.video_through(position, || Ok(())).unwrap().unwrap(),
        100_000_000,
        expected,
    );
    assert_eq!(player.audio_channels(), None);
    assert!(player.next_pcm(|| Ok(())).unwrap().is_none());
    drop(player);

    let canceled = NativeWebmDecoder::open_library_artifact(
        root.path(),
        "indexed-video",
        "segment-000000",
        || Err("cancelled".into()),
    );
    assert_eq!(canceled.err().as_deref(), Some("cancelled"));
    let artifact =
        resolve_library_artifact(root.path(), "indexed-video", "segment-000000").unwrap();
    let mut bytes = fs::read(&artifact.path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(&artifact.path, &bytes).unwrap();
    assert!(open().err().unwrap().contains("哈希不一致"));
}

#[test]
fn native_thumbnail_accepts_padding_from_a_two_pixel_recording() {
    use crate::recording::mux::vp9_webm::Vp9WebmWriter;

    let root = tempfile::tempdir().unwrap();
    let mut config = vp9_journal_config("indexed-tiny");
    config.width = 2;
    config.height = 2;
    let mut journal = RecordingJournal::create(root.path(), config).unwrap();
    let (file, pending) = journal.begin_segment().unwrap();
    let rgba = [0, 0, 0, 255].repeat(4);
    let mut writer = Vp9WebmWriter::new(file, 2, 2, 10, 1).unwrap();
    writer.push_rgba(&rgba, 0).unwrap();
    let encoded = writer.finish_with_stats(100_000_000).unwrap();
    journal
        .commit_segment(pending, encoded.writer, 100_000_000, encoded.frame_count, 0)
        .unwrap();
    journal.interrupt().unwrap();
    let source = resolve_library_thumbnail_source(root.path(), "indexed-tiny")
        .unwrap()
        .unwrap();
    let image = crate::recording::thumbnail::decode_thumbnail(&source).unwrap();
    let frame = VideoFrame {
        timestamp_ns: 0,
        image,
    };
    color(frame, 0, [0, 0, 0]);
}

#[test]
fn indexed_seek_does_not_decode_the_prefix_of_a_long_recording() {
    use crate::recording::mux::vp9_webm::{Vp9PacketEncoder, WebmPacketMux};

    let root = tempfile::tempdir().unwrap();
    let expected = [70, 170, 210];
    let rgba = [expected[0], expected[1], expected[2], 255].repeat(64 * 48);
    let mut encoder = Vp9PacketEncoder::new(64, 48, 1, 1).unwrap();
    let mut keyframe = Vec::new();
    let mut save_frame = |packet: &[u8], _: u64, key: bool| {
        assert!(key);
        keyframe.extend_from_slice(packet);
        Ok(())
    };
    encoder.push_rgba(&rgba, 0, &mut save_frame).unwrap();
    encoder.finish(TIMEBASE_HZ, &mut save_frame).unwrap();

    let mut config = vp9_journal_config("indexed-long");
    config.target_fps_numerator = 1;
    let mut journal = RecordingJournal::create(root.path(), config).unwrap();
    let (file, pending) = journal.begin_segment().unwrap();
    let mut mux = WebmPacketMux::new(file, 64, 48).unwrap();
    for second in 0..1_800 {
        mux.add_frame(&keyframe, second * TIMEBASE_HZ, true)
            .unwrap();
    }
    let output = mux.finish(1_800 * TIMEBASE_HZ).unwrap();
    journal
        .commit_segment(
            pending,
            output.writer,
            1_800 * TIMEBASE_HZ,
            output.frame_count,
            0,
        )
        .unwrap();
    journal.interrupt().unwrap();

    let mut player = NativeWebmDecoder::open_library_artifact(
        root.path(),
        "indexed-long",
        "segment-000000",
        || Ok(()),
    )
    .unwrap();
    let target = 1_790 * TIMEBASE_HZ + 500_000_000;
    player.seek(target, || Ok(())).unwrap();
    let mut checkpoints = 0;
    let frame = player
        .video_through(target, || {
            checkpoints += 1;
            Ok(())
        })
        .unwrap()
        .unwrap();
    color(frame, 1_790 * TIMEBASE_HZ, expected);
    assert!(checkpoints < 64, "定位后扫描了 {checkpoints} 步");
}
