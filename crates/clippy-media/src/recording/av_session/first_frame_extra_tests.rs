#[test]
fn pending_first_frame_controls_return_without_pausing_video() {
    let temporary = tempfile::tempdir().unwrap();
    let (session, released, reads, finished, dropped, _, video_events) =
        setup_delayed(temporary.path(), "first-frame-controls", None);
    assert!(matches!(
        session.pause(),
        Err(AvRecordingSessionError::StartupPending)
    ));
    assert!(matches!(
        session.resume(),
        Err(AvRecordingSessionError::StartupPending)
    ));
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    released.store(true, Ordering::Release);
    finished.recv_timeout(Duration::from_secs(5)).unwrap();
    for _ in 0..20 {
        video_events.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    assert!(session.stop().is_ok());
    dropped.recv_timeout(Duration::from_secs(5)).unwrap();
}

#[test]
fn stop_before_first_frame_cancels_pending_audio_and_keeps_interrupted_journal() {
    let temporary = tempfile::tempdir().unwrap();
    let (session, _, reads, _, dropped, created, _) =
        setup_delayed(temporary.path(), "first-frame-stop", None);
    let result = session.stop();
    assert!(result.is_err(), "没有首帧的会话不能提交 complete");
    let (drop_thread, final_reads) = dropped.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(drop_thread, created);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert_eq!(final_reads, 0);
    let manifest: Value = serde_json::from_slice(
        &fs::read(
            temporary
                .path()
                .join("recordings/first-frame-stop/manifest.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["state"], "interrupted");
}
