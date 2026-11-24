fn precision_pipeline(origin: u64) -> AudioPipeline {
    let pipeline = AudioPipeline::new(origin);
    pipeline.configure_timestamp_precision(AudioTimestampPrecision::HundredNanoseconds).unwrap();
    pipeline
}

#[test]
fn wasapi_qpc_policy_is_fixed_after_pcm_or_control() {
    for control in [false, true] {
        let pipeline = precision_pipeline(0);
        if control { pipeline.pause(0).unwrap(); } else { pipeline.push(chunk(0, 0, 640)).unwrap(); }
        assert_eq!(pipeline.configure_timestamp_precision(AudioTimestampPrecision::Exact), Err(AudioPipelineError::TimestampPrecisionAlreadyStarted));
        pipeline.configure_timestamp_precision(AudioTimestampPrecision::HundredNanoseconds).unwrap();
    }
    let pipeline = precision_pipeline(0); pipeline.finish(0).unwrap();
    assert_eq!(pipeline.configure_timestamp_precision(AudioTimestampPrecision::Exact), Err(AudioPipelineError::Closed));
}

#[test]
fn wasapi_qpc_inclusive_tick_aligns_only_media_and_keeps_source_pcm() {
    let origin = 1_000_000_000;
    for overlap in [0, 1, 33, 99, 100] {
        let pipeline = precision_pipeline(origin);
        pipeline.push(chunk(0, origin, 960)).unwrap();
        let timestamp = origin + 20_000_000 - overlap;
        let mut second = chunk(1, timestamp, 640); second.samples.fill(0.25);
        assert_eq!(pipeline.push(second).unwrap(), AudioPushOutcome::Queued { presentation_at_ns:20_000_000, duration_ns:13_333_333, gap_before_ns:0 });
        pipeline.pop().unwrap(); let queued=pipeline.pop().unwrap().unwrap();
        assert_eq!(queued.chunk.captured_at_ns,timestamp); assert_eq!(queued.chunk.sequence,1);
        assert_eq!(queued.chunk.samples.as_ref(),vec![0.25;1280]); assert_eq!(queued.chunk.frame_count,640);
    }
    let exact = AudioPipeline::new(origin); exact.push(chunk(0,origin,960)).unwrap();
    assert_eq!(exact.push(chunk(1,origin+19_999_999,640)),Err(AudioPipelineError::PresentationOverlap));
}

#[test]
fn wasapi_qpc_real_overlap_rejects_atomically_and_real_gap_remains() {
    let pipeline = precision_pipeline(0); pipeline.push(chunk(0,0,960)).unwrap();
    let before=pipeline.stats().unwrap();
    assert_eq!(pipeline.push(chunk(1,19_999_899,640)),Err(AudioPipelineError::PresentationOverlap));
    assert_eq!(pipeline.stats().unwrap(),before);
    assert_eq!(pipeline.push(chunk(1,20_000_101,640)).unwrap(),AudioPushOutcome::Queued { presentation_at_ns:20_000_101, duration_ns:13_333_333, gap_before_ns:101 });
}

#[test]
fn wasapi_qpc_raw_origin_sequence_and_resume_lower_bounds_stay_strict() {
    let pipeline = precision_pipeline(100);
    assert_eq!(pipeline.push(chunk(0,99,1)),Err(AudioPipelineError::SourceBeforeOrigin));
    pipeline.push(chunk(0,100,640)).unwrap();
    assert_eq!(pipeline.push(chunk(0,13_333_400,640)),Err(AudioPipelineError::SequenceNotIncreasing));
    assert_eq!(pipeline.push(chunk(1,100,640)),Err(AudioPipelineError::SourceTimestampNotIncreasing));
    pipeline.pause(13_333_433).unwrap();
    assert_eq!(pipeline.resume(13_333_433),Err(AudioPipelineError::InvalidResumeTimestamp));
    pipeline.resume(113_333_433).unwrap();
    assert_eq!(pipeline.push(chunk(1,113_333_432,640)),Err(AudioPipelineError::SourceTimestampNotIncreasing));
    pipeline.push(chunk(1,113_333_433,640)).unwrap();
    assert_eq!(pipeline.push(chunk(2,113_333_433,640)),Err(AudioPipelineError::SourceTimestampNotIncreasing));
}

