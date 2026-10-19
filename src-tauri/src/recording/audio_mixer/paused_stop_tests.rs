use crate::recording::audio::{AudioPipeline, AudioPipelineError};
use crate::recording::audio_worker::{AudioCaptureWorker, AudioCaptureWorkerError};
use crate::recording::clock::RecordingSessionClock;

fn assert_paused_prefix(chunks: &[CapturedAudioChunk]) {
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].frame_count, 960);
    for (index, sample) in chunks[0].samples.iter().enumerate() {
        assert!((*sample - 0.5 * paused_stop_tone(index / 2)).abs() < 0.000001);
    }
}

fn check_paused_source(initial: bool, system_earlier: bool) {
    let (mut source, probe) = paused_stop_source(initial, system_earlier);
    source.start_capture().unwrap();
    if initial {
        let first = source
            .capture_next_available(Duration::ZERO)
            .unwrap()
            .unwrap();
        assert_paused_prefix(&[first]);
    }
    assert_eq!(source.pause_capture().unwrap(), 20_000_000);
    let stop = source.stop_capture();
    let tails = source.take_stopped_chunks();
    drop(source);
    paused_stop_observation(
        if !initial {
            "source-before-first"
        } else if system_earlier {
            "source-system-earlier"
        } else {
            "source-microphone-earlier"
        },
        serde_json::json!({"stop":format!("{stop:?}"),"tailFrames":tails.as_ref().ok().map(|chunks|chunks.iter().map(|chunk|u64::from(chunk.frame_count)).sum::<u64>()),"stopAttempts":probe.stop_attempts.load(Ordering::Acquire),"sourceDrops":probe.drops.load(Ordering::Acquire)}),
    );
    assert_eq!(stop.unwrap(), 2_040_000_000);
    assert!(tails.unwrap().is_empty(), "暂停区间不能制造stopped PCM");
    assert_eq!(probe.drops.load(Ordering::Acquire), 2);
}

#[test]
fn mixed_paused_stop_system_earlier_does_not_manufacture_pcm() {
    check_paused_source(true, true);
}

#[test]
fn mixed_paused_stop_microphone_earlier_does_not_manufacture_pcm() {
    check_paused_source(true, false);
}

#[test]
fn mixed_paused_stop_before_first_packet_returns_no_pcm() {
    check_paused_source(false, true);
}

fn check_paused_worker(initial: bool) {
    let (source, probe) = paused_stop_source(initial, true);
    let pipeline = Arc::new(AudioPipeline::new(0));
    let worker = AudioCaptureWorker::spawn_with_factory(
        move |_| Ok(source),
        RecordingSessionClock::new(),
        Arc::clone(&pipeline),
    )
    .unwrap();
    wait_paused_stop(|| {
        probe.started.load(Ordering::Acquire) == 2
            && (!initial || pipeline.stats().unwrap().accepted_chunks == 1)
    });
    worker.pause().unwrap();
    let result = worker.stop();
    let mut prefix = Vec::new();
    while let Some(chunk) = pipeline.pop().unwrap() {
        prefix.push(chunk.chunk);
    }
    paused_stop_observation(
        if initial {
            "worker-prefix"
        } else {
            "worker-before-first"
        },
        serde_json::json!({"result":format!("{result:?}"),"prefixFrames":prefix.iter().map(|chunk|u64::from(chunk.frame_count)).sum::<u64>(),"sourceDrops":probe.drops.load(Ordering::Acquire),"stopAttempts":probe.stop_attempts.load(Ordering::Acquire),"stats":format!("{:?}",pipeline.stats())}),
    );
    if initial {
        assert_paused_prefix(&prefix);
    } else {
        assert!(prefix.is_empty());
    }
    assert_eq!(probe.drops.load(Ordering::Acquire), 2);
    let report = result.unwrap();
    assert_eq!(report.duration_ns, Some(20_000_000));
    assert_eq!(report.captured_frames, if initial { 960 } else { 0 });
}

#[test]
fn mixed_paused_stop_original_worker_retains_complete_active_prefix() {
    check_paused_worker(true);
}

#[test]
fn mixed_paused_stop_original_worker_without_first_pcm_finishes() {
    check_paused_worker(false);
}

