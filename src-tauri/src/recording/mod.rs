//! 录屏会话的持久化基础。
//!
//! 平台帧源与编码器尚未接入；这里先固定崩溃恢复合同，避免后续实现把未封尾视频当成一次性临时文件。

mod manifest;
// PX-REC-01 下一阶段的平台帧源会消费这些生产类型；当前先用合成帧锁定跨平台合同。
#[allow(dead_code)]
mod frame;
#[allow(dead_code)]
mod pipeline;
#[allow(dead_code)]
mod timeline;
// 诊断编码器先验证分段、时间线与恢复；平台默认编码器选型完成前不进入产品入口。
#[allow(dead_code)]
mod mux;

use std::fmt;
use std::io;
use std::path::Path;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RecoverySummary {
    scanned_sessions: usize,
    interrupted_sessions: usize,
    recoverable_segments: usize,
    rejected_sessions: usize,
    session_limit_reached: bool,
}

impl RecoverySummary {
    pub(crate) fn has_activity(self) -> bool {
        self.interrupted_sessions > 0 || self.rejected_sessions > 0 || self.session_limit_reached
    }
}

impl fmt::Display for RecoverySummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "扫描 {} 个会话，中断 {} 个，可恢复 {} 个分段，拒绝 {} 个{}",
            self.scanned_sessions,
            self.interrupted_sessions,
            self.recoverable_segments,
            self.rejected_sessions,
            if self.session_limit_reached {
                "，已达到扫描上限"
            } else {
                ""
            }
        )
    }
}

pub(crate) fn recover_interrupted_sessions(app_data_dir: &Path) -> io::Result<RecoverySummary> {
    manifest::recover_interrupted_sessions(app_data_dir)
}
