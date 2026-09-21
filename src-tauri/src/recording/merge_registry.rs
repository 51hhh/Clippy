//! 结果库恢复合并的进程级单槽所有权。

use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
pub(crate) struct RecordingMergeRegistry {
    active_session: Mutex<Option<String>>,
}

pub(in crate::recording) struct RecordingMergeGuard {
    registry: Arc<RecordingMergeRegistry>,
    session_id: String,
}

impl RecordingMergeRegistry {
    pub(in crate::recording) fn begin(
        self: &Arc<Self>,
        session_id: &str,
    ) -> Result<RecordingMergeGuard, String> {
        let mut active = self
            .active_session
            .lock()
            .map_err(|error| format!("录屏恢复合并状态损坏: {error}"))?;
        if active.is_some() {
            return Err("已有录屏正在恢复合并".to_string());
        }
        *active = Some(session_id.to_string());
        Ok(RecordingMergeGuard {
            registry: Arc::clone(self),
            session_id: session_id.to_string(),
        })
    }
}

impl Drop for RecordingMergeGuard {
    fn drop(&mut self) {
        match self.registry.active_session.lock() {
            Ok(mut active) if active.as_deref() == Some(self.session_id.as_str()) => *active = None,
            Ok(_) => log::warn!("录屏恢复合并 guard 与当前会话不一致"),
            Err(error) => log::warn!("释放录屏恢复合并状态失败: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_merge_runs_at_a_time_and_drop_releases_the_slot() {
        let registry = Arc::new(RecordingMergeRegistry::default());
        let guard = registry.begin("one").unwrap();
        assert!(registry.begin("one").is_err());
        assert!(registry.begin("two").is_err());
        drop(guard);
        assert!(registry.begin("two").is_ok());
    }
}
