use super::*;

#[test]
fn native_video_boundary_splits_pcm_at_its_cfr_segment_cut() {
    let temporary = tempfile::tempdir().unwrap();
    let (writer, video, audio_pipeline, session) = setup(temporary.path(), "av-cfr-worker");
    let worker =
        AvEncoderWorker::spawn(writer, Arc::clone(&video), Arc::clone(&audio_pipeline), 0).unwrap();
    for (sequence, timestamp) in [0, 110_000_000, 220_000_000, 330_000_000]
        .into_iter()
        .enumerate()
    {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while video.stats().unwrap().queued_frames
            >= crate::recording::pipeline::FRAME_QUEUE_CAPACITY
        {
            assert!(
                std::time::Instant::now() < deadline,
                "原容量的视频队列必须被消费"
            );
            thread::yield_now();
        }
        video
            .push(frame(sequence as u64, timestamp, sequence as u8 + 1))
            .unwrap();
    }
    video.finish(440_000_000).unwrap();
    for sequence in 0..4 {
        audio_pipeline
            .push(audio(sequence, sequence * 100_000_000, 4_800))
            .unwrap();
    }
    audio_pipeline.finish(440_000_000).unwrap();
    let report = worker
        .wait()
        .expect("PCM 应在 CFR 分段边界拆分，不能先跨入旧分段");
    assert_eq!(report.video_input_frames, 4);
    assert_eq!(video.stats().unwrap().dropped_by_backpressure, 0);
    assert_eq!(report.audio_input_chunks, 4);
    assert_eq!(report.audio_input_frames, 19_200);
    assert_eq!(report.audio_dropped_before_video_frames, 0);
    assert_eq!(report.finish.mux_duration_ns, 440_000_000);
    assert_eq!(report.output.video_frame_count, 5);
    assert_eq!(report.output.audio_pcm_frame_count, 21_120);
    report.output.completion.complete().unwrap();
    let value: Value =
        serde_json::from_slice(&fs::read(session.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(value["state"], "complete");
    let segments = value["segments"].as_array().unwrap();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0]["durationNs"], 200_000_000);
    assert_eq!(segments[0]["audio"]["pcmFrameCount"], 9_600);
    assert_eq!(segments[1]["startedAtNs"], 200_000_000);
    assert_eq!(segments[1]["audio"]["pcmFrameCount"], 11_520);
}
