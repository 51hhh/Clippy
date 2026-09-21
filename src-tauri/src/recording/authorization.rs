//! 录屏系统授权等待阶段的进程内取消令牌。
//!
//! 令牌只由 Rust 控制窗 registry 保存并传给平台帧源；WebView 只能用调用方窗口标签请求取消，不能
//! 构造或复用令牌。原子位负责迟到观察，Notify 负责及时唤醒 Portal future。

use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

#[derive(Debug, Default)]
pub(super) struct RecordingAuthorizationCancellation {
    cancelled: AtomicBool,
    changed: Notify,
}

impl RecordingAuthorizationCancellation {
    /// 返回本次调用是否首次发布取消。
    pub(super) fn cancel(&self) -> bool {
        let first = !self.cancelled.swap(true, Ordering::AcqRel);
        if first {
            // 只有一个 Portal waiter；notify_one 会在 waiter 尚未首次 poll 时保留 permit，避免丢唤醒。
            self.changed.notify_one();
        }
        first
    }

    #[cfg(any(test, target_os = "linux"))]
    pub(super) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    #[cfg(any(test, target_os = "linux"))]
    pub(super) async fn cancelled(&self) {
        loop {
            let changed = self.changed.notified();
            if self.is_cancelled() {
                return;
            }
            changed.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn cancellation_is_idempotent_and_late_waiters_observe_it() {
        let cancellation = RecordingAuthorizationCancellation::default();
        assert!(cancellation.cancel());
        assert!(!cancellation.cancel());

        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(cancellation.cancelled());
    }

    #[test]
    fn cancellation_wakes_an_existing_waiter() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let cancellation = Arc::new(RecordingAuthorizationCancellation::default());
        let publisher = Arc::clone(&cancellation);
        runtime.block_on(async move {
            let waiter = cancellation.cancelled();
            let publisher = async move {
                tokio::task::yield_now().await;
                assert!(publisher.cancel());
            };
            tokio::join!(waiter, publisher);
        });
    }
}
