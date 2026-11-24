use super::*;

#[test]
fn audio_advance_keeps_native_replacement_in_the_same_slot() {
    let mut encoder = Vp9PacketEncoder::new(2, 2, 10, 1).unwrap();
    let mut timestamps = Vec::new();
    let mut emit = |_: &[u8], timestamp, _: bool| {
        timestamps.push(timestamp);
        Ok(())
    };
    encoder
        .push_rgba(&solid_rgba(2, 2, [0, 0, 0]), 0, &mut emit)
        .unwrap();
    encoder
        .advance_for_audio(50_000_000, 50_000_000, &mut emit)
        .unwrap();
    assert_eq!(encoder.submitted_frames, 0);
    encoder
        .push_rgba(&solid_rgba(2, 2, [255, 255, 255]), 50_000_000, &mut emit)
        .unwrap();
    assert_eq!(encoder.pending.as_ref().unwrap().image.y, [235; 4]);
    encoder
        .advance_for_audio(100_000_000, 150_000_000, &mut emit)
        .unwrap();
    encoder
        .push_rgba(&solid_rgba(2, 2, [255, 0, 0]), 150_000_000, &mut emit)
        .unwrap();
    assert_eq!(encoder.pending.as_ref().unwrap().image.y, [63; 4]);
    assert_eq!(encoder.finish(200_000_000, &mut emit).unwrap(), 2);
    assert_eq!(timestamps, [0, 100_000_000]);
}

#[test]
fn audio_advance_respects_future_video_bound_and_preserves_native_pts() {
    let mut encoder = Vp9PacketEncoder::new(2, 2, 10, 1).unwrap();
    let mut timestamps = Vec::new();
    let mut emit = |_: &[u8], timestamp, _: bool| {
        timestamps.push(timestamp);
        Ok(())
    };
    encoder
        .push_rgba(&solid_rgba(2, 2, [0, 0, 0]), 0, &mut emit)
        .unwrap();
    encoder
        .advance_for_audio(900_000_000, 500_000_000, &mut emit)
        .unwrap();
    assert_eq!(encoder.submitted_frames, 5);
    assert_eq!(encoder.pending.as_ref().unwrap().slot, 5);
    assert_eq!(encoder.last_presentation_ns, Some(0));
    encoder
        .push_rgba(&solid_rgba(2, 2, [255, 255, 255]), 500_000_000, &mut emit)
        .unwrap();
    assert_eq!(encoder.finish(600_000_000, &mut emit).unwrap(), 6);
    assert_eq!(
        timestamps,
        [
            0,
            100_000_000,
            200_000_000,
            300_000_000,
            400_000_000,
            500_000_000
        ]
    );
}

#[test]
fn exact_finish_after_audio_advance_does_not_encode_an_extra_placeholder() {
    let mut encoder = Vp9PacketEncoder::new(2, 2, 1, 1).unwrap();
    let mut timestamps = Vec::new();
    let mut emit = |_: &[u8], timestamp, _: bool| {
        timestamps.push(timestamp);
        Ok(())
    };
    encoder
        .push_rgba(&solid_rgba(2, 2, [0, 0, 0]), 0, &mut emit)
        .unwrap();
    encoder
        .advance_for_audio(1_900_000_000, u64::MAX, &mut emit)
        .unwrap();
    assert_eq!(encoder.pending.as_ref().unwrap().slot, 2);
    assert_eq!(encoder.finish(2_000_000_000, &mut emit).unwrap(), 2);
    assert_eq!(timestamps, [0, 1_000_000_000]);
}

#[test]
fn exact_boundary_after_audio_advance_accepts_the_real_next_frame() {
    let mut encoder = Vp9PacketEncoder::new(2, 2, 1, 1).unwrap();
    let mut timestamps = Vec::new();
    let mut emit = |_: &[u8], timestamp, _: bool| {
        timestamps.push(timestamp);
        Ok(())
    };
    encoder
        .push_rgba(&solid_rgba(2, 2, [0, 0, 0]), 0, &mut emit)
        .unwrap();
    encoder
        .advance_for_audio(1_900_000_000, 2_000_000_000, &mut emit)
        .unwrap();
    encoder.flush_until(2_000_000_000, &mut emit).unwrap();
    assert_eq!(encoder.submitted_frames, 2);
    encoder
        .push_rgba(&solid_rgba(2, 2, [255, 255, 255]), 2_000_000_000, &mut emit)
        .unwrap();
    assert_eq!(encoder.pending.as_ref().unwrap().image.y, [235; 4]);
    assert_eq!(encoder.finish(3_000_000_000, &mut emit).unwrap(), 3);
    assert_eq!(timestamps, [0, 1_000_000_000, 2_000_000_000]);
}
