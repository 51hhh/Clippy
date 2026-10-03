use super::*;
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn longshot_input_cancel_waits_for_entered_mutation_then_rejects_late_call() {
    let permission = NativeInputPermission::new();
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let (revoked_tx, revoked_rx) = mpsc::sync_channel(0);
    let (settled_tx, settled_rx) = mpsc::channel();
    let worker_permission = permission.clone();
    let worker = std::thread::spawn(move || {
        worker_permission.execute(|| {
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            Ok(())
        })
    });
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let cancel_permission = permission.clone();
    let cancel = std::thread::spawn(move || {
        cancel_permission.revoke();
        revoked_tx.send(()).unwrap();
        let result = cancel_permission.wait_idle();
        settled_tx.send(()).unwrap();
        result
    });
    revoked_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let active_after_revoke = permission.check().is_ok();
    let settled_before_release = settled_rx.try_recv().is_ok();
    release_tx.send(()).unwrap();
    worker.join().unwrap().unwrap();
    cancel.join().unwrap().unwrap();
    let late_called = std::cell::Cell::new(false);
    let late = permission.execute(|| {
        late_called.set(true);
        Ok(())
    });
    assert!(!active_after_revoke, "撤销应立即阻止尚未进入的输入");
    assert!(!settled_before_release, "已进入的调用必须先结算");
    assert_eq!(late.unwrap_err().code(), "longshot_session_superseded");
    assert!(!late_called.get());
}
