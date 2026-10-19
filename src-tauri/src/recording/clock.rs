//! 录屏会话唯一的单调时钟。
//!
//! 平台视频源与后续音频适配器只能复制这个时钟，不能各自创建时间原点。容器呈现时间线仍由
//! 对应轨道协调器映射；这里返回的是从会话创建开始的源时间戳。

use std::time::Instant;

#[derive(Debug, Clone)]
pub(super) struct RecordingSessionClock {
    origin: Instant,
}

impl RecordingSessionClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }

    pub fn now_ns(&self) -> u64 {
        duration_ns(self.origin.elapsed())
    }

    #[cfg(test)]
    fn timestamp_at(&self, sampled_at: Instant) -> u64 {
        duration_ns(sampled_at.saturating_duration_since(self.origin))
    }
}

fn duration_ns(duration: std::time::Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_map_one_sample_to_the_same_session_timestamp() {
        let clock = RecordingSessionClock::new();
        let clone = clock.clone();
        let sampled_at = Instant::now();

        assert_eq!(
            clock.timestamp_at(sampled_at),
            clone.timestamp_at(sampled_at)
        );
    }

    #[test]
    fn timestamp_saturates_instead_of_wrapping() {
        assert_eq!(
            duration_ns(std::time::Duration::from_nanos(u64::MAX)),
            u64::MAX
        );
        assert_eq!(
            duration_ns(std::time::Duration::from_secs(u64::MAX)),
            u64::MAX
        );
    }
}
