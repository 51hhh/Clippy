use super::*;
use std::sync::mpsc;

fn native_frame(timestamp: u64) -> StampedFrame {
    StampedFrame {
        captured_at_ns: timestamp,
        frame: Frame::new(2, 2, vec![0; 16]),
    }
}

#[test]
fn pending_native_frame_constrains_idle_lower_bound() {
    let bridge = FrameBridge::default();
    bridge.replace(native_frame(100));
    bridge.publish_idle_lower_bound(500);
    assert_eq!(bridge.capture_lower_bound_ns(0).unwrap(), Some(100));
    assert_eq!(
        bridge
            .take_after(0, Duration::ZERO)
            .unwrap()
            .unwrap()
            .captured_at_ns,
        100
    );
    assert_eq!(bridge.capture_lower_bound_ns(0).unwrap(), Some(500));
    bridge.publish_idle_lower_bound(200);
    assert_eq!(bridge.capture_lower_bound_ns(0).unwrap(), Some(500));
}

#[test]
fn resumed_minimum_excludes_stale_cached_frame_without_advancing_real_frame() {
    let bridge = FrameBridge::default();
    bridge.replace(native_frame(100));
    bridge.publish_idle_lower_bound(200);
    assert_eq!(bridge.capture_lower_bound_ns(300).unwrap(), Some(300));
    assert!(bridge.take_after(300, Duration::ZERO).unwrap().is_none());
    bridge.replace(native_frame(350));
    bridge.publish_idle_lower_bound(500);
    assert_eq!(bridge.capture_lower_bound_ns(300).unwrap(), Some(350));
}

#[test]
fn actual_bridge_idle_stamp_precedes_next_frame_and_drop_joins_with_sender_alive() {
    let (sender, frames) = mpsc::sync_channel(0);
    let bridge = Arc::new(FrameBridge::default());
    let handle = bridge_lifecycle::FrameBridgeThread::spawn(
        frames,
        Arc::clone(&bridge),
        RecordingSessionClock::new(),
    )
    .unwrap();
    sender.send(Frame::new(2, 2, vec![1; 16])).unwrap();
    let first = bridge
        .take_after(0, Duration::from_secs(5))
        .unwrap()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let boundary = loop {
        let boundary = bridge.capture_lower_bound_ns(0).unwrap().unwrap();
        if boundary > first.captured_at_ns {
            break boundary;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    };
    sender.send(Frame::new(2, 2, vec![2; 16])).unwrap();
    let next = bridge
        .take_after(0, Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert!(next.captured_at_ns >= boundary);
    assert_eq!(next.frame.raw, vec![2; 16]);
    drop(handle);
    assert!(matches!(
        bridge.capture_lower_bound_ns(0),
        Err(WindowsFrameSourceError::StreamClosed)
    ));
    assert!(sender.send(Frame::new(2, 2, vec![0; 16])).is_err());
}

#[test]
fn poisoned_bridge_returns_error_instead_of_false_progress() {
    let bridge = Arc::new(FrameBridge::default());
    let other = Arc::clone(&bridge);
    assert!(std::thread::spawn(move || {
        let _guard = other.latest.lock().unwrap();
        panic!("注入桥锁故障");
    })
    .join()
    .is_err());
    assert!(matches!(
        bridge.capture_lower_bound_ns(0),
        Err(WindowsFrameSourceError::BridgePoisoned)
    ));
}
