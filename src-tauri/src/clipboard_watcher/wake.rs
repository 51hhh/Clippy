//! 让 watcher“立刻轮询一次”的唤醒口。
//!
//! **为什么需要它。** 截图点对钩只把 PNG 写进系统剪贴板（`capture/mod.rs` 的 `Copy` 分支），
//! 入库是 watcher 下一次轮询才做的事。轮询周期 500 ms，于是"复制完立刻去 Pin"这条常见
//! 操作里，数据库最新的一条还是**上一张图**，前端列表缓存（`clipboard-list.js` 的
//! `_allClips[0]`）同样还是上一条，而 `clip-added` 要等入库之后才发——用户看到的就是
//! "pin 出来的是之前那张图"。
//!
//! **为什么不让写入方自己 `insert_clip`。** watcher 的哈希算的是**它自己**把剪贴板 RGBA
//! 重新编出来的那张 PNG，与我们手里这串字节几乎不可能一致（`write_clip_to_clipboard`
//! 的图片分支里已经写着同一条理由）。写入方自己插一条，500 ms 后 watcher 照样会因为
//! 哈希不同再插一条。要让两边哈希一致就得用 watcher 的编码器把整图再编一遍，
//! 那正是刚从提交热路径上省掉的开销。所以入库仍然只有 watcher 一条路径——
//! 哈希基准、去重、`clip-added` 全都不变，这里只把它的下一次轮询从"最多 500 ms 之后"
//! 提前到"马上"。

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// 带待处理标记的定时等待。
pub struct PollSignal {
    pending: Mutex<bool>,
    ready: Condvar,
}

impl PollSignal {
    pub const fn new() -> Self {
        Self {
            pending: Mutex::new(false),
            ready: Condvar::new(),
        }
    }

    /// 记一次待处理的轮询请求并唤醒等待中的 watcher。
    ///
    /// 用布尔标记而不是裸 `notify_one()`：敲的时候 watcher 可能正忙（不在等待），
    /// 裸通知会直接丢掉；标记会留到它下一次进入等待时立刻兑现。
    pub fn signal(&self) {
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.ready.notify_all();
    }

    /// 最多等 `timeout`，期间被 `signal` 敲过就立刻返回并消耗掉标记。
    ///
    /// 返回是否因唤醒而提前返回（`false` 表示等满了超时）。按绝对截止时间循环，
    /// 这样条件变量的虚假唤醒不会让等待提前结束。
    pub fn wait(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        while !*pending {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (guard, _) = self
                .ready
                .wait_timeout(pending, remaining)
                .unwrap_or_else(|e| e.into_inner());
            pending = guard;
        }
        *pending = false;
        true
    }
}

impl Default for PollSignal {
    fn default() -> Self {
        Self::new()
    }
}

/// 进程内唯一的那个信号：watcher 等它，剪贴板写入方敲它。
///
/// 做成全局而不是挂在 `ClipboardWatcher` 上，是为了让"程序化写入之后历史立刻反映它"
/// 成为 `writer.rs` 的固有行为——写入口只有那三个函数，在那里敲一次就覆盖全部调用方，
/// 不必指望每个新调用点都记得自己补一句。
static SHARED: PollSignal = PollSignal::new();

/// 剪贴板写入成功后敲一下，让 watcher 别再等满一个轮询周期。
pub fn nudge() {
    SHARED.signal();
}

/// watcher 的等待入口，替代裸 `thread::sleep`。
pub fn wait_for_next_poll(interval: Duration) -> bool {
    SHARED.wait(interval)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// 已经敲过的标记不能丢：唤醒发生在 watcher 正忙的时候是常态
    /// （它刚好在编码上一张图），裸 `notify_one` 那一版会把这次写入的即时入库丢掉。
    #[test]
    fn a_signal_raised_while_nobody_waits_is_still_delivered() {
        let signal = PollSignal::new();
        signal.signal();
        let at = Instant::now();
        assert!(signal.wait(Duration::from_secs(30)));
        assert!(
            at.elapsed() < Duration::from_secs(1),
            "应当立刻返回而不是等满超时"
        );
    }

    /// 标记只兑现一次，否则 watcher 会退化成忙轮询。
    #[test]
    fn the_pending_flag_is_consumed_by_one_wait() {
        let signal = PollSignal::new();
        signal.signal();
        assert!(signal.wait(Duration::from_secs(30)));
        let at = Instant::now();
        assert!(!signal.wait(Duration::from_millis(120)));
        assert!(
            at.elapsed() >= Duration::from_millis(100),
            "第二次必须重新等满"
        );
    }

    /// 没人敲就老老实实等满周期——这是 watcher 的常态，不能忙转。
    #[test]
    fn without_a_signal_the_wait_runs_out_the_interval() {
        let signal = PollSignal::new();
        let at = Instant::now();
        assert!(!signal.wait(Duration::from_millis(150)));
        assert!(at.elapsed() >= Duration::from_millis(140));
    }

    /// 正在等待的线程要被当场唤醒，而不是等到超时。
    #[test]
    fn a_waiting_thread_wakes_up_immediately() {
        let signal = Arc::new(PollSignal::new());
        let waiter = Arc::clone(&signal);
        let handle = std::thread::spawn(move || {
            let at = Instant::now();
            let woken = waiter.wait(Duration::from_secs(30));
            (woken, at.elapsed())
        });
        std::thread::sleep(Duration::from_millis(50));
        signal.signal();
        let (woken, elapsed) = handle.join().unwrap();
        assert!(woken);
        assert!(elapsed < Duration::from_secs(5), "实际等了 {elapsed:?}");
    }
}
