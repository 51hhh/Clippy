use super::*;

#[test]
fn lower_bound_neither_establishes_epoch_nor_advances_native_identity() {
    let mut timeline = RecordingTimeline::default();
    assert_eq!(timeline.map_capture_lower_bound(1_000).unwrap(), None);
    assert_eq!(
        timeline.finish(1_100),
        Err(TimelineError::FinishBeforeFirstFrame)
    );
    assert_eq!(timeline.map_frame(100).unwrap(), Some(0));
    assert_eq!(timeline.map_capture_lower_bound(500).unwrap(), Some(400));
    assert_eq!(timeline.last_source_ns, Some(100));
    assert_eq!(timeline.last_presentation_ns, Some(0));
    assert_eq!(timeline.map_frame(500).unwrap(), Some(400));
    assert_eq!(
        timeline.map_capture_lower_bound(499),
        Err(TimelineError::SourceTimestampNotIncreasing)
    );
}

#[test]
fn paused_lower_bound_is_ignored_and_resumed_bound_excludes_pause() {
    let mut timeline = RecordingTimeline::default();
    timeline.map_frame(100).unwrap();
    timeline.pause(200).unwrap();
    assert!(timeline.is_paused());
    assert_eq!(timeline.map_capture_lower_bound(10_000).unwrap(), None);
    assert_eq!(timeline.map_frame(0).unwrap(), None);
    timeline.resume(1_200).unwrap();
    assert!(!timeline.is_paused());
    assert_eq!(timeline.map_capture_lower_bound(1_300).unwrap(), Some(200));
    assert_eq!(timeline.finish(1_300).unwrap(), 200);
}
