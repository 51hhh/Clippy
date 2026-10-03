//! 原生输入调用边界；取消与实际 mutation 共用同一会话许可。

use super::CaptureError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub(super) struct NativeInputPermission {
    active: Arc<AtomicBool>,
    calls: Arc<Mutex<()>>,
}

impl NativeInputPermission {
    pub(super) fn new() -> Self {
        Self {
            active: Arc::new(AtomicBool::new(true)),
            calls: Arc::new(Mutex::new(())),
        }
    }

    pub(super) fn check(&self) -> Result<(), CaptureError> {
        if self.active.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(CaptureError::LongshotSessionSuperseded)
        }
    }

    pub(super) fn execute<T>(
        &self,
        operation: impl FnOnce() -> Result<T, CaptureError>,
    ) -> Result<T, CaptureError> {
        // 只串行实际输入调用，不把 settle、抓帧或图像处理放入临界区。
        let _calls = self.calls.lock().map_err(CaptureError::state_lock)?;
        self.check()?;
        operation()
    }

    pub(super) fn revoke(&self) {
        // 先撤销，以便 manager 立即废弃 lease；已进入的 OS 调用另行结算。
        self.active.store(false, Ordering::Release);
    }

    pub(super) fn wait_idle(&self) -> Result<(), CaptureError> {
        let _calls = self.calls.lock().map_err(CaptureError::state_lock)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "input_permission/tests.rs"]
mod tests;
