//! 双轨采集 factory 就绪后的释放等待，等待期间仍可由 owner Drop/Stop 取消。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

const START_WAIT_SLICE: Duration = Duration::from_millis(50);

pub(super) fn wait_for_start(start: Option<Receiver<()>>, stop_requested: &AtomicBool) -> bool {
    let Some(start) = start else {
        return true;
    };
    while !stop_requested.load(Ordering::Acquire) {
        match start.recv_timeout(START_WAIT_SLICE) {
            Ok(()) => return !stop_requested.load(Ordering::Acquire),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return false,
        }
    }
    false
}
