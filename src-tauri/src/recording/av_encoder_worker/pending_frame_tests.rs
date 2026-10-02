use super::*;
include!("../idle_frontier_read_tests.rs");

fn wait_audio_drained(audio: &AudioPipeline, worker: &AvEncoderWorker) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while audio.stats().unwrap().queued_chunks != 0 {
        assert!(
            !worker.is_finished(),
            "未封闭 slot 的 PCM 消费不应中止 owner"
        );
        assert!(std::time::Instant::now() < deadline, "应在预算内排空 PCM");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn reserved_pcm_survives_pause_resume_with_native_slot_replacement() {
    let temporary = tempfile::tempdir().unwrap();
    let (writer, video, audio_pipeline, path) = setup(temporary.path(), "pending-pause");
    let worker =
        AvEncoderWorker::spawn(writer, Arc::clone(&video), Arc::clone(&audio_pipeline), 0).unwrap();
    video.push(frame(0, 0, 1)).unwrap();
    video.publish_capture_lower_bound(0).unwrap();
    for sequence in 0..40 {
        audio_pipeline
            .push(audio(sequence, sequence * 1_000_000, 48))
            .unwrap();
    }
    wait_audio_drained(&audio_pipeline, &worker);
    video.pause(40_000_000).unwrap();
    audio_pipeline.pause(40_000_000).unwrap();
    audio_pipeline.push(audio(40, 41_000_000, 48)).unwrap();
    video.publish_capture_lower_bound(500_000_000).unwrap();
    video.resume(1_040_000_000).unwrap();
    audio_pipeline.resume(1_040_000_000).unwrap();
    video.push(frame(1, 1_040_000_001, 200)).unwrap();
    audio_pipeline
        .push(audio(41, 1_040_000_001, 2_400))
        .unwrap();
    video.finish(1_090_000_001).unwrap();
    audio_pipeline.finish(1_090_000_001).unwrap();
    let report = worker.wait().unwrap();
    assert_eq!(report.video_input_frames, 2);
    assert_eq!(report.output.video_frame_count, 1);
    assert_eq!(report.output.audio_pcm_frame_count, 4_320);
    assert_eq!(report.finish.mux_duration_ns, 90_000_001);
    assert_eq!(audio_pipeline.stats().unwrap().ignored_while_paused, 1);
    report.output.completion.complete().unwrap();
    let value: Value =
        serde_json::from_slice(&fs::read(path.join("manifest.json")).unwrap()).unwrap();
    read_idle_complete(&path, &value, 10, 90_000_001);
}

fn interrupted_reserved(drop_owner: bool) {
    let temporary = tempfile::tempdir().unwrap();
    let (writer, video, audio_pipeline, path) = setup(
        temporary.path(),
        if drop_owner {
            "pending-drop"
        } else {
            "pending-error"
        },
    );
    let worker =
        AvEncoderWorker::spawn(writer, Arc::clone(&video), Arc::clone(&audio_pipeline), 0).unwrap();
    video.push(frame(0, 0, 1)).unwrap();
    video.publish_capture_lower_bound(450_000_000).unwrap();
    for sequence in 0..5 {
        audio_pipeline
            .push(audio(sequence, sequence * 100_000_000, 4_800))
            .unwrap();
        wait_audio_drained(&audio_pipeline, &worker);
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let prefix = loop {
        let value: Value =
            serde_json::from_slice(&fs::read(path.join("manifest.json")).unwrap()).unwrap();
        let segments = value["segments"].as_array().unwrap();
        if segments.len() == 2
            && segments
                .iter()
                .all(|segment| path.join(segment["fileName"].as_str().unwrap()).is_file())
        {
            break value;
        }
        assert!(!worker.is_finished());
        assert!(
            std::time::Instant::now() < deadline,
            "PCM 必须在下一 slot 前提交前缀"
        );
        thread::sleep(Duration::from_millis(5));
    };
    if drop_owner {
        drop(worker);
    } else {
        audio_pipeline.abort().unwrap();
        assert!(matches!(
            worker.wait(),
            Err(AvEncoderWorkerError::AudioPipeline(
                AudioPipelineError::Aborted
            ))
        ));
    }
    assert!(!video.is_open().unwrap());
    let value: Value =
        serde_json::from_slice(&fs::read(path.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(value["state"], "interrupted");
    assert_eq!(value["segments"], prefix["segments"]);
    for segment in value["segments"].as_array().unwrap() {
        read_idle_artifact(&path, &value, segment, 10);
    }
    assert!(!path.join("recording.webm.partial").exists());
    assert!(!path.join("segment-000002.webm.partial").exists());
}

#[test]
fn reserved_audio_abort_preserves_strict_committed_prefix() {
    interrupted_reserved(false);
}

#[test]
fn reserved_owner_drop_joins_bridges_and_preserves_strict_committed_prefix() {
    interrupted_reserved(true);
}
