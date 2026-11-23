// 仅在修复后运行；原实现会提前分配一天静音，不能作为未改实现的安全对照。
#[test]
fn mixed_stop_ready_bound_large_gap_does_not_allocate_all_remaining_pcm() {
    let (mut source, probe) = stop_bound_source(true, true, 2_000_000_000);
    source.system.stop_at = 86_400_000_000_000;
    source.microphone.stop_at = 86_400_000_000_000;
    source.start_capture().unwrap();
    source
        .capture_next_available(Duration::ZERO)
        .unwrap()
        .unwrap();
    assert_eq!(source.stop_capture().unwrap(), 86_400_000_000_000);
    let first = source.take_stopped_chunks().unwrap();
    assert_eq!(first.iter().map(|x| x.frame_count).sum::<u32>(), 48_000);
    assert!(first
        .iter()
        .all(|x| x.frame_count <= 960 && x.samples.iter().all(|s| *s == 0.0)));
    drop(source);
    assert_eq!(probe.drops.load(Ordering::Acquire), 2);
    stop_bound_observation(
        "source-large-gap-green-only",
        serde_json::json!({"stopNs":86_400_000_000_000_u64,"firstBatchFrames":48000,"sourceDrops":probe.drops.load(Ordering::Acquire),"originalBaselineRun":false}),
    );
}
