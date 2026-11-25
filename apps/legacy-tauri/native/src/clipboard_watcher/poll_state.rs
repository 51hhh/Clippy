//! 观察与持久化分离：只有成功入库或用户明确抑制，才结束当前内容的处理。
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct PollState {
    observed: Option<String>,
    settled: bool,
    failures: u32,
    retry_at: Option<Instant>,
}

impl PollState {
    pub fn observe(&mut self, identity: &str, suppressed: bool, now: Instant) -> bool {
        if self.observed.as_deref() != Some(identity) {
            self.observed = Some(identity.to_owned());
            self.settled = false;
            self.failures = 0;
            self.retry_at = None;
        }
        if suppressed {
            self.settle();
        }
        !self.settled && self.retry_at.is_none_or(|deadline| now >= deadline)
    }

    pub fn settle(&mut self) {
        self.settled = true;
        self.failures = 0;
        self.retry_at = None;
    }

    pub fn failed(&mut self, now: Instant) {
        // 500ms、1s、2s、4s、8s；封顶后仍重试，不能把临时错误变成永久漏收。
        let delay = Duration::from_millis(500 << self.failures.min(4));
        self.failures = self.failures.saturating_add(1);
        self.retry_at = Some(now + delay);
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn needs_retry(&self, now: Instant) -> bool {
        self.observed.is_some()
            && !self.settled
            && self.retry_at.is_none_or(|deadline| now >= deadline)
    }

    #[cfg(target_os = "linux")]
    pub fn has_pending(&self) -> bool {
        self.observed.is_some() && !self.settled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{models::ContentType, storage::StorageEngine};

    #[test]
    fn suppressed_content_stays_out_of_history_until_observed_content_changes() {
        for identity in ["text:ocr", "html:translation", "image:codec"] {
            let mut state = PollState::default();
            let now = Instant::now();
            assert!(state.observe("before", false, now));
            state.settle();
            assert!(!state.observe(identity, true, now));
            assert!(!state.observe(identity, false, now + Duration::from_secs(1)));
            assert!(!state.observe(identity, false, now + Duration::from_secs(2)));
            assert!(state.observe("external", false, now));
            state.settle();
            assert!(state.observe(identity, false, now));
        }
    }

    #[test]
    fn transient_storage_failure_retries_once_then_settles_for_every_source() {
        for identity in ["text", "html", "image", "tmux"] {
            let mut state = PollState::default();
            let now = Instant::now();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("retry.db");
            let db = StorageEngine::new(&path).unwrap();
            let faults = rusqlite::Connection::open(&path).unwrap();
            faults.execute_batch("CREATE TRIGGER fail_insert BEFORE INSERT ON clips BEGIN SELECT RAISE(ABORT, 'temporary failure'); END;").unwrap();
            let mut attempts = 0;
            for elapsed in [0, 100, 500, 1000, 2000] {
                let time = now + Duration::from_millis(elapsed);
                if state.observe(identity, false, time) {
                    attempts += 1;
                    // 实际 SQLite 事务失败后恢复；不触碰用户 DB。
                    if attempts == 1 {
                        let failed = db.insert_clip(
                            &ContentType::Text,
                            Some("first"),
                            None,
                            None,
                            identity,
                            5,
                            false,
                        );
                        assert!(failed.is_err());
                        faults.execute_batch("DROP TRIGGER fail_insert;").unwrap();
                        state.failed(time);
                    } else {
                        db.insert_clip(
                            &ContentType::Text,
                            Some(identity),
                            None,
                            None,
                            identity,
                            4,
                            false,
                        )
                        .unwrap();
                        state.settle();
                    }
                }
            }
            assert_eq!(attempts, 2);
            assert_eq!(db.get_clips(None, false, 0, 10).unwrap().len(), 1);
        }
    }

    #[test]
    fn backoff_is_bounded_and_new_or_recovered_images_do_not_inherit_it() {
        let mut state = PollState::default();
        let mut now = Instant::now();
        for index in 0..12 {
            assert!(state.observe("bad", false, now));
            state.failed(now);
            let delay = Duration::from_millis(500 << index.min(4));
            assert!(!state.observe("bad", false, now + delay - Duration::from_millis(1)));
            now += delay;
        }
        assert!(state.observe("good", false, now));
        state.settle();
        state.reset();
        assert!(state.observe("good", false, now));
    }
}
