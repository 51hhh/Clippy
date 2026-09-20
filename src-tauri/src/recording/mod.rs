//! 录屏会话的持久化基础。
//!
//! X11 帧源、诊断编码器、VP9 原型与控制宿主已经分层接入。可信开始入口只在显式 VP9 构建的
//! 原生 X11 会话开放；默认构建与未完成原生验收的平台继续保持关闭。

mod manifest;
// 独立 Recording 覆盖层通过这里完成可信选区交接；跨平台门控仍让部分构建不引用全部类型。
#[allow(dead_code)]
mod selection;
// PX-REC-01 下一阶段的平台帧源会消费这些生产类型；当前先用合成帧锁定跨平台合同。
#[allow(dead_code)]
mod frame;
#[allow(dead_code)]
mod pipeline;
#[allow(dead_code)]
mod timeline;
// 持续采集 worker 已接入受门控产品会话；合成帧继续固定停止、节流与错误传播。
#[allow(dead_code)]
mod worker;
// 诊断编码消费线程先闭合 capture → pipeline → mux；产品会话接入前保持领域内可见。
#[allow(dead_code)]
mod encoder_worker;
// 编码线程按呈现时间周期封尾，已提交前缀不依赖正常 Stop 才进入恢复清单。
#[allow(dead_code)]
mod segmenting;
// 会话 owner 闭合 journal、采集与编码；默认关闭的构建仍不会引用全部生产类型。
#[allow(dead_code)]
mod session;
// 单活动录屏注册表固定代次与迟到命令语义。
#[allow(dead_code)]
mod manager;
// 截图选区、桌面恢复、控制面与会话注册表必须由一个生命周期按顺序交接。
#[allow(dead_code)]
mod lifecycle;
// 控制窗只从后端 registry 取得 exact generation token；前端不提交可伪造 token。
#[allow(dead_code)]
mod control_registry;
// Tauri 宿主接独立 Recording 覆盖层入口与控制面；产品策略仍按平台门槛关闭入口。
pub(crate) mod control_host;
// 完整输出与中断分段通过独立结果库导出；WebView 不接触应用数据路径。
pub(crate) mod library;
// 控制窗是否可见必须先通过平台排除能力与物理几何规划；窗口宿主接入前先固定纯函数合同。
#[allow(dead_code)]
mod control_window;
// 平台原生窗口排除与几何后备必须共享同一能力结论，避免旧系统误调用新 API。
mod control_exclusion;
// 诊断编码器先验证分段、时间线与恢复；平台默认编码器选型完成前不进入产品入口。
#[allow(dead_code)]
mod mux;
// 平台帧源先在各自系统内验证；尚未接入录屏产品会话。
#[allow(dead_code)]
mod platform;
// 工程基准直接串起真实 X11 帧源、三槽 pipeline、分段与最终 VP9。
#[cfg(all(target_os = "linux", feature = "recording-vp9-prototype"))]
mod benchmark;

#[cfg(all(target_os = "linux", feature = "recording-vp9-prototype"))]
pub use benchmark::X11Vp9BenchmarkReport;
#[cfg(all(target_os = "linux", feature = "recording-vp9-prototype"))]
pub(crate) use benchmark::{run_x11_vp9_benchmark, X11Vp9BenchmarkOptions};
pub(crate) use control_host::handle_control_destroyed;
pub(crate) use control_registry::RecordingControlRegistry;
pub(crate) use lifecycle::RecordingLifecycle;
#[cfg(feature = "recording-vp9-prototype")]
pub(crate) use mux::vp9_webm::Vp9WebmWriter;

use std::fmt;
use std::io;
use std::path::Path;

/// 当前阶段只允许显式启用 VP9 原型的原生 X11 构建进入产品选区。
/// Windows、macOS 与 Wayland 的帧源骨架继续由 CI 编译，但在完成真机与控制面验收前不展示入口。
pub(crate) fn product_entry_available() -> bool {
    product_entry_available_for(crate::platform::current_session())
}

fn product_entry_available_for(session: crate::platform::DesktopSession) -> bool {
    cfg!(all(
        feature = "recording-vp9-prototype",
        target_os = "linux"
    )) && session == crate::platform::DesktopSession::X11
}

#[cfg(test)]
mod product_entry_tests {
    use super::product_entry_available_for;
    use crate::platform::DesktopSession;

    #[test]
    fn product_entry_policy_rejects_unapproved_sessions() {
        for session in [
            DesktopSession::Wayland,
            DesktopSession::Native,
            DesktopSession::Unknown,
        ] {
            assert!(!product_entry_available_for(session));
        }
    }

    #[test]
    fn x11_requires_the_explicit_vp9_linux_build() {
        assert_eq!(
            product_entry_available_for(DesktopSession::X11),
            cfg!(all(
                feature = "recording-vp9-prototype",
                target_os = "linux"
            ))
        );
    }
}

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