#[test]
fn mixed_paused_stop_unexpected_native_tail_still_fails() {
    for system in [true, false] {
        let (mut source, probe) = paused_stop_source(true, true);
        if system {
            source.system.unexpected_tail = true;
        } else {
            source.microphone.unexpected_tail = true;
        }
        let pipeline = Arc::new(AudioPipeline::new(0));
        let worker = AudioCaptureWorker::spawn_with_factory(
            move |_| Ok(source),
            RecordingSessionClock::new(),
            Arc::clone(&pipeline),
        )
        .unwrap();
        wait_paused_stop(|| pipeline.stats().unwrap().accepted_chunks == 1);
        worker.pause().unwrap();
        let result = worker.stop();
        let mut prefix = Vec::new();
        while let Some(chunk) = pipeline.pop().unwrap() {
            prefix.push(chunk.chunk);
        }
        paused_stop_observation(
            if system {
                "unexpected-system-tail"
            } else {
                "unexpected-microphone-tail"
            },
            serde_json::json!({"result":format!("{result:?}"),"sourceDrops":probe.drops.load(Ordering::Acquire)}),
        );
        assert!(result.is_err(), "暂停期间真实尾块不能被静默忽略");
        assert_paused_prefix(&prefix);
        assert_eq!(probe.drops.load(Ordering::Acquire), 2);
    }
}

#[test]
fn mixed_paused_stop_both_sources_attempt_stop_and_keep_failure_identity() {
    for system in [true, false] {
        let (mut source, probe) = paused_stop_source(true, true);
        source.start_capture().unwrap();
        source
            .capture_next_available(Duration::ZERO)
            .unwrap()
            .unwrap();
        source.pause_capture().unwrap();
        if system {
            source.system.fail_stop = true;
        } else {
            source.microphone.fail_stop = true;
        }
        let result = source.stop_capture();
        if system {
            assert_eq!(
                result,
                Err(MixedAudioSourceError::System("system stop failed".into()))
            );
        } else {
            assert_eq!(
                result,
                Err(MixedAudioSourceError::Microphone(
                    "microphone stop failed".into()
                ))
            );
        }
        assert_eq!(probe.stop_attempts.load(Ordering::Acquire), 2);
        drop(source);
        assert_eq!(probe.drops.load(Ordering::Acquire), 2);
    }
}

#[test]
fn mixed_paused_stop_native_control_regression_is_not_normalized() {
    let (mut source, probe) = paused_stop_source(true, true);
    source.start_capture().unwrap();
    source
        .capture_next_available(Duration::ZERO)
        .unwrap()
        .unwrap();
    source.pause_capture().unwrap();
    source.system.stop_at = 10_000_000;
    assert_eq!(
        source.stop_capture(),
        Err(MixedAudioSourceError::Mixer(
            AudioMixerError::ControlTimestampRegressed
        ))
    );
    assert_eq!(probe.stop_attempts.load(Ordering::Acquire), 2);
}

#[test]
fn mixed_paused_stop_invalid_tail_keeps_format_error() {
    let (mut source, _) = paused_stop_source(true, true);
    source.start_capture().unwrap();
    source
        .capture_next_available(Duration::ZERO)
        .unwrap()
        .unwrap();
    source.pause_capture().unwrap();
    source.microphone.invalid_tail = true;
    assert_eq!(
        source.stop_capture(),
        Err(MixedAudioSourceError::Mixer(
            AudioMixerError::UnsupportedFormat
        ))
    );
}

#[test]
fn mixed_paused_stop_original_worker_still_rejects_paused_pcm() {
    let (mut source, probe) = paused_stop_source(true, true);
    source.system.unexpected_tail = true;
    let single = source.system;
    // 单源直接进入原worker，证明不能靠放宽worker暂停保护修复混音源。
    drop(source.microphone);
    let pipeline = Arc::new(AudioPipeline::new(0));
    let worker = AudioCaptureWorker::spawn_with_factory(
        move |_| Ok(single),
        RecordingSessionClock::new(),
        Arc::clone(&pipeline),
    )
    .unwrap();
    wait_paused_stop(|| pipeline.stats().unwrap().accepted_chunks == 1);
    worker.pause().unwrap();
    assert_eq!(
        worker.stop().unwrap_err(),
        AudioCaptureWorkerError::Pipeline(AudioPipelineError::AlreadyPaused)
    );
    assert_eq!(probe.drops.load(Ordering::Acquire), 2);
}
