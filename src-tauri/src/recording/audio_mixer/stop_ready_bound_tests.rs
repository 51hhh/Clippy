use crate::recording::audio::{AudioPipeline, AudioPipelineError};
use crate::recording::audio_worker::{AudioCaptureWorker, AudioCaptureWorkerError};
use crate::recording::clock::RecordingSessionClock;

fn check_stop_bound_source(initial: bool, system_earlier: bool) {
    let (mut source, probe) = stop_bound_source(initial, system_earlier, 2_000_000_000);
    source.start_capture().unwrap();
    if initial {
        let first = source
            .capture_next_available(Duration::ZERO)
            .unwrap()
            .unwrap();
        assert_eq!(first.frame_count, 960);
    }
    let stop = source.stop_capture();
    let first = source.take_stopped_chunks();
    let name = if !initial {
        "source-before-first"
    } else if system_earlier {
        "source-system-earlier"
    } else {
        "source-microphone-earlier"
    };
    stop_bound_observation(
        name,
        serde_json::json!({"stop":format!("{stop:?}"),"firstBatchFrames":first.as_ref().ok().map(|x|x.iter().map(|c|u64::from(c.frame_count)).sum::<u64>()),"stopAttempts":probe.stop_attempts.load(Ordering::Acquire)}),
    );
    assert_eq!(stop.unwrap(), 2_040_000_000);
    let mut batch = first.unwrap();
    let mut total = if initial { 960 } else { 0 };
    let mut sequence = u64::from(initial);
    loop {
        assert!(
            batch.iter().map(|x| u64::from(x.frame_count)).sum::<u64>() <= 48_000,
            "停止ready输出必须有固定一秒上限"
        );
        if batch.is_empty() {
            break;
        }
        for chunk in batch {
            assert_eq!(chunk.sequence, sequence);
            sequence += 1;
            assert_eq!(chunk.captured_at_ns, total * 1_000_000_000 / 48_000);
            assert!(chunk.frame_count <= 960);
            for (index, sample) in chunk.samples.iter().enumerate() {
                let frame = total + index as u64 / 2;
                let gain = if initial && frame < 960 {
                    0.5
                } else if (96_000..96_960).contains(&frame) {
                    if system_earlier {
                        0.4
                    } else {
                        0.1
                    }
                } else if (96_960..97_920).contains(&frame) {
                    if system_earlier {
                        0.1
                    } else {
                        0.4
                    }
                } else {
                    0.0
                };
                assert!(
                    (*sample - gain * stop_bound_tone(frame as usize)).abs() < 0.000001,
                    "合法PCM不能丢失或平移"
                );
            }
            total += u64::from(chunk.frame_count);
        }
        batch = source.take_stopped_chunks().unwrap();
    }
    assert_eq!(total, 97_920);
    assert!(source.take_stopped_chunks().unwrap().is_empty());
    drop(source);
    assert_eq!(probe.drops.load(Ordering::Acquire), 2);
    stop_bound_observation(
        &format!("{name}-complete"),
        serde_json::json!({"allFramesIncludingPrefix":total,"sourceDrops":probe.drops.load(Ordering::Acquire)}),
    );
}
#[test]
fn mixed_stop_ready_bound_system_earlier_keeps_complete_pcm() {
    check_stop_bound_source(true, true);
}
#[test]
fn mixed_stop_ready_bound_microphone_earlier_keeps_complete_pcm() {
    check_stop_bound_source(true, false);
}
#[test]
fn mixed_stop_ready_bound_before_first_packet_keeps_all_silence_and_tails() {
    check_stop_bound_source(false, true);
}

