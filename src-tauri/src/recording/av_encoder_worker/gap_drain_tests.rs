use super::*;

#[test]
fn video_eos_before_queued_audio_drains_and_completes_a_long_stop_tail() {
    let temporary = tempfile::tempdir().unwrap();
    let (writer, video, audio_pipeline, session) = setup(temporary.path(), "av-eos-tail");
    let worker =
        AvEncoderWorker::spawn(writer, Arc::clone(&video), Arc::clone(&audio_pipeline), 0).unwrap();
    video.push(frame(0, 100_000_000, 1)).unwrap();
    video.finish(2_100_000_000).unwrap();
    for sequence in 0..10 {
        audio_pipeline
            .push(audio(sequence, 100_000_000 + sequence * 100_000_000, 4_800))
            .unwrap();
    }
    audio_pipeline.finish(2_100_000_000).unwrap();
    let report = worker
        .wait()
        .expect("视频 EOS 后的 PCM 与静音尾段不能耗尽 packet 队列");
    assert_eq!(report.video_input_frames, 1);
    assert_eq!(report.audio_input_chunks, 10);
    assert_eq!(report.audio_input_frames, 48_000);
    assert_eq!(report.audio_dropped_before_video_frames, 0);
    assert_eq!(report.finish.mux_duration_ns, 2_000_000_000);
    assert_eq!(report.output.video_frame_count, 20);
    assert_eq!(report.output.audio_pcm_frame_count, 96_000);
    report.output.completion.complete().unwrap();
    let value: Value =
        serde_json::from_slice(&fs::read(session.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(value["state"], "complete");
    assert_eq!(value["finalOutput"]["audio"]["pcmFrameCount"], 96_000);
}
