use super::*;

#[test]
fn pcm_capacity_accounts_for_partial_packet_in_mono_and_stereo() {
    for channels in [1, 2] {
        let mut encoder = OpusPacketEncoder::new(channels).unwrap();
        assert_eq!(encoder.frame_capacity_for_packets(32).unwrap(), 30_720);
        assert!(encoder
            .push(queued(0, 0, 48, channels, 0))
            .unwrap()
            .is_empty());
        assert_eq!(encoder.frame_capacity_for_packets(0).unwrap(), 0);
        assert_eq!(encoder.frame_capacity_for_packets(1).unwrap(), 912);
        assert_eq!(encoder.frame_capacity_for_packets(32).unwrap(), 30_672);
        let packets = encoder
            .push(queued(1, 1_000_000, 912, channels, 48))
            .unwrap();
        assert_eq!(packets.len(), 1);
        assert_eq!(encoder.frame_capacity_for_packets(1).unwrap(), 960);
        let done = encoder.finish().unwrap();
        assert_eq!(done.stats.real_frames, 960);
    }
}

#[test]
fn capacity_overflow_is_rejected_without_mutating_encoder() {
    let encoder = OpusPacketEncoder::new(2).unwrap();
    assert_eq!(
        encoder.frame_capacity_for_packets(usize::MAX),
        Err(OpusWebmError::TimelineOverflow)
    );
    assert_eq!(encoder.frame_capacity_for_packets(1).unwrap(), 960);
}
