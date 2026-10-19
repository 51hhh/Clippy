//! 结果库恢复合并的进程级单槽与删除的会话所有权。

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
pub(crate) struct RecordingMergeRegistry {
    operations: Mutex<RecordingLibraryOperations>,
}

#[derive(Debug, Default)]
struct RecordingLibraryOperations {
    merging_session: Option<String>,
    deleting_sessions: HashSet<String>,
}

pub(in crate::recording) struct RecordingMergeGuard {
    registry: Arc<RecordingMergeRegistry>,
    session_id: String,
}

pub(in crate::recording) struct RecordingDeleteGuard {
    registry: Arc<RecordingMergeRegistry>,
    session_id: String,
}

impl RecordingMergeRegistry {
    pub(in crate::recording) fn begin(
        self: &Arc<Self>,
        session_id: &str,
    ) -> Result<RecordingMergeGuard, String> {
        let mut active = self
            .operations
            .lock()
            .map_err(|error| format!("录屏恢复合并状态损坏: {error}"))?;
        if active.merging_session.is_some() {
            return Err("已有录屏正在恢复合并".to_string());
        }
        if active.deleting_sessions.contains(session_id) {
            return Err("此录屏正在删除".to_string());
        }
        active.merging_session = Some(session_id.to_string());
        Ok(RecordingMergeGuard {
            registry: Arc::clone(self),
            session_id: session_id.to_string(),
        })
    }

    pub(in crate::recording) fn begin_delete(
        self: &Arc<Self>,
        session_id: &str,
    ) -> Result<RecordingDeleteGuard, String> {
        let mut active = self
            .operations
            .lock()
            .map_err(|error| format!("录屏删除状态损坏: {error}"))?;
        // 合并和删除在同一把锁内认领，不能用先查询、再执行留下竞态窗口。
        if active.merging_session.as_deref() == Some(session_id)
            || active.deleting_sessions.contains(session_id)
        {
            return Err("此录屏正在恢复合并或删除".to_string());
        }
        active.deleting_sessions.insert(session_id.to_string());
        Ok(RecordingDeleteGuard {
            registry: Arc::clone(self),
            session_id: session_id.to_string(),
        })
    }
}

impl Drop for RecordingMergeGuard {
    fn drop(&mut self) {
        match self.registry.operations.lock() {
            Ok(mut active)
                if active.merging_session.as_deref() == Some(self.session_id.as_str()) =>
            {
                active.merging_session = None;
            }
            Ok(_) => log::warn!("录屏恢复合并 guard 与当前会话不一致"),
            Err(error) => log::warn!("释放录屏恢复合并状态失败: {error}"),
        }
    }
}

impl Drop for RecordingDeleteGuard {
    fn drop(&mut self) {
        match self.registry.operations.lock() {
            Ok(mut active) => {
                if !active.deleting_sessions.remove(&self.session_id) {
                    log::warn!("录屏删除 guard 与当前会话不一致");
                }
            }
            Err(error) => log::warn!("释放录屏删除状态失败: {error}"),
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