fn check_stop_bound_worker(fail_second: bool) {
    let (source, probe) = stop_bound_batches(fail_second);
    let pipeline = Arc::new(AudioPipeline::new(0));
    let worker = AudioCaptureWorker::spawn_with_factory(
        move |_| Ok(source),
        RecordingSessionClock::new(),
        Arc::clone(&pipeline),
    )
    .unwrap();
    wait_stop_bound(|| worker.is_finished() || pipeline.stats().unwrap().accepted_chunks == 1);
    let result = worker.stop();
    let mut chunks = Vec::new();
    while let Some(x) = pipeline.pop().unwrap() {
        chunks.push(x.chunk);
    }
    let total = chunks.iter().map(|x| x.frame_count).sum::<u32>();
    stop_bound_observation(
        if fail_second {
            "worker-late-batch-error"
        } else {
            "worker-all-batches"
        },
        serde_json::json!({"result":format!("{result:?}"),"prefixFrames":total,"sourceDrops":probe.drops.load(Ordering::Acquire),"stopAttempts":probe.stop_attempts.load(Ordering::Acquire)}),
    );
    assert_eq!(probe.drops.load(Ordering::Acquire), 2);
    assert_eq!(probe.stop_attempts.load(Ordering::Acquire), 2);
    for (n, chunk) in chunks.iter().enumerate() {
        let gain = [0.5, 0.4, 0.1][n];
        for (i, sample) in chunk.samples.iter().enumerate() {
            assert!((*sample - gain * stop_bound_tone(i / 2)).abs() < 0.000001);
        }
    }
    if fail_second {
        assert!(
            matches!(result,Err(AudioCaptureWorkerError::Source(ref s)) if s.contains("second finite tail batch failed"))
        );
        assert_eq!(total, 1920);
    } else {
        let r = result.unwrap();
        assert_eq!(r.duration_ns, Some(60_000_000));
        assert_eq!(r.captured_frames, 2880);
        assert_eq!(total, 2880);
    }
}
#[test]
fn mixed_stop_ready_bound_original_worker_drains_all_finite_batches() {
    check_stop_bound_worker(false);
}
#[test]
fn mixed_stop_ready_bound_later_batch_error_cannot_finish() {
    check_stop_bound_worker(true);
}

#[test]
fn mixed_stop_ready_bound_original_pipeline_backpressure_remains_strict() {
    let (source, probe) = stop_bound_source(true, true, 2_000_000_000);
    let pipeline = Arc::new(AudioPipeline::new(0));
    let worker = AudioCaptureWorker::spawn_with_factory(
        move |_| Ok(source),
        RecordingSessionClock::new(),
        Arc::clone(&pipeline),
    )
    .unwrap();
    wait_stop_bound(|| worker.is_finished() || pipeline.stats().unwrap().accepted_chunks == 1);
    let result = worker.stop();
    let mut frames = 0;
    while let Some(x) = pipeline.pop().unwrap() {
        frames += x.chunk.frame_count;
    }
    stop_bound_observation(
        "worker-backpressure",
        serde_json::json!({"result":format!("{result:?}"),"prefixFrames":frames,"sourceDrops":probe.drops.load(Ordering::Acquire)}),
    );
    assert_eq!(
        result,
        Err(AudioCaptureWorkerError::Pipeline(
            AudioPipelineError::Backpressure
        ))
    );
    assert_eq!(frames, 48_000);
    assert_eq!(probe.drops.load(Ordering::Acquire), 2);
}
#[test]
fn mixed_stop_ready_bound_both_stops_attempted_on_true_native_error() {
    let (mut source, probe) = stop_bound_source(true, true, 2_000_000_000);
    source.microphone.fail_stop = true;
    let result = source.stop_capture();
    drop(source);
    stop_bound_observation(
        "source-native-error",
        serde_json::json!({"result":format!("{result:?}"),"stopAttempts":probe.stop_attempts.load(Ordering::Acquire),"sourceDrops":probe.drops.load(Ordering::Acquire)}),
    );
    assert_eq!(
        result,
        Err(MixedAudioSourceError::Microphone(
            "microphone stop failed".into()
        ))
    );
    assert_eq!(probe.stop_attempts.load(Ordering::Acquire), 2);
    assert_eq!(probe.drops.load(Ordering::Acquire), 2);
}
