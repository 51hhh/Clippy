use crate::recording::audio::AudioPipeline;
use crate::recording::audio_worker::AudioCaptureWorker;
use crate::recording::clock::RecordingSessionClock;

fn assert_skew_pcm(chunks: &[CapturedAudioChunk], gains: &[f32]) {
    assert_eq!(chunks.len(),gains.len());
    for (chunk,gain) in chunks.iter().zip(gains) {
        assert_eq!(chunk.frame_count,960);
        for (index,sample) in chunk.samples.iter().enumerate() {
            assert!((*sample-*gain*skew_tone(index/2)).abs()<0.000001,"有效 PCM 样本不能丢失或平移");
        }
    }
}

fn check_skew_resume(system_earlier: bool) {
    let (mut source,_)=skew_source(SkewMode::Resume,system_earlier); source.start_capture().unwrap();
    let initial=source.capture_next_available(Duration::ZERO).unwrap().unwrap(); assert_skew_pcm(&[initial],&[0.5]);
    source.pause_capture().unwrap(); let bound=source.resume_capture().unwrap();
    let first=source.capture_next_available(Duration::ZERO);
    skew_observation(if system_earlier {"resume-system-earlier"} else {"resume-microphone-earlier"},serde_json::json!({"bound":bound,"first":format!("{:?}",first.as_ref().map(|chunk|chunk.as_ref().map(|chunk|(chunk.captured_at_ns,chunk.frame_count))))}));
    let first=first.unwrap().unwrap(); let second=source.capture_next_available(Duration::ZERO).unwrap().unwrap();
    assert_eq!(bound,1_000_000_000); assert_eq!(first.captured_at_ns,1_000_000_000); assert_eq!(second.captured_at_ns,1_020_000_000);
    assert_skew_pcm(&[first,second],&[if system_earlier {0.4} else {0.1},0.5]);
}

#[test]
fn mixed_control_skew_system_earlier_resume_preserves_first_pcm_time() { check_skew_resume(true); }
#[test]
fn mixed_control_skew_microphone_earlier_resume_preserves_first_pcm_time() { check_skew_resume(false); }

fn check_skew_stop(system_earlier: bool) {
    let (mut source,drops)=skew_source(SkewMode::StopTail,system_earlier); source.start_capture().unwrap();
    let stop=source.stop_capture().unwrap(); let tails=source.take_stopped_chunks().unwrap();
    skew_observation(if system_earlier {"stop-system-earlier"} else {"stop-microphone-earlier"},serde_json::json!({"stop":stop,"frames":tails.iter().map(|chunk|chunk.frame_count).sum::<u32>(),"chunks":tails.len()}));
    assert_eq!(stop,40_000_000); assert_skew_pcm(&tails,&[0.5,if system_earlier {0.1} else {0.4}]);
    assert_eq!(tails[0].captured_at_ns,0); assert_eq!(tails[1].captured_at_ns,20_000_000);
    drop(source); assert_eq!(drops.load(Ordering::Acquire),2);
}

#[test]
fn mixed_control_skew_system_earlier_stop_keeps_later_microphone_tail() { check_skew_stop(true); }
#[test]
fn mixed_control_skew_microphone_earlier_stop_keeps_later_system_tail() { check_skew_stop(false); }

#[test]
fn mixed_control_skew_worker_resume_preserves_complete_prefix_and_both_sources() {
    let (source,drops)=skew_source(SkewMode::Resume,true); let pipeline=Arc::new(AudioPipeline::new(0));
    let worker=AudioCaptureWorker::spawn_with_factory(move |_|Ok(source),RecordingSessionClock::new(),Arc::clone(&pipeline)).unwrap();
    wait_skew(||worker.is_finished()||pipeline.stats().unwrap().accepted_chunks==1);
    worker.pause().unwrap(); worker.resume().unwrap();
    wait_skew(||worker.is_finished()||pipeline.stats().unwrap().accepted_chunks==3);
    let result=worker.stop(); let mut prefix=Vec::new(); while let Some(chunk)=pipeline.pop().unwrap() {prefix.push(chunk.chunk);}
    skew_observation("worker-resume",serde_json::json!({"result":format!("{result:?}"),"prefixFrames":prefix.iter().map(|chunk|chunk.frame_count).sum::<u32>(),"sourceDrops":drops.load(Ordering::Acquire)}));
    let report=result.unwrap(); assert_eq!(report.captured_frames,2880); assert_eq!(report.duration_ns,Some(60_000_000));
    assert_skew_pcm(&prefix,&[0.5,0.4,0.5]); assert_eq!(drops.load(Ordering::Acquire),2);
}

#[test]
fn mixed_control_skew_worker_stop_preserves_later_tail_instead_of_silence() {
    let (source,drops)=skew_source(SkewMode::StopTail,true); let pipeline=Arc::new(AudioPipeline::new(0));
    let worker=AudioCaptureWorker::spawn_with_factory(move |_|Ok(source),RecordingSessionClock::new(),Arc::clone(&pipeline)).unwrap();
    let result=worker.stop(); let mut prefix=Vec::new(); while let Some(chunk)=pipeline.pop().unwrap() {prefix.push(chunk.chunk);}
    skew_observation("worker-stop",serde_json::json!({"result":format!("{result:?}"),"prefixFrames":prefix.iter().map(|chunk|chunk.frame_count).sum::<u32>(),"sourceDrops":drops.load(Ordering::Acquire)}));
    let report=result.unwrap(); assert_eq!(report.captured_frames,1920); assert_eq!(report.duration_ns,Some(40_000_000));
    assert_skew_pcm(&prefix,&[0.5,0.1]); assert_eq!(drops.load(Ordering::Acquire),2);
}

#[test]
fn mixed_control_skew_later_peer_stop_cannot_hide_own_early_stop() {
    for system_earlier in [true,false] {
        let (mut source,_)=skew_source(SkewMode::StopTail,system_earlier); source.start_capture().unwrap();
        if system_earlier {source.system.stop_at=10_000_000;} else {source.microphone.stop_at=10_000_000;}
        assert_eq!(source.stop_capture(),Err(MixedAudioSourceError::Mixer(AudioMixerError::StopBeforeLastSample)));
    }
}
