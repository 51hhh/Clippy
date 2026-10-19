use crate::recording::audio::{AudioPipeline, AudioTimestampPrecision};
use crate::recording::audio_worker::AudioCaptureWorker;
use crate::recording::clock::RecordingSessionClock;
use std::path::PathBuf;

#[test]
fn mixed_frame_boundary_startup_keeps_its_pcm_lower_bound() {
    let origin = 1_000_005_000;
    let (mut source, _) = boundary_source(origin, origin, 960, origin + 20_000_000, false);
    let bound = source.start_capture().unwrap().unwrap();
    let output = source.capture_next_available(Duration::ZERO).unwrap().unwrap();
    boundary_observation("startup", serde_json::json!({"bound":bound,"outputAt":output.captured_at_ns,"frames":output.frame_count}));
    assert_eq!(bound, origin); assert_eq!(output.captured_at_ns, bound); assert_eq!(output.frame_count, 960);
    assert!(output.samples.iter().all(|sample| *sample == 0.5));
}

#[test]
fn mixed_frame_boundary_resume_first_pcm_enters_original_exact_pipeline() {
    let (mut source, _) = boundary_source(0, 0, 960, 20_000_000, false);
    source.start_capture().unwrap(); let pipeline = AudioPipeline::new(0);
    pipeline.push(source.capture_next_available(Duration::ZERO).unwrap().unwrap()).unwrap();
    pipeline.pause(source.pause_capture().unwrap()).unwrap();
    let bound = source.resume_capture().unwrap(); pipeline.resume(bound).unwrap();
    let output = source.capture_next_available(Duration::ZERO).unwrap().unwrap();
    let at = output.captured_at_ns; let result = pipeline.push(output);
    boundary_observation("resume", serde_json::json!({"bound":bound,"outputAt":at,"result":format!("{result:?}"),"accepted":pipeline.stats().unwrap().accepted_chunks}));
    result.unwrap(); assert_eq!(at, bound); assert_eq!(pipeline.stats().unwrap().accepted_chunks, 2);
    assert_eq!(pipeline.finish(source.stop_capture().unwrap()).unwrap(), 40_000_000);
}

#[test]
fn mixed_frame_boundary_tail_finish_covers_all_original_samples() {
    let (mut source, _) = boundary_source(1_000_000_000, 1_000_020_000, 640, 1_013_353_333, true);
    source.start_capture().unwrap(); let pipeline = AudioPipeline::new(1_000_000_000);
    let stop = source.stop_capture().unwrap(); let tails = source.take_stopped_chunks().unwrap();
    assert_eq!(tails.len(), 1); assert_eq!(tails[0].frame_count, 641);
    assert_eq!(&tails[0].samples[..2], &[0.0, 0.0]); assert!(tails[0].samples[2..].iter().all(|sample| *sample == 0.5));
    let end = tails[0].captured_at_ns + u64::from(tails[0].frame_count) * 1_000_000_000 / 48_000;
    pipeline.push(tails.into_iter().next().unwrap()).unwrap(); let result = pipeline.finish(stop);
    boundary_observation("tail-finish", serde_json::json!({"rawStop":1_013_353_333u64,"stop":stop,"pcmEnd":end,"result":format!("{result:?}")}));
    assert_eq!(result.unwrap(), 13_354_166); assert!(stop >= end);
}

#[test]
fn mixed_frame_boundary_worker_stops_with_complete_tail_and_drops_both_sources() {
    let (source, drops) = boundary_source(1_000_000_000, 1_000_020_000, 640, 1_013_353_333, true);
    let pipeline = Arc::new(AudioPipeline::new(1_000_000_000));
    let worker = AudioCaptureWorker::spawn_with_factory(move |_| Ok(source), RecordingSessionClock::new(), Arc::clone(&pipeline)).unwrap();
    let result = worker.stop(); let prefix = pipeline.pop().unwrap().unwrap();
    boundary_observation("worker", serde_json::json!({"result":format!("{result:?}"),"prefixFrames":prefix.chunk.frame_count,"sourceDrops":drops.load(Ordering::Acquire)}));
    let report = result.unwrap(); assert_eq!(report.captured_frames, 641); assert_eq!(report.duration_ns, Some(13_354_166));
    assert_eq!(prefix.chunk.frame_count, 641); assert!(pipeline.pop().unwrap().is_none()); assert_eq!(drops.load(Ordering::Acquire), 2);
}

