#[test]
fn wasapi_qpc_original_mixer_keeps_contiguous_pcm_on_audio_frame_grid() {
    let mut mixer = AudioMixer::default();
    for input in [MixerInput::System, MixerInput::Microphone] {
        for (sequence, captured_at_ns) in [0, 13_333_300].into_iter().enumerate() {
            mixer.push(input, CapturedAudioChunk { sequence: sequence as u64, captured_at_ns, format: AudioFormat::normalized(2), frame_count: 640, samples: vec![0.25; 1280].into_boxed_slice() }).unwrap();
        }
    }
    mixer.finish(26_666_666, 26_666_666).unwrap();
    let first = mixer.pop_ready().unwrap().unwrap();
    let last = mixer.pop_ready().unwrap().unwrap();
    assert_eq!((first.captured_at_ns, first.frame_count), (0, 960));
    assert_eq!((last.captured_at_ns, last.frame_count), (20_000_000, 320));
    assert!(first.samples.iter().chain(last.samples.iter()).all(|value| *value == 0.25));
    assert!(mixer.pop_ready().unwrap().is_none());
}
