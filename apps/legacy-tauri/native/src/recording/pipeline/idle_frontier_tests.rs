use super::*;

#[test]
fn lower_bound_without_video_cannot_establish_epoch_or_count_frames() {
    let pipeline = RecordingPipeline::default();
    pipeline.publish_capture_lower_bound(10_000).unwrap();
    assert_eq!(pipeline.stats().unwrap().accepted_frames, 0);
    assert!(pipeline.state.lock().unwrap().capture_lower_bound.is_none());
    assert!(pipeline.push(frame(0, 100, 1)).is_ok());
}

#[test]
fn lower_bound_rejects_earlier_frame_without_consuming_valid_identity() {
    let pipeline = RecordingPipeline::default();
    pipeline.push(frame(0, 100, 1)).unwrap();
    pipeline.publish_capture_lower_bound(500).unwrap();
    assert_eq!(
        pipeline.push(frame(1, 499, 2)),
        Err(PipelineError::FrameBeforeLowerBound)
    );
    assert_eq!(
        pipeline.pause(499),
        Err(PipelineError::FrameBeforeLowerBound)
    );
    assert_eq!(
        pipeline.finish(499),
        Err(PipelineError::FrameBeforeLowerBound)
    );
    assert_eq!(
        pipeline.push(frame(1, 500, 2)).unwrap(),
        PushOutcome::Queued {
            presentation_at_ns: 400
        }
    );
    assert_eq!(pipeline.stats().unwrap().accepted_frames, 2);
}

#[test]
fn pause_ignores_old_frames_and_preserves_resumed_lower_bound() {
    let pipeline = RecordingPipeline::default();
    pipeline.push(frame(0, 100, 1)).unwrap();
    pipeline.publish_capture_lower_bound(200).unwrap();
    pipeline.pause(200).unwrap();
    pipeline.publish_capture_lower_bound(10_000).unwrap();
    assert_eq!(
        pipeline.push(frame(1, 0, 2)).unwrap(),
        PushOutcome::IgnoredWhilePaused
    );
    pipeline.resume(1_200).unwrap();
    pipeline.publish_capture_lower_bound(1_300).unwrap();
    assert_eq!(
        pipeline.push(frame(1, 1_300, 2)).unwrap(),
        PushOutcome::Queued {
            presentation_at_ns: 200
        }
    );
    assert_eq!(pipeline.stats().unwrap().dropped_by_backpressure, 0);
}

#[cfg(feature = "recording-opus-webm")]
#[test]
fn av_drain_coalesces_bound_and_keeps_frames_and_terminal_first() {
    let pipeline = RecordingPipeline::default();
    for sequence in 0..3 {
        pipeline.push(frame(sequence, 100 + sequence, 1)).unwrap();
    }
    for timestamp in 103..10_000 {
        pipeline.publish_capture_lower_bound(timestamp).unwrap();
    }
    assert_eq!(
        pipeline.stats().unwrap().queued_frames,
        FRAME_QUEUE_CAPACITY
    );
    for expected in 0..3 {
        let AvPipelineDrain::Frame(value) = pipeline.pop_wait_av().unwrap() else {
            panic!("先排真实帧");
        };
        assert_eq!(value.frame.sequence, expected);
    }
    assert!(matches!(
        pipeline.pop_wait_av().unwrap(),
        AvPipelineDrain::CaptureLowerBound {
            presentation_at_ns: 9_899
        }
    ));
    pipeline.publish_capture_lower_bound(10_000).unwrap();
    assert_eq!(pipeline.finish(10_000).unwrap(), 9_900);
    assert!(matches!(
        pipeline.pop_wait_av().unwrap(),
        AvPipelineDrain::Finished { duration_ns: 9_900 }
    ));
}

#[cfg(feature = "recording-opus-webm")]
#[test]
fn av_waiter_wakes_for_bound_then_abort_without_phantom_frames() {
    let pipeline = std::sync::Arc::new(RecordingPipeline::default());
    pipeline.push(frame(0, 100, 1)).unwrap();
    pipeline.pop().unwrap();
    let other = std::sync::Arc::clone(&pipeline);
    let (sent, received) = std::sync::mpsc::channel();
    let waiter = std::thread::spawn(move || {
        sent.send(other.pop_wait_av()).unwrap();
        other.pop_wait_av()
    });
    pipeline.publish_capture_lower_bound(500).unwrap();
    assert!(matches!(
        received
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap(),
        AvPipelineDrain::CaptureLowerBound {
            presentation_at_ns: 400
        }
    ));
    pipeline.abort().unwrap();
    assert!(matches!(
        waiter.join().unwrap(),
        Err(PipelineError::Aborted)
    ));
    assert_eq!(pipeline.stats().unwrap().accepted_frames, 1);
}
