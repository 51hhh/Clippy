use super::*;
use crate::recording::platform::windows::WindowsFrameSourceError;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(5);
const OBSERVE: Duration = Duration::from_millis(500);

struct ExitSignal(Sender<()>);
impl Drop for ExitSignal {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

fn closed(bridge: &FrameBridge) -> bool {
    matches!(
        bridge.take_after(0, Duration::ZERO),
        Err(WindowsFrameSourceError::StreamClosed)
    )
}

fn await_closed(bridge: &FrameBridge) {
    assert!(matches!(
        bridge.take_after(0, DEADLINE),
        Err(WindowsFrameSourceError::StreamClosed)
    ));
}

#[test]
fn windows_wgc_bridge_start_failure_waits_for_thread_cleanup() {
    let (ready, started) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let (exit, exited) = mpsc::channel();
    let thread = thread::spawn(move || {
        let _exit = ExitSignal(exit);
        ready.send(()).unwrap();
        gate.recv_timeout(DEADLINE).unwrap();
    });
    started.recv_timeout(DEADLINE).unwrap();
    let bridge = FrameBridgeThread::new(Arc::new(AtomicBool::new(false)), thread);
    let (result_tx, result_rx) = mpsc::channel();
    let owner = thread::spawn(move || {
        let result = start_recorder((), bridge, |_| Err::<(), _>("original start failure"));
        result_tx.send(result.err().unwrap()).unwrap();
    });
    let early = result_rx.recv_timeout(OBSERVE);
    let returned_early = early.is_ok();
    // 旧 owner 提前返回时也先释放并等待 fixture 退出，再报告失败。
    release.send(()).unwrap();
    exited.recv_timeout(DEADLINE).unwrap();
    let error = early.unwrap_or_else(|_| result_rx.recv_timeout(DEADLINE).unwrap());
    owner.join().unwrap();
    assert!(
        !returned_early,
        "start error returned while bridge remained gated"
    );
    assert_eq!(error, "original start failure");
}

#[test]
fn windows_wgc_bridge_start_failure_cancels_with_sender_still_retained() {
    let (sender, frames) = mpsc::sync_channel(0);
    let retained_sender = sender.clone();
    let bridge = Arc::new(FrameBridge::default());
    let worker =
        FrameBridgeThread::spawn(frames, Arc::clone(&bridge), RecordingSessionClock::new())
            .unwrap();
    let (result_tx, result_rx) = mpsc::channel();
    let owner = thread::spawn(move || {
        let result = start_recorder(sender, worker, |_| Err::<(), _>("native start failed"));
        result_tx.send(result.err().unwrap()).unwrap();
    });
    let early = result_rx.recv_timeout(OBSERVE);
    let was_closed_at_return = early.is_ok() && closed(&bridge);
    drop(retained_sender);
    await_closed(&bridge);
    let error = early.unwrap_or_else(|_| result_rx.recv_timeout(DEADLINE).unwrap());
    owner.join().unwrap();
    assert!(
        was_closed_at_return,
        "rollback returned without closing bridge while another sender was alive"
    );
    assert_eq!(error, "native start failed");
}

#[test]
fn windows_wgc_bridge_normal_shutdown_does_not_depend_on_last_sender_drop() {
    let (sender, frames) = mpsc::sync_channel(0);
    let retained_sender = sender.clone();
    let bridge = Arc::new(FrameBridge::default());
    let worker =
        FrameBridgeThread::spawn(frames, Arc::clone(&bridge), RecordingSessionClock::new())
            .unwrap();
    let (done, finished) = mpsc::channel();
    let owner = thread::spawn(move || {
        shutdown_bridge(Some(sender), worker, |_| {});
        done.send(()).unwrap();
    });
    let early = finished.recv_timeout(OBSERVE);
    let completed_before_sender_release = early.is_ok();
    let was_closed = closed(&bridge);
    drop(retained_sender);
    if matches!(early, Err(RecvTimeoutError::Timeout)) {
        finished.recv_timeout(DEADLINE).unwrap();
    }
    await_closed(&bridge);
    owner.join().unwrap();
    assert!(
        completed_before_sender_release,
        "shutdown was waiting for a sender outside the owner"
    );
    assert!(was_closed, "shutdown returned before frame bridge closed");
}

#[test]
fn windows_wgc_bridge_normal_shutdown_cancels_before_native_stop_callback() {
    let (sender, frames) = mpsc::sync_channel(0);
    let bridge = Arc::new(FrameBridge::default());
    let worker =
        FrameBridgeThread::spawn(frames, Arc::clone(&bridge), RecordingSessionClock::new())
            .unwrap();
    let cancelled = Arc::clone(&worker.cancelled);
    let mut cancelled_before_stop = false;
    shutdown_bridge(Some(sender), worker, |_| {
        cancelled_before_stop = cancelled.load(Ordering::Acquire);
    });
    assert!(
        cancelled_before_stop,
        "native stop ran before bridge cancellation"
    );
    assert!(closed(&bridge));
}

#[test]
fn windows_wgc_bridge_original_start_error_survives_bridge_panic() {
    let (ready, started) = mpsc::channel();
    let thread = thread::spawn(move || {
        ready.send(()).unwrap();
        panic!("injected bridge panic");
    });
    started.recv_timeout(DEADLINE).unwrap();
    let bridge = FrameBridgeThread::new(Arc::new(AtomicBool::new(false)), thread);
    let result = start_recorder((), bridge, |_| {
        Err::<(), _>(WindowsFrameSourceError::Initialize(
            "original native error".into(),
        ))
    });
    assert!(
        matches!(result, Err(WindowsFrameSourceError::Initialize(message)) if message == "original native error")
    );
}

#[test]
fn windows_wgc_bridge_success_transfers_live_worker_and_preserves_frames_and_clock() {
    let (sender, frames) = mpsc::sync_channel(0);
    let bridge = Arc::new(FrameBridge::default());
    let worker =
        FrameBridgeThread::spawn(frames, Arc::clone(&bridge), RecordingSessionClock::new())
            .unwrap();
    let (sender, worker) =
        start_recorder(sender, worker, |_| Ok::<(), WindowsFrameSourceError>(()))
            .ok()
            .unwrap();
    assert!(!closed(&bridge));
    let mut last_timestamp = None;
    for value in [10, 20, 30] {
        sender.send(Frame::new(1, 1, vec![value; 4])).unwrap();
        let received = bridge.take_after(0, DEADLINE).unwrap().unwrap();
        assert_eq!(received.frame.raw, vec![value; 4]);
        assert_eq!((received.frame.width, received.frame.height), (1, 1));
        if let Some(previous) = last_timestamp {
            assert!(received.captured_at_ns > previous);
        }
        last_timestamp = Some(received.captured_at_ns);
    }
    shutdown_bridge(Some(sender), worker, |_| {});
    assert!(closed(&bridge));
}

#[test]
fn windows_wgc_bridge_closed_input_joins_without_a_recorder() {
    let (sender, frames) = mpsc::sync_channel(0);
    let bridge = Arc::new(FrameBridge::default());
    let worker =
        FrameBridgeThread::spawn(frames, Arc::clone(&bridge), RecordingSessionClock::new())
            .unwrap();
    drop(sender);
    await_closed(&bridge);
    shutdown_bridge::<()>(None, worker, |_| unreachable!());
    assert!(closed(&bridge));
}
