use crate::recording::timeline::{RecordingTimeline, TimelineError};

#[test]
fn av_control_timeline_common_pause_rejects_backward_metadata() {
    let mut video = RecordingTimeline::default();
    assert_eq!(video.extend_pause(200), Err(TimelineError::NotPaused));
    video.map_frame(100).unwrap();
    video.pause(200).unwrap();
    assert_eq!(
        video.extend_pause(190),
        Err(TimelineError::SourceTimestampNotIncreasing)
    );
    video.extend_pause(240).unwrap();
    video.resume(520).unwrap();
    assert_eq!(video.map_frame(700).unwrap(), Some(320));

    let audio = AudioPipeline::new(100_000_000);
    assert_eq!(
        audio.extend_pause(200_000_000),
        Err(AudioPipelineError::NotPaused)
    );
    audio.pause(200_000_000).unwrap();
    assert_eq!(
        audio.extend_pause(190_000_000),
        Err(AudioPipelineError::InvalidPauseTimestamp)
    );
    audio.extend_pause(240_000_000).unwrap();
    audio.resume_at(560_000_000, 520_000_000).unwrap();
    assert_eq!(audio.finish(1_000_000_000).unwrap(), 620_000_000);
}

#[test]
fn av_control_timeline_invalid_common_resume_preserves_paused_state() {
    let audio = AudioPipeline::new(100_000_000);
    audio.pause(200_000_000).unwrap();
    for (source, media) in [
        (560_000_000, 600_000_000),
        (560_000_000, 50_000_000),
        (200_000_000, 520_000_000),
    ] {
        assert_eq!(
            audio.resume_at(source, media),
            Err(AudioPipelineError::InvalidResumeTimestamp)
        );
    }
    audio.resume_at(560_000_000, 520_000_000).unwrap();
    assert_eq!(audio.finish(1_000_000_000).unwrap(), 580_000_000);
}

#[test]
fn av_control_timeline_future_pcm_pause_keeps_valid_fast_resume() {
    // 对应 WASAPI safe_control_timestamp 取包末尾，视频恢复早于该公共起点的零交集。
    let observed = owner_mapping(
        "future-pcm-pause",
        [180_000_000, 600_000_000],
        [520_000_000, 640_000_000],
        700_000_000,
        1,
    );
    assert_eq!(observed[0], (600_000_000, 600_000_000, 580_000_000));
}

#[test]
fn av_control_timeline_media_pause_does_not_replace_raw_video_guard() {
    let mut video = RecordingTimeline::default();
    video.map_frame(100).unwrap();
    video.pause(180).unwrap();
    video.extend_pause(600).unwrap();
    video.resume(520).unwrap();
    assert_eq!(video.map_frame(540).unwrap(), Some(440));
    assert_eq!(video.finish(580).unwrap(), 480);

    let mut stopped = RecordingTimeline::default();
    stopped.map_frame(100).unwrap();
    stopped.pause(180).unwrap();
    stopped.extend_pause(600).unwrap();
    assert_eq!(stopped.finish(580).unwrap(), 480);
}

#[test]
fn av_control_timeline_native_pcm_lower_bound_survives_media_override() {
    let temporary = tempfile::tempdir().unwrap();
    // 视频 520 ms 已启动，音频 560 ms 才启动：540 ms 的 PCM 仍必须拒绝。
    let probe = ControlProbe::new([200_000_000; 2], [520_000_000, 560_000_000], 540_000_000);
    let mut session = manual_owner(&probe, temporary.path());
    session.pause().unwrap();
    session.resume().unwrap();
    probe.allowed.store(2, Ordering::Release);
    wait_probe(|| {
        session.audio_capture.as_ref().unwrap().is_finished()
            && session.video_pipeline.stats().unwrap().accepted_frames == 2
    });
    let error = session.audio_capture.take().unwrap().wait().unwrap_err();
    assert_eq!(
        error,
        AudioCaptureWorkerError::Pipeline(AudioPipelineError::SourceTimestampNotIncreasing)
    );
    record_probe(
        "stale-native-pcm",
        &serde_json::json!({"error":error.to_string()}),
    );
    drop(session);
    assert_eq!(probe.drops[0].load(Ordering::Acquire), 1);
    assert_eq!(probe.drops[1].load(Ordering::Acquire), 1);
}

#[test]
fn av_control_timeline_metadata_failure_returns_original_worker_root() {
    for side in 0..2 {
        let temporary = tempfile::tempdir().unwrap();
        let probe = ControlProbe::new([200_000_000; 2], [520_000_000; 2], 700_000_000);
        let mut session = manual_owner(&probe, temporary.path());
        session.pause().unwrap();
        if side == 0 {
            session.video_pipeline.abort().unwrap();
            let worker = session.video_capture.take().unwrap();
            let error = worker.extend_pause(210_000_000).unwrap_err();
            assert_eq!(error, CaptureWorkerError::Pipeline(PipelineError::Aborted));
            assert_eq!(worker.wait().unwrap_err(), error);
        } else {
            session.audio_pipeline.abort().unwrap();
            let worker = session.audio_capture.take().unwrap();
            let error = worker.extend_pause(210_000_000).unwrap_err();
            assert_eq!(
                error,
                AudioCaptureWorkerError::Pipeline(AudioPipelineError::Aborted)
            );
            assert_eq!(worker.wait().unwrap_err(), error);
        }
        drop(session);
        let controls = probe.hooks.lock().unwrap();
        assert_eq!(
            controls
                .iter()
                .filter(|(_, action, _)| *action != "stop")
                .count(),
            2
        );
        assert_eq!(probe.drops[0].load(Ordering::Acquire), 1);
        assert_eq!(probe.drops[1].load(Ordering::Acquire), 1);
    }
}
