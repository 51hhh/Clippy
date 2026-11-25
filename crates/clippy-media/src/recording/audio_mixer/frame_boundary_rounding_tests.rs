#[test]
fn mixed_frame_boundary_derived_resume_does_not_move_native_input_cursor() {
    // 派生暂停末尾 + 1 ns 恰好跨过下一帧的 nearest 中点，原生恢复首包仍在前一帧。
    let origin = 1_000_031_249;
    let raw_pause = 1_020_083_334;
    let (mut source, _) = boundary_source(origin, 1_000_083_334, 960, raw_pause, false);
    source.system.resume_at = raw_pause + 1; source.microphone.resume_at = raw_pause + 1;
    source.start_capture().unwrap(); let pipeline = AudioPipeline::new(origin);
    let first = source.capture_next_available(Duration::ZERO).unwrap().unwrap();
    let second = source.capture_next_available(Duration::ZERO).unwrap().unwrap();
    assert_eq!((first.frame_count,second.frame_count),(960,3));
    assert_eq!(&first.samples[..6], &[0.0;6]);
    assert!(first.samples[6..].iter().chain(second.samples.iter()).all(|sample| *sample == 0.5));
    pipeline.push(first).unwrap(); pipeline.push(second).unwrap();
    let pause = source.pause_capture().unwrap(); pipeline.pause(pause).unwrap();
    let resume = source.resume_capture().unwrap(); pipeline.resume(resume).unwrap();
    let output = source.capture_next_available(Duration::ZERO).unwrap().unwrap();
    boundary_observation("resume-rounding-cliff",serde_json::json!({"rawPause":raw_pause,"pause":pause,"rawResume":raw_pause+1,"resume":resume,"outputAt":output.captured_at_ns,"frames":output.frame_count}));
    assert_eq!(pause,1_020_093_749); assert_eq!(resume,1_020_093_750);
    assert_eq!(output.captured_at_ns,resume); assert_eq!(output.frame_count,960);
    assert!(output.samples.iter().all(|sample| *sample == 0.5));
    pipeline.push(output).unwrap(); pipeline.finish(source.stop_capture().unwrap()).unwrap();
}