#[test]
fn mixed_frame_boundary_pause_and_immediate_resume_cover_derived_pcm() {
    let raw_pause = 1_013_353_333;
    let (mut source, _) = boundary_source(1_000_000_000, 1_000_020_000, 640, raw_pause, false);
    source.system.resume_at = raw_pause + 1; source.microphone.resume_at = raw_pause + 1;
    source.start_capture().unwrap(); let pipeline = AudioPipeline::new(1_000_000_000);
    pipeline.push(source.capture_next_available(Duration::ZERO).unwrap().unwrap()).unwrap();
    let pause = source.pause_capture().unwrap(); let paused = pipeline.pause(pause);
    boundary_observation("pause", serde_json::json!({"rawPause":raw_pause,"pause":pause,"result":format!("{paused:?}")}));
    paused.unwrap(); let resume = source.resume_capture().unwrap(); pipeline.resume(resume).unwrap();
    let output = source.capture_next_available(Duration::ZERO).unwrap().unwrap();
    boundary_observation("immediate-resume", serde_json::json!({"rawResume":raw_pause+1,"resume":resume,"outputAt":output.captured_at_ns,"frames":output.frame_count}));
    assert!(resume > pause); assert_eq!(output.captured_at_ns, resume); assert_eq!(output.frame_count, 640);
    assert!(output.samples.iter().all(|sample| *sample == 0.5)); pipeline.push(output).unwrap();
    pipeline.finish(source.stop_capture().unwrap()).unwrap();
}

#[test]
fn mixed_frame_boundary_raw_control_regression_is_not_hidden_by_pcm_end() {
    let (mut source, _) = boundary_source(0, 0, 960, 20_000_000, false);
    source.start_capture().unwrap(); source.capture_next_available(Duration::ZERO).unwrap().unwrap();
    source.control_timestamp_ns().unwrap(); source.system.control.store(19_999_999, Ordering::Release);
    let result = source.control_timestamp_ns();
    boundary_observation("raw-control-regression", serde_json::json!({"result":format!("{result:?}")}));
    assert_eq!(result, Err(MixedAudioSourceError::Mixer(AudioMixerError::ControlTimestampRegressed)));
}

#[test]
fn mixed_frame_boundary_invalid_native_tail_still_fails_before_normalization() {
    let (mut source, _) = boundary_source(1_000_000_000, 1_000_020_000, 640, 1_013_300_000, true);
    source.start_capture().unwrap();
    assert_eq!(source.stop_capture(), Err(MixedAudioSourceError::Mixer(AudioMixerError::StopBeforeLastSample)));
}

#[test]
fn mixed_frame_boundary_output_end_overflow_keeps_cursor_and_sequence() {
    let origin = u64::MAX - 5_000_000; let mut mixer = AudioMixer::default(); mixer.restart_at(origin).unwrap();
    for input in [MixerInput::System, MixerInput::Microphone] {
        mixer.push(input, CapturedAudioChunk { sequence:0, captured_at_ns:origin, format:AudioFormat::normalized(2), frame_count:960, samples:vec![0.5;1920].into_boxed_slice() }).unwrap();
    }
    let before = (mixer.output_cursor_frame, mixer.next_sequence); let result = mixer.pop_ready();
    boundary_observation("output-overflow", serde_json::json!({"result":format!("{:?}",result.as_ref().map(|value| value.as_ref().map(|chunk|chunk.frame_count)))}));
    assert!(matches!(result, Err(AudioMixerError::TimelineOverflow))); assert_eq!((mixer.output_cursor_frame,mixer.next_sequence),before);
}

#[test]
fn mixed_frame_boundary_remains_exact_and_duplicate_input_is_rejected() {
    let (source, _) = boundary_source(0, 0, 960, 20_000_000, false);
    assert_eq!(source.timestamp_precision(), AudioTimestampPrecision::Exact);
    let mut mixer = AudioMixer::default(); mixer.push(MixerInput::System, constant(1,0,960,2,0.5)).unwrap();
    assert_eq!(mixer.push(MixerInput::System,constant(1,960,960,2,0.5)),Err(AudioMixerError::SequenceNotIncreasing));
}
