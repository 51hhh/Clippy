use super::*;

#[test]
fn audio_waits_for_a_future_video_in_the_same_webm_timecode() {
    let encoder = OpusPacketEncoder::new(2).unwrap();
    let mut interleaver =
        AvPacketInterleaver::new(Cursor::new(Vec::new()), 2, 2, encoder.track_config()).unwrap();
    interleaver.enqueue_video(&[1], 0, true).unwrap();
    interleaver.flush_ready(33_333_333, 6_500_000).unwrap();
    interleaver.enqueue_video(&[2], 33_333_333, false).unwrap();
    interleaver.flush_ready(66_666_666, 46_500_000).unwrap();
    interleaver.enqueue_audio(audio_packet(46_500_000)).unwrap();
    interleaver.flush_ready(66_666_666, 66_500_000).unwrap();
    interleaver.enqueue_audio(audio_packet(66_500_000)).unwrap();
    interleaver.flush_ready(66_666_666, 86_500_000).unwrap();
    assert_eq!(interleaver.pending_counts(), (0, 1));
    interleaver.enqueue_video(&[3], 66_666_666, false).unwrap();
    interleaver.flush_ready(100_000_000, 86_500_000).unwrap();
    assert_eq!(interleaver.pending_counts(), (0, 0));
    let output = interleaver.finish(100_000_000).unwrap();
    assert_eq!(output.video_frame_count, 3);
    assert_eq!(output.audio_packet_count, 2);
}

#[test]
fn queued_video_precedes_audio_after_timecode_quantization() {
    let encoder = OpusPacketEncoder::new(2).unwrap();
    let mut interleaver =
        AvPacketInterleaver::new(Cursor::new(Vec::new()), 2, 2, encoder.track_config()).unwrap();
    interleaver.enqueue_audio(audio_packet(66_500_000)).unwrap();
    interleaver.enqueue_video(&[1], 66_666_666, true).unwrap();
    interleaver.flush_ready(66_666_666, 86_500_000).unwrap();
    assert_eq!(interleaver.pending_counts(), (0, 1));
    interleaver.flush_ready(100_000_000, 86_500_000).unwrap();
    let output = interleaver.finish(100_000_000).unwrap();
    assert_eq!(output.video_frame_count, 1);
    assert_eq!(output.audio_packet_count, 1);
}
