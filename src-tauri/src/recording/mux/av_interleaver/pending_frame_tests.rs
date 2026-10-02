use super::*;

#[test]
fn observed_audio_capacity_preserves_thirty_two_packet_failure_closed_guard() {
    let encoder = OpusPacketEncoder::new(2).unwrap();
    let mut mux =
        AvPacketInterleaver::new(Cursor::new(Vec::new()), 2, 2, encoder.track_config()).unwrap();
    for index in 0..32 {
        assert_eq!(mux.available_audio_packets(), 32 - index);
        mux.enqueue_audio(audio_packet(index as u64 * 20_000_000 + 6_500_000))
            .unwrap();
    }
    assert_eq!(mux.available_audio_packets(), 0);
    assert_eq!(
        mux.enqueue_audio(audio_packet(646_500_000)),
        Err(OpusWebmError::InterleaveQueueFull)
    );
    mux.enqueue_video(&[1], 0, true).unwrap();
    mux.flush_ready(1_000_000_000, 646_500_000).unwrap();
    assert_eq!(mux.available_audio_packets(), 32);
    assert_eq!(mux.pending_counts(), (0, 0));
}

#[test]
fn capacity_observation_does_not_relax_same_timestamp_video_before_audio() {
    let encoder = OpusPacketEncoder::new(1).unwrap();
    let mut mux =
        AvPacketInterleaver::new(Cursor::new(Vec::new()), 2, 2, encoder.track_config()).unwrap();
    mux.enqueue_audio(audio_packet(10_000_000)).unwrap();
    assert_eq!(mux.available_audio_packets(), 31);
    mux.flush_ready(10_000_000, 30_000_000).unwrap();
    assert_eq!(mux.available_audio_packets(), 31);
    mux.enqueue_video(&[1], 10_000_000, true).unwrap();
    mux.flush_ready(20_000_000, 30_000_000).unwrap();
    assert_eq!(mux.available_audio_packets(), 32);
}