#[test]
fn wasapi_qpc_pause_resume_and_finish_keep_raw_controls_and_media_tail() {
    let pipeline=precision_pipeline(0);
    pipeline.push(chunk(0,0,640)).unwrap(); pipeline.push(chunk(1,13_333_300,640)).unwrap();
    assert_eq!(pipeline.pause(26_666_565),Err(AudioPipelineError::InvalidPauseTimestamp));
    pipeline.pause(26_666_633).unwrap();
    assert_eq!(pipeline.state.lock().unwrap().timeline.paused_at_ns,Some(26_666_633));
    pipeline.resume(126_666_633).unwrap(); pipeline.push(chunk(2,126_666_633,640)).unwrap();
    assert_eq!(pipeline.finish(139_999_898),Err(AudioPipelineError::FinishBeforeBufferedAudioEnd));
    assert_eq!(pipeline.finish(139_999_966).unwrap(),39_999_999);
    let state=pipeline.state.lock().unwrap();
    assert_eq!(state.timeline.last_source_ns,Some(139_999_966)); assert_eq!(state.timeline.accumulated_pause_ns,100_000_000);
}

#[test]
fn wasapi_qpc_backpressure_does_not_consume_alignment_or_sequence() {
    let pipeline=precision_pipeline(0);
    for sequence in 0..10 { pipeline.push(chunk(sequence,sequence*100_000_000,4800)).unwrap(); }
    let before=pipeline.stats().unwrap();
    assert_eq!(pipeline.push(chunk(10,999_999_900,640)),Err(AudioPipelineError::Backpressure));
    assert_eq!(pipeline.stats().unwrap(),before); pipeline.pop().unwrap();
    assert!(matches!(pipeline.push(chunk(10,999_999_900,640)),Ok(AudioPushOutcome::Queued { presentation_at_ns:1_000_000_000, gap_before_ns:0,.. })));
}

#[test]
fn wasapi_qpc_alignment_overflow_keeps_previous_timeline() {
    let pipeline=precision_pipeline(0); pipeline.push(chunk(0,u64::MAX-20_833,1)).unwrap();
    let before=pipeline.stats().unwrap();
    assert_eq!(pipeline.push(chunk(1,u64::MAX-99,1)),Err(AudioPipelineError::TimelineOverflow));
    assert_eq!(pipeline.stats().unwrap(),before); assert_eq!(pipeline.finish(u64::MAX).unwrap(),u64::MAX);
}

#[test]
fn wasapi_qpc_multiple_quantized_packets_keep_all_frames_and_native_pts() {
    let pipeline=precision_pipeline(0); let mut raw_end=0;
    for sequence in 0..20 {
        let raw=sequence*640*1_000_000_000/48_000/100*100;
        let mut value=chunk(sequence,raw,640); value.samples.fill(sequence as f32/100.0);
        pipeline.push(value).unwrap(); raw_end=raw+13_333_333;
    }
    let duration=pipeline.finish(raw_end).unwrap(); assert!(duration.abs_diff(266_666_666)<=100);
    let mut frames=0;
    for sequence in 0..20 {
        let value=pipeline.pop().unwrap().unwrap(); frames+=value.chunk.frame_count;
        assert_eq!(value.chunk.captured_at_ns,sequence*640*1_000_000_000/48_000/100*100);
        assert!(value.chunk.samples.iter().all(|value| *value==sequence as f32/100.0));
    }
    assert_eq!(frames,12_800); assert!(pipeline.pop().unwrap().is_none());
}
