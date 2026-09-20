//! 录屏会话的持久化基础。
//!
//! 平台帧源与编码器尚未接入；这里先固定崩溃恢复合同，避免后续实现把未封尾视频当成一次性临时文件。

mod manifest;
// 先固定截图覆盖层到平台帧源之间的可信交接；产品会话接入后移除 dead_code 例外。
#[allow(dead_code)]
mod selection;
// PX-REC-01 下一阶段的平台帧源会消费这些生产类型；当前先用合成帧锁定跨平台合同。
#[allow(dead_code)]
mod frame;
#[allow(dead_code)]
mod pipeline;
#[allow(dead_code)]
mod timeline;
// 持续采集 worker 尚未接入产品会话；当前用合成帧固定停止、节流与错误传播。
#[allow(dead_code)]
mod worker;
// 诊断编码消费线程先闭合 capture → pipeline → mux；产品会话接入前保持领域内可见。
#[allow(dead_code)]
mod encoder_worker;
// 会话 owner 已闭合 journal、采集与诊断编码；产品注册表和 IPC 接入前保持领域内可见。
#[allow(dead_code)]
mod session;
// 单活动录屏注册表已固定代次与迟到命令语义；产品 IPC 接入前保持领域内可见。
#[allow(dead_code)]
mod manager;
// 诊断编码器先验证分段、时间线与恢复；平台默认编码器选型完成前不进入产品入口。
#[allow(dead_code)]
mod mux;
// 平台帧源先在各自系统内验证；尚未接入录屏产品会话。
#[allow(dead_code)]
mod platform;

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
