use super::*;
include!("../idle_frontier_read_tests.rs");

fn wait_commit(session: &std::path::Path, minimum_end: u64) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let value: Value =
            serde_json::from_slice(&fs::read(session.join("manifest.json")).unwrap()).unwrap();
        let end = value["segments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|segment| {
                segment["startedAtNs"].as_u64().unwrap() + segment["durationNs"].as_u64().unwrap()
            })
            .max()
            .unwrap_or(0);
        if end >= minimum_end
            && value["segments"].as_array().unwrap().iter().all(|segment| {
                session
                    .join(segment["fileName"].as_str().unwrap())
                    .is_file()
            })
        {
            return value;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "有下界时必须在 Stop 前提交周期分段"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn idle_metadata_commits_prefix_then_accepts_native_frame_in_unclosed_slot() {
    let temporary = tempfile::tempdir().unwrap();
    let (writer, video, audio_pipeline, session) = setup(temporary.path(), "idle-later-native");
    let worker =
        AvEncoderWorker::spawn(writer, Arc::clone(&video), Arc::clone(&audio_pipeline), 0).unwrap();
    video.push(frame(0, 0, 1)).unwrap();
    video.publish_capture_lower_bound(550_000_000).unwrap();
    for sequence in 0..6 {
        audio_pipeline
            .push(audio(sequence, sequence * 100_000_000, 4_800))
            .unwrap();
    }
    let before = wait_commit(&session, 400_000_000);
    for segment in before["segments"].as_array().unwrap() {
        read_idle_artifact(&session, &before, segment, 10);
    }
    assert_eq!(video.stats().unwrap().accepted_frames, 1);
    // 下界 550ms 封闭的是 500ms 之前的 slot；550ms 真实帧仍替换 500ms slot。
    video.push(frame(1, 550_000_000, 200)).unwrap();
    video.finish(700_000_000).unwrap();
    audio_pipeline.push(audio(6, 600_000_000, 4_800)).unwrap();
    audio_pipeline.finish(700_000_000).unwrap();
    let report = worker.wait().unwrap();
    assert_eq!(report.video_input_frames, 2);
    assert_eq!(report.output.video_frame_count, 7);
    assert_eq!(report.output.audio_pcm_frame_count, 33_600);
    assert_eq!(report.audio_input_chunks, 7);
    assert_eq!(video.stats().unwrap().dropped_by_backpressure, 0);
    report.output.completion.complete().unwrap();
    let value: Value =
        serde_json::from_slice(&fs::read(session.join("manifest.json")).unwrap()).unwrap();
    read_idle_complete(&session, &value, 10, 700_000_000);
}

#[test]
fn unsupported_source_without_bound_keeps_waiting_for_real_video_or_eos() {
    let temporary = tempfile::tempdir().unwrap();
    let (writer, video, audio_pipeline, session) = setup(temporary.path(), "idle-no-bound");
    let worker =
        AvEncoderWorker::spawn(writer, Arc::clone(&video), Arc::clone(&audio_pipeline), 0).unwrap();
    video.push(frame(0, 0, 1)).unwrap();
    for sequence in 0..5 {
        audio_pipeline
            .push(audio(sequence, sequence * 100_000_000, 4_800))
            .unwrap();
    }
    thread::sleep(Duration::from_millis(100));
    assert!(!worker.is_finished());
    let value: Value =
        serde_json::from_slice(&fs::read(session.join("manifest.json")).unwrap()).unwrap();
    assert!(value["segments"].as_array().unwrap().is_empty());
    video.finish(600_000_000).unwrap();
    audio_pipeline.finish(600_000_000).unwrap();
    let report = worker.wait().unwrap();
    assert_eq!(report.video_input_frames, 1);
    assert_eq!(report.audio_input_chunks, 5);
    report.output.completion.complete().unwrap();
    let value: Value =
        serde_json::from_slice(&fs::read(session.join("manifest.json")).unwrap()).unwrap();
    read_idle_complete(&session, &value, 10, 600_000_000);
}

fn interrupted_idle(drop_owner: bool) {
    let temporary = tempfile::tempdir().unwrap();
    let (writer, video, audio_pipeline, session) = setup(
        temporary.path(),
        if drop_owner {
            "idle-owner-drop"
        } else {
            "idle-owner-error"
        },
    );
    let worker =
        AvEncoderWorker::spawn(writer, Arc::clone(&video), Arc::clone(&audio_pipeline), 0).unwrap();
    video.push(frame(0, 0, 1)).unwrap();
    video.publish_capture_lower_bound(600_000_000).unwrap();
    for sequence in 0..6 {
        audio_pipeline
            .push(audio(sequence, sequence * 100_000_000, 4_800))
            .unwrap();
    }
    let before = wait_commit(&session, 400_000_000);
    let prefix = before["segments"].as_array().unwrap().clone();
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
        serde_json::from_slice(&fs::read(session.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(value["state"], "interrupted");
    assert_eq!(
        &value["segments"].as_array().unwrap()[..prefix.len()],
        prefix.as_slice()
    );
    for segment in &prefix {
        read_idle_artifact(&session, &value, segment, 10);
    }
}

#[test]
fn idle_audio_abort_preserves_strictly_readable_committed_prefix() {
    interrupted_idle(false);
}

#[test]
fn idle_owner_drop_joins_bridges_and_preserves_committed_prefix() {
    interrupted_idle(true);
}
