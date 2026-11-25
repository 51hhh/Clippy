use crate::recording::audio::{AudioPipeline, AudioPipelineDrain, AudioPushOutcome};

fn marked_pcm(frames: u32) -> Vec<f32> {
    (0..frames)
        .flat_map(|frame| [frame as f32 / 4096.0, frame as f32 / 4096.0 + 0.0625])
        .collect()
}

fn flattened(packet: &PacketChunks) -> Vec<f32> {
    packet.chunks.iter().flat_map(|chunk| chunk.samples.iter().copied()).collect()
}

#[test]
fn wasapi_activation_tail_retains_crossing_packet_pcm_and_original_identity() {
    let pcm = marked_pcm(960);
    let original = packet_to_chunks(7, 100_000_000, 960, Some(&pcm)).unwrap();
    let retained = packet_after_activation(original, 110_000_000).unwrap();
    assert_eq!(retained.chunks.len(), 1);
    let chunk = &retained.chunks[0];
    assert_eq!(chunk.sequence, 7);
    assert_eq!(chunk.captured_at_ns, 110_000_000);
    assert_eq!(chunk.frame_count, 480);
    assert_eq!(chunk.format, AudioFormat::normalized(2));
    assert_eq!(&*chunk.samples, &pcm[960..]);
    assert_eq!((retained.next_sequence, retained.end_ns), (8, 120_000_000));
}

#[test]
fn wasapi_activation_tail_keeps_later_split_chunks_on_original_grid() {
    let pcm = marked_pcm(2400);
    let original = packet_to_chunks(9, 1_000_000_000, 2400, Some(&pcm)).unwrap();
    let retained = packet_after_activation(original, 1_010_000_000).unwrap();
    let shapes: Vec<_> = retained.chunks.iter().map(|chunk| {
        (chunk.sequence, chunk.captured_at_ns, chunk.frame_count)
    }).collect();
    assert_eq!(shapes, [(9, 1_010_000_000, 480), (10, 1_020_000_000, 960), (11, 1_040_000_000, 480)]);
    assert_eq!(flattened(&retained), pcm[960..]);
    assert_eq!((retained.next_sequence, retained.end_ns), (12, 1_050_000_000));
}

#[test]
fn wasapi_activation_tail_rounds_non_sample_boundary_up_without_backdating() {
    let pcm = marked_pcm(960);
    let original = packet_to_chunks(30, 0, 960, Some(&pcm)).unwrap();
    let retained = packet_after_activation(original, 20_834).unwrap();
    assert_eq!(retained.chunks.len(), 1);
    let chunk = &retained.chunks[0];
    assert_eq!((chunk.sequence, chunk.captured_at_ns, chunk.frame_count), (30, 41_666, 958));
    assert!(chunk.captured_at_ns >= 20_834);
    assert_eq!(&*chunk.samples, &pcm[4..]);
    assert_eq!((retained.next_sequence, retained.end_ns), (31, 20_000_000));
}

#[test]
fn wasapi_activation_tail_restored_pipeline_commits_original_valid_suffix() {
    let pcm = marked_pcm(960);
    let packet = packet_to_chunks(14, 100_000_000, 960, Some(&pcm)).unwrap();
    let mut retained = packet_after_activation(packet, 110_000_000).unwrap();
    let chunk = retained.chunks.pop_front().expect("恢复边界之后的真实 PCM 不能整包丢失");
    let pipeline = AudioPipeline::new(100_000_000);
    pipeline.pause(100_000_000).unwrap();
    pipeline.resume(110_000_000).unwrap();
    assert_eq!(pipeline.push(chunk).unwrap(), AudioPushOutcome::Queued {
        presentation_at_ns: 0, duration_ns: 10_000_000, gap_before_ns: 0,
    });
    assert_eq!(pipeline.finish(120_000_000).unwrap(), 10_000_000);
    let AudioPipelineDrain::Chunk(queued) = pipeline.pop_wait().unwrap() else {
        panic!("有效后缀应交给原编码输入队列");
    };
    assert_eq!((queued.chunk.sequence, queued.chunk.frame_count), (14, 480));
    assert_eq!(&*queued.chunk.samples, &pcm[960..]);
    assert!(matches!(pipeline.pop_wait().unwrap(), AudioPipelineDrain::Finished { duration_ns: 10_000_000 }));
}

#[test]
fn wasapi_activation_tail_silent_suffix_retains_exact_zero_samples() {
    let packet = packet_to_chunks(2, 200_000_000, 960, None).unwrap();
    let retained = packet_after_activation(packet, 210_000_000).unwrap();
    assert_eq!(retained.chunks.len(), 1);
    assert_eq!(retained.chunks[0].frame_count, 480);
    assert_eq!(retained.chunks[0].samples.len(), 960);
    assert!(retained.chunks[0].samples.iter().all(|sample| sample.to_bits() == 0));
    assert_eq!((retained.next_sequence, retained.end_ns), (3, 220_000_000));
}

#[test]
fn wasapi_activation_tail_wholly_old_packet_is_empty_with_counters_preserved() {
    let pcm = marked_pcm(960);
    for cutoff in [20_000_000, 21_000_000, u64::MAX] {
        let packet = packet_to_chunks(40, 0, 960, Some(&pcm)).unwrap();
        let retained = packet_after_activation(packet, cutoff).unwrap();
        assert!(retained.chunks.is_empty());
        assert_eq!((retained.next_sequence, retained.end_ns), (41, 20_000_000));
    }
}

#[test]
fn wasapi_activation_tail_at_or_after_inclusive_boundary_is_unchanged() {
    let pcm = marked_pcm(2400);
    for cutoff in [0, 99_999_999, 100_000_000] {
        let packet = packet_to_chunks(50, 100_000_000, 2400, Some(&pcm)).unwrap();
        let retained = packet_after_activation(packet, cutoff).unwrap();
        let shapes: Vec<_> = retained.chunks.iter().map(|chunk| {
            (chunk.sequence, chunk.captured_at_ns, chunk.frame_count)
        }).collect();
        assert_eq!(shapes, [(50, 100_000_000, 960), (51, 120_000_000, 960), (52, 140_000_000, 480)]);
        assert_eq!(flattened(&retained), pcm);
        assert_eq!((retained.next_sequence, retained.end_ns), (53, 150_000_000));
    }
}
