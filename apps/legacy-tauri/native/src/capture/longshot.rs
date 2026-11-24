//! 旧界面保留桌面采集与控制窗，拼接会话由共享层维护。

use super::CaptureError;
use crate::pin::PinOrigin;

mod auto_scroll;
#[cfg(all(target_os = "linux", feature = "longshot-wayland-auto"))]
mod auto_scroll_wayland;
mod controller;
mod frame_adapter;
mod lifecycle;
mod manager;
mod recapture;
use clippy_capture::longshot::session;
pub(crate) mod window_host;

// lifecycle 与控制窗 registry 通过 AppState 保持唯一实例；重导出固定 capture 领域边界。
pub(crate) use auto_scroll::LongshotAutoDirection;
pub(in crate::capture) use auto_scroll::LongshotAutoTarget;
pub(super) use frame_adapter::LongshotFrameAdapter;
pub(crate) use lifecycle::LongshotLifecycle;
pub(super) use manager::{LongshotManager, LongshotSessionToken, LongshotStart};
pub(super) use recapture::capture_monitor_frame;
pub(super) use clippy_capture::longshot::{LongshotAppendOutcome, LongshotSession, LongshotSnapshot};
pub(crate) use window_host::{
    handle_controller_destroyed, LongshotActivation, LongshotAutoCapability,
    LongshotControllerHandle, LongshotControllerLaunch, LongshotControllerRegistry,
    LongshotIpcError, LongshotOutputAction, LongshotOutputResult, LongshotSnapshotDto,
};

/// 长截图完成后的可信像素及其桌面全局逻辑来源矩形。
///
/// 来源矩形只由冻结首帧和实际物理裁剪区反算，不能由控制窗或前端提交。
#[derive(Debug, Clone, PartialEq)]
pub(in crate::capture) struct LongshotArtifact {
    pub(in crate::capture) png: Vec<u8>,
    pub(in crate::capture) origin: PinOrigin,
}
