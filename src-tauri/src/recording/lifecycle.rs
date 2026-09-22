//! 普通截图到持续录屏的唯一桌面资源生命周期。
//!
//! 平台采集计划先在 Ordinary 会话仍完整时核验；成功后才消费截图会话并恢复桌面。控制面准备完成后，
//! 原生帧源才在采集线程内创建。停止、取消和启动失败都会先回收会话与控制面，最后显式释放
//! Recording gate。

#[cfg(feature = "recording-opus-webm")]
use super::av_session::AvRecordingConfig;
use super::clock::RecordingSessionClock;
use super::manager::{
    RecordingManager, RecordingManagerError, RecordingSessionReport, RecordingToken,
};
use super::platform::{
    PlatformAudioSourceKind, PlatformAudioSourcePlan, PlatformFrameSource, PlatformFrameSourcePlan,
    RecordingControlTarget, RecordingSourceDescriptor,
};
use super::segmenting::{RecordingEncoder, DEFAULT_SEGMENT_DURATION_NS};
use super::selection::PreparedRecordingSelection;
use super::session::DiagnosticRecordingConfig;
use super::worker::RecordingFrameSource;
use crate::capture::{
    CaptureError, CaptureManager, CaptureModeOwnership, CaptureSelection, RecordingCaptureSpec,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RecordingAudioMode {
    None,
    SystemAudio,
    Microphone,
}

pub(super) fn available_recording_audio_modes() -> &'static [RecordingAudioMode] {
    #[cfg(all(target_os = "linux", feature = "recording-linux-av-qa"))]
    {
        &[
            RecordingAudioMode::None,
            RecordingAudioMode::SystemAudio,
            RecordingAudioMode::Microphone,
        ]
    }
    #[cfg(all(target_os = "windows", feature = "recording-windows-av-qa"))]
    {
        &[
            RecordingAudioMode::None,
            RecordingAudioMode::SystemAudio,
            RecordingAudioMode::Microphone,
        ]
    }
    #[cfg(all(target_os = "macos", feature = "recording-macos-av-qa"))]
    {
        macos_recording_audio_modes(
            super::macos_screencapturekit_audio_runtime_available(),
            super::macos_screencapturekit_microphone_runtime_available(),
        )
    }
    #[cfg(not(any(
        all(target_os = "linux", feature = "recording-linux-av-qa"),
        all(target_os = "windows", feature = "recording-windows-av-qa"),
        all(target_os = "macos", feature = "recording-macos-av-qa")
    )))]
    {
        &[RecordingAudioMode::None]
    }
}

#[cfg(any(test, all(target_os = "macos", feature = "recording-macos-av-qa")))]
fn macos_recording_audio_modes(
    system_audio_available: bool,
    microphone_available: bool,
) -> &'static [RecordingAudioMode] {
    if system_audio_available && microphone_available {
        &[
            RecordingAudioMode::None,
            RecordingAudioMode::SystemAudio,
            RecordingAudioMode::Microphone,
        ]
    } else if system_audio_available {
        &[RecordingAudioMode::None, RecordingAudioMode::SystemAudio]
    } else {
        &[RecordingAudioMode::None]
    }
}

fn recording_audio_mode_available(mode: RecordingAudioMode) -> bool {
    available_recording_audio_modes().contains(&mode)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordingStartRequest {
    pub session_id: String,
    pub frames_per_second: u32,
    pub include_cursor: bool,
    pub audio_mode: RecordingAudioMode,
    /// 只由后端产品策略选择，IPC 不得让前端提交任意编码器或诊断参数。
    pub encoder: RecordingEncoder,
}

pub(super) struct RecordingStartContext<'a> {
    pub capture: &'a CaptureManager,
    pub caller: &'a str,
    pub selection: &'a CaptureSelection,
    pub app_data_dir: &'a Path,
}

pub(super) struct PreparedRecordingSource<F> {
    pub source_factory: F,
    pub descriptor: RecordingSourceDescriptor,
    pub audio_plan: Option<PlatformAudioSourcePlan>,
}

#[derive(Debug, Error)]
pub(super) enum RecordingLifecycleError {
    #[error("已有录屏生命周期正在进行")]
    Busy,
    #[error("录屏生命周期不存在")]
    Missing,
    #[error("录屏生命周期已经更新")]
    Superseded,
    #[error("录屏生命周期锁已损坏")]
    Poisoned,
    #[error(transparent)]
    Capture(#[from] CaptureError),
    #[error("录屏帧源准备失败: {0}")]
    Source(String),
    #[error("当前构建不支持请求的录屏音频模式")]
    AudioUnavailable,
    #[error("录屏桌面动作 {operation} 失败: {message}")]
    Desktop {
        operation: &'static str,
        message: String,
    },
    #[error(transparent)]
    Manager(#[from] RecordingManagerError),
    #[error("录屏主流程失败后释放模式所有权也失败；主错误: {primary}；释放错误: {release}")]
    ReleaseAfterFailure {
        primary: String,
        release: CaptureError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopResources {
    overlays: Vec<String>,
    pins: Vec<String>,
    sources: Vec<String>,
}

struct PublishingSession {
    session_id: String,
    ownership: CaptureModeOwnership,
    desktop: DesktopResources,
}

struct ActiveSession {
    token: RecordingToken,
    ownership: CaptureModeOwnership,
}

enum LifecycleSlot {
    Empty,
    Starting,
    Publishing(PublishingSession),
    Active(ActiveSession),
    Terminating(RecordingToken),
    Releasing(String),
    TerminalFailed,
}

/// 桌面实现不得在内部回调 lifecycle；所有方法都在状态锁外执行。
pub(super) trait DesktopActions {
    fn close_overlays(&self, labels: &[String]) -> Result<(), String>;
    fn restore_pins(&self, labels: &[String]) -> Result<(), String>;
    fn restore_sources(&self, labels: &[String]) -> Result<(), String>;
    fn settle_after_restore(&self) -> Result<(), String>;
    fn prepare_control(
        &self,
        session_id: &str,
        descriptor: &RecordingSourceDescriptor,
    ) -> Result<RecordingControlTarget, String>;
    fn bind_control(&self, token: &RecordingToken) -> Result<(), String>;
    fn close_control(&self, session_id: &str) -> Result<(), String>;
}

pub(crate) struct RecordingLifecycle {
    manager: RecordingManager,
    slot: Mutex<LifecycleSlot>,
}

impl Default for RecordingLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingLifecycle {
    pub(crate) fn new() -> Self {
        Self {
            manager: RecordingManager::new(),
            slot: Mutex::new(LifecycleSlot::Empty),
        }
    }

    /// 生产平台适配器。仍由调用方决定何时开放入口；前端不能选择后端或提交物理来源。
    pub(super) fn start_platform<A: DesktopActions>(
        &self,
        context: RecordingStartContext<'_>,
        request: RecordingStartRequest,
        actions: &A,
    ) -> Result<RecordingToken, RecordingLifecycleError> {
        if !recording_audio_mode_available(request.audio_mode) {
            return Err(RecordingLifecycleError::AudioUnavailable);
        }
        let audio_mode = request.audio_mode;
        self.start::<PlatformFrameSource, _, _, _>(
            context,
            request,
            |selection| {
                let plan = PlatformFrameSourcePlan::prepare(selection)
                    .map_err(|error| error.to_string())?;
                let descriptor = plan.descriptor().clone();
                let audio_plan = match audio_mode {
                    RecordingAudioMode::None => None,
                    RecordingAudioMode::SystemAudio => Some(
                        plan.audio_plan(PlatformAudioSourceKind::SystemAudio)
                            .map_err(|error| error.to_string())?,
                    ),
                    RecordingAudioMode::Microphone => Some(
                        plan.audio_plan(PlatformAudioSourceKind::Microphone)
                            .map_err(|error| error.to_string())?,
                    ),
                };
                Ok(PreparedRecordingSource {
                    source_factory: move |control_target, clock| {
                        plan.connect(control_target, clock)
                            .map_err(|error| error.to_string())
                    },
                    descriptor,
                    audio_plan,
                })
            },
            actions,
        )
    }

    pub(super) fn start<S, F, C, A>(
        &self,
        context: RecordingStartContext<'_>,
        request: RecordingStartRequest,
        connect: C,
        actions: &A,
    ) -> Result<RecordingToken, RecordingLifecycleError>
    where
        S: RecordingFrameSource,
        F: FnOnce(RecordingControlTarget, RecordingSessionClock) -> Result<S, String>
            + Send
            + 'static,
        C: FnOnce(RecordingCaptureSpec) -> Result<PreparedRecordingSource<F>, String>,
        A: DesktopActions,
    {
        let RecordingStartContext {
            capture,
            caller,
            selection,
            app_data_dir,
        } = context;
        self.start_with(
            app_data_dir,
            request,
            || {
                let prepared = PreparedRecordingSelection::prepare(capture, caller, selection)?;
                let prepared_source =
                    connect(prepared.spec()).map_err(RecordingLifecycleError::Source)?;
                let handoff = prepared.commit(capture)?;
                Ok(CommittedRecording {
                    source_factory: prepared_source.source_factory,
                    descriptor: prepared_source.descriptor,
                    audio_plan: prepared_source.audio_plan,
                    ownership: handoff.ownership,
                    desktop: DesktopResources {
                        overlays: handoff.resources.overlay_labels(),
                        pins: handoff.resources.lowered_pins,
                        sources: handoff.resources.restore_labels,
                    },
                })
            },
            actions,
        )
    }

    pub(super) fn pause(&self, token: &RecordingToken) -> Result<(), RecordingLifecycleError> {
        self.require_active(token)?;
        self.manager.pause(token)?;
        Ok(())
    }

    pub(super) fn resume(&self, token: &RecordingToken) -> Result<(), RecordingLifecycleError> {
        self.require_active(token)?;
        self.manager.resume(token)?;
        Ok(())
    }

    pub(super) fn has_terminated_worker(
        &self,
        token: &RecordingToken,
    ) -> Result<bool, RecordingLifecycleError> {
        self.require_active(token)?;
        Ok(self.manager.has_terminated_worker(token)?)
    }

    /// 原生托盘没有 WebView caller，可只从当前 Active slot 取得 exact generation。
    pub(super) fn pause_active(&self) -> Result<(), RecordingLifecycleError> {
        let token = self.active_token()?;
        self.pause(&token)
    }

    pub(super) fn resume_active(&self) -> Result<(), RecordingLifecycleError> {
        let token = self.active_token()?;
        self.resume(&token)
    }

    /// 原生菜单需要在事件回调当下固定代次，再把可能阻塞的停止工作交给后台线程。
    pub(super) fn active_token_for_native(
        &self,
    ) -> Result<RecordingToken, RecordingLifecycleError> {
        self.active_token()
    }

    pub(super) fn stop<A: DesktopActions>(
        &self,
        token: &RecordingToken,
        actions: &A,
    ) -> Result<RecordingSessionReport, RecordingLifecycleError> {
        let active = self.claim_terminating(token)?;
        let primary = self
            .manager
            .stop(token)
            .map_err(RecordingLifecycleError::from);
        self.finish_termination(active, actions, primary)
    }

    pub(super) fn cancel<A: DesktopActions>(
        &self,
        token: &RecordingToken,
        actions: &A,
    ) -> Result<(), RecordingLifecycleError> {
        let active = self.claim_terminating(token)?;
        let primary = self
            .manager
            .cancel(token)
            .map_err(RecordingLifecycleError::from);
        self.finish_termination(active, actions, primary)
    }

    fn start_with<S, F, P, A>(
        &self,
        app_data_dir: &Path,
        request: RecordingStartRequest,
        prepare: P,
        actions: &A,
    ) -> Result<RecordingToken, RecordingLifecycleError>
    where
        S: RecordingFrameSource,
        F: FnOnce(RecordingControlTarget, RecordingSessionClock) -> Result<S, String>
            + Send
            + 'static,
        P: FnOnce() -> Result<CommittedRecording<F>, RecordingLifecycleError>,
        A: DesktopActions,
    {
        self.claim_starting()?;
        let committed = match prepare() {
            Ok(committed) => committed,
            Err(error) => {
                self.rollback_starting();
                return Err(error);
            }
        };
        let CommittedRecording {
            source_factory,
            descriptor,
            audio_plan,
            ownership,
            desktop,
        } = committed;
        #[cfg(not(feature = "recording-opus-webm"))]
        let _ = audio_plan;
        {
            let mut slot = self.lock_slot()?;
            if !matches!(*slot, LifecycleSlot::Starting) {
                return Err(RecordingLifecycleError::Superseded);
            }
            *slot = LifecycleSlot::Publishing(PublishingSession {
                session_id: request.session_id.clone(),
                ownership,
                desktop,
            });
        }

        if let Err(error) = self.restore_desktop(actions) {
            return self.fail_publishing(&request.session_id, actions, false, error);
        }
        let control_target = match actions.prepare_control(&request.session_id, &descriptor) {
            Ok(control_target) => control_target,
            Err(message) => {
                return self.fail_publishing(
                    &request.session_id,
                    actions,
                    true,
                    RecordingLifecycleError::Desktop {
                        operation: "prepare_control",
                        message,
                    },
                );
            }
        };

        #[cfg(feature = "recording-opus-webm")]
        let started = match request.audio_mode {
            RecordingAudioMode::None => {
                let config = DiagnosticRecordingConfig {
                    session_id: request.session_id.clone(),
                    source_id: descriptor.source_id,
                    physical_x: descriptor.physical_x,
                    physical_y: descriptor.physical_y,
                    width: descriptor.width,
                    height: descriptor.height,
                    frames_per_second: request.frames_per_second,
                    include_cursor: request.include_cursor,
                    encoder: request.encoder,
                    segment_duration_ns: DEFAULT_SEGMENT_DURATION_NS,
                };
                self.manager
                    .start_with_factory(app_data_dir, config, move |clock| {
                        source_factory(control_target, clock)
                    })
            }
            RecordingAudioMode::SystemAudio | RecordingAudioMode::Microphone => {
                let audio_plan = match audio_plan {
                    Some(plan) => plan,
                    None => {
                        return self.fail_publishing(
                            &request.session_id,
                            actions,
                            true,
                            RecordingLifecycleError::AudioUnavailable,
                        );
                    }
                };
                let config = AvRecordingConfig {
                    session_id: request.session_id.clone(),
                    source_id: descriptor.source_id,
                    physical_x: descriptor.physical_x,
                    physical_y: descriptor.physical_y,
                    width: descriptor.width,
                    height: descriptor.height,
                    frames_per_second: request.frames_per_second,
                    include_cursor: request.include_cursor,
                    audio_channels: audio_plan.channels(),
                    segment_duration_ns: DEFAULT_SEGMENT_DURATION_NS,
                };
                self.manager.start_av_with_factories(
                    app_data_dir,
                    config,
                    move |clock| source_factory(control_target, clock),
                    move |clock| audio_plan.connect(clock).map_err(|error| error.to_string()),
                )
            }
        };
        #[cfg(not(feature = "recording-opus-webm"))]
        let started = {
            debug_assert_eq!(request.audio_mode, RecordingAudioMode::None);
            let config = DiagnosticRecordingConfig {
                session_id: request.session_id.clone(),
                source_id: descriptor.source_id,
                physical_x: descriptor.physical_x,
                physical_y: descriptor.physical_y,
                width: descriptor.width,
                height: descriptor.height,
                frames_per_second: request.frames_per_second,
                include_cursor: request.include_cursor,
                encoder: request.encoder,
                segment_duration_ns: DEFAULT_SEGMENT_DURATION_NS,
            };
            self.manager
                .start_with_factory(app_data_dir, config, move |clock| {
                    source_factory(control_target, clock)
                })
        };
        let token = match started {
            Ok(token) => token,
            Err(error) => {
                return self.fail_publishing(
                    &request.session_id,
                    actions,
                    true,
                    RecordingLifecycleError::Manager(error),
                );
            }
        };
        if let Err(message) = actions.bind_control(&token) {
            if let Err(error) = self.manager.cancel(&token) {
                log::error!("录屏控制面绑定失败后取消会话也失败: {error}");
            }
            return self.fail_publishing(
                &request.session_id,
                actions,
                true,
                RecordingLifecycleError::Desktop {
                    operation: "bind_control",
                    message,
                },
            );
        }
        let publishing = match self.take_publishing(&request.session_id) {
            Ok(publishing) => publishing,
            Err(error) => {
                let _ = self.manager.cancel(&token);
                let _ = actions.close_control(&request.session_id);
                return Err(error);
            }
        };
        let mut slot = self.lock_slot()?;
        *slot = LifecycleSlot::Active(ActiveSession {
            token: token.clone(),
            ownership: publishing.ownership,
        });
        Ok(token)
    }

    fn restore_desktop<A: DesktopActions>(
        &self,
        actions: &A,
    ) -> Result<(), RecordingLifecycleError> {
        let resources = {
            let slot = self.lock_slot()?;
            match &*slot {
                LifecycleSlot::Publishing(session) => session.desktop.clone(),
                _ => return Err(RecordingLifecycleError::Superseded),
            }
        };
        let mut first_error = None;
        for (operation, result) in [
            (
                "close_overlays",
                actions.close_overlays(&resources.overlays),
            ),
            ("restore_pins", actions.restore_pins(&resources.pins)),
            (
                "restore_sources",
                actions.restore_sources(&resources.sources),
            ),
            ("settle_after_restore", actions.settle_after_restore()),
        ] {
            if first_error.is_none() {
                if let Err(message) = result {
                    first_error = Some(RecordingLifecycleError::Desktop { operation, message });
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn fail_publishing<T, A: DesktopActions>(
        &self,
        session_id: &str,
        actions: &A,
        control_published: bool,
        primary: RecordingLifecycleError,
    ) -> Result<T, RecordingLifecycleError> {
        if control_published {
            if let Err(error) = actions.close_control(session_id) {
                log::error!("录屏启动失败后关闭控制面也失败: {error}");
            }
        }
        let publishing = self.take_publishing(session_id)?;
        let primary_message = primary.to_string();
        match self.release_ownership(publishing.ownership, session_id) {
            Ok(()) => Err(primary),
            Err(release) => Err(RecordingLifecycleError::ReleaseAfterFailure {
                primary: primary_message,
                release,
            }),
        }
    }

    fn finish_termination<T, A: DesktopActions>(
        &self,
        active: ActiveSession,
        actions: &A,
        primary: Result<T, RecordingLifecycleError>,
    ) -> Result<T, RecordingLifecycleError> {
        let session_id = active.token.session_id.clone();
        let result = match (primary, actions.close_control(&session_id)) {
            (Ok(value), Ok(())) => Ok(value),
            (Ok(_), Err(message)) => Err(RecordingLifecycleError::Desktop {
                operation: "close_control",
                message,
            }),
            (Err(error), secondary) => {
                if let Err(secondary) = secondary {
                    log::error!("录屏主流程失败后关闭控制面也失败: {secondary}");
                }
                Err(error)
            }
        };
        let primary_message = result.as_ref().err().map(ToString::to_string);
        match self.release_ownership(active.ownership, &session_id) {
            Ok(()) => result,
            Err(release) => Err(RecordingLifecycleError::ReleaseAfterFailure {
                primary: primary_message
                    .unwrap_or_else(|| "录屏已结束，但释放模式所有权失败".to_string()),
                release,
            }),
        }
    }

    fn release_ownership(
        &self,
        ownership: CaptureModeOwnership,
        session_id: &str,
    ) -> Result<(), CaptureError> {
        {
            let mut slot = self
                .slot
                .lock()
                .map_err(|error| CaptureError::StateLock(error.to_string()))?;
            match &*slot {
                LifecycleSlot::Starting => {}
                LifecycleSlot::Terminating(token) if token.session_id == session_id => {}
                _ => return Err(CaptureError::CaptureModeSuperseded),
            }
            *slot = LifecycleSlot::Releasing(session_id.to_string());
        }
        let release = ownership.release();
        let mut slot = self
            .slot
            .lock()
            .map_err(|error| CaptureError::StateLock(error.to_string()))?;
        match &*slot {
            LifecycleSlot::Releasing(active) if active == session_id => {}
            _ => return Err(CaptureError::CaptureModeSuperseded),
        }
        match release {
            Ok(()) => {
                *slot = LifecycleSlot::Empty;
                Ok(())
            }
            Err(error) => {
                *slot = LifecycleSlot::TerminalFailed;
                Err(error)
            }
        }
    }

    fn active_token(&self) -> Result<RecordingToken, RecordingLifecycleError> {
        let slot = self.lock_slot()?;
        match &*slot {
            LifecycleSlot::Active(active) => Ok(active.token.clone()),
            LifecycleSlot::Empty => Err(RecordingLifecycleError::Missing),
            LifecycleSlot::Starting
            | LifecycleSlot::Publishing(_)
            | LifecycleSlot::Terminating(_)
            | LifecycleSlot::Releasing(_) => Err(RecordingLifecycleError::Busy),
            LifecycleSlot::TerminalFailed => Err(RecordingLifecycleError::Superseded),
        }
    }

    fn claim_starting(&self) -> Result<(), RecordingLifecycleError> {
        let mut slot = self.lock_slot()?;
        if !matches!(*slot, LifecycleSlot::Empty) {
            return Err(RecordingLifecycleError::Busy);
        }
        *slot = LifecycleSlot::Starting;
        Ok(())
    }

    fn rollback_starting(&self) {
        match self.slot.lock() {
            Ok(mut slot) if matches!(*slot, LifecycleSlot::Starting) => {
                *slot = LifecycleSlot::Empty;
            }
            Ok(_) => log::error!("录屏准备失败后 lifecycle 已不再是 Starting"),
            Err(error) => log::error!("录屏准备失败后回滚 lifecycle 失败: {error}"),
        }
    }

    fn take_publishing(
        &self,
        session_id: &str,
    ) -> Result<PublishingSession, RecordingLifecycleError> {
        let mut slot = self.lock_slot()?;
        let previous = std::mem::replace(&mut *slot, LifecycleSlot::Starting);
        match previous {
            LifecycleSlot::Publishing(session) if session.session_id == session_id => Ok(session),
            other => {
                *slot = other;
                Err(RecordingLifecycleError::Superseded)
            }
        }
    }

    fn require_active(&self, token: &RecordingToken) -> Result<(), RecordingLifecycleError> {
        let slot = self.lock_slot()?;
        match &*slot {
            LifecycleSlot::Empty => Err(RecordingLifecycleError::Missing),
            LifecycleSlot::Active(active) if active.token == *token => Ok(()),
            LifecycleSlot::Active(_) => Err(RecordingLifecycleError::Superseded),
            LifecycleSlot::Starting
            | LifecycleSlot::Publishing(_)
            | LifecycleSlot::Terminating(_)
            | LifecycleSlot::Releasing(_)
            | LifecycleSlot::TerminalFailed => Err(RecordingLifecycleError::Busy),
        }
    }

    fn claim_terminating(
        &self,
        token: &RecordingToken,
    ) -> Result<ActiveSession, RecordingLifecycleError> {
        let mut slot = self.lock_slot()?;
        match &*slot {
            LifecycleSlot::Empty => return Err(RecordingLifecycleError::Missing),
            LifecycleSlot::Active(active) if active.token != *token => {
                return Err(RecordingLifecycleError::Superseded);
            }
            LifecycleSlot::Active(_) => {}
            LifecycleSlot::Starting
            | LifecycleSlot::Publishing(_)
            | LifecycleSlot::Terminating(_)
            | LifecycleSlot::Releasing(_)
            | LifecycleSlot::TerminalFailed => return Err(RecordingLifecycleError::Busy),
        }
        let active = match std::mem::replace(&mut *slot, LifecycleSlot::Starting) {
            LifecycleSlot::Active(active) => active,
            _ => unreachable!("已在同一把锁内确认 matching Active"),
        };
        *slot = LifecycleSlot::Terminating(active.token.clone());
        Ok(active)
    }

    fn lock_slot(&self) -> Result<MutexGuard<'_, LifecycleSlot>, RecordingLifecycleError> {
        self.slot
            .lock()
            .map_err(|_| RecordingLifecycleError::Poisoned)
    }
}

struct CommittedRecording<F> {
    source_factory: F,
    descriptor: RecordingSourceDescriptor,
    audio_plan: Option<PlatformAudioSourcePlan>,
    ownership: CaptureModeOwnership,
    desktop: DesktopResources,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{CaptureMode, CaptureModeGate};
    use crate::recording::frame::CapturedFrame;
    use crate::recording::manager::RecordingManagerStatus;
    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    #[test]
    fn audio_modes_follow_the_backend_build_gate() {
        let modes = available_recording_audio_modes();
        assert_eq!(modes.first(), Some(&RecordingAudioMode::None));
        assert_eq!(
            modes
                .iter()
                .filter(|mode| **mode == RecordingAudioMode::None)
                .count(),
            1
        );
        if cfg!(all(target_os = "linux", feature = "recording-linux-av-qa"))
            || cfg!(all(
                target_os = "windows",
                feature = "recording-windows-av-qa"
            ))
        {
            assert_eq!(
                modes,
                &[
                    RecordingAudioMode::None,
                    RecordingAudioMode::SystemAudio,
                    RecordingAudioMode::Microphone,
                ]
            );
        } else if cfg!(all(target_os = "macos", feature = "recording-macos-av-qa"))
            && crate::recording::macos_screencapturekit_audio_runtime_available()
        {
            let expected =
                if crate::recording::macos_screencapturekit_microphone_runtime_available() {
                    &[
                        RecordingAudioMode::None,
                        RecordingAudioMode::SystemAudio,
                        RecordingAudioMode::Microphone,
                    ][..]
                } else {
                    &[RecordingAudioMode::None, RecordingAudioMode::SystemAudio][..]
                };
            assert_eq!(modes, expected);
            assert!(recording_audio_mode_available(
                RecordingAudioMode::SystemAudio
            ));
            assert_eq!(
                recording_audio_mode_available(RecordingAudioMode::Microphone),
                crate::recording::macos_screencapturekit_microphone_runtime_available()
            );
        } else {
            assert_eq!(modes, &[RecordingAudioMode::None]);
            assert!(!recording_audio_mode_available(
                RecordingAudioMode::SystemAudio
            ));
            assert!(!recording_audio_mode_available(
                RecordingAudioMode::Microphone
            ));
        }
    }

    #[test]
    fn audio_mode_wire_values_are_stable_and_unknown_values_fail_closed() {
        assert_eq!(
            serde_json::to_string(&RecordingAudioMode::SystemAudio).unwrap(),
            "\"systemAudio\""
        );
        assert_eq!(
            serde_json::to_string(&RecordingAudioMode::Microphone).unwrap(),
            "\"microphone\""
        );
        assert!(serde_json::from_str::<RecordingAudioMode>("\"camera\"").is_err());
    }

    #[test]
    fn macos_audio_mode_policy_keeps_12_13_and_15_distinct() {
        assert_eq!(
            macos_recording_audio_modes(false, false),
            &[RecordingAudioMode::None]
        );
        assert_eq!(
            macos_recording_audio_modes(true, false),
            &[RecordingAudioMode::None, RecordingAudioMode::SystemAudio]
        );
        assert_eq!(
            macos_recording_audio_modes(true, true),
            &[
                RecordingAudioMode::None,
                RecordingAudioMode::SystemAudio,
                RecordingAudioMode::Microphone,
            ]
        );
        assert_eq!(
            macos_recording_audio_modes(false, true),
            &[RecordingAudioMode::None]
        );
        assert!(!macos_recording_audio_modes(true, false).contains(&RecordingAudioMode::Microphone));
    }

    struct FixtureSource {
        events: Arc<Mutex<Vec<String>>>,
        sequence: u64,
        timestamp_ns: u64,
    }

    impl RecordingFrameSource for FixtureSource {
        type Error = Infallible;

        fn capture_next(&mut self) -> Result<CapturedFrame, Self::Error> {
            self.events.lock().unwrap().push("capture".to_string());
            let sequence = self.sequence;
            let captured_at_ns = self.timestamp_ns;
            self.sequence += 1;
            self.timestamp_ns += 100_000_000;
            Ok(CapturedFrame {
                sequence,
                captured_at_ns,
                width: 2,
                height: 2,
                stride: 8,
                rgba: vec![sequence as u8; 16].into_boxed_slice(),
            })
        }

        fn control_timestamp_ns(&mut self) -> Result<u64, Self::Error> {
            let timestamp_ns = self.timestamp_ns;
            self.timestamp_ns += 1;
            Ok(timestamp_ns)
        }
    }

    struct RecordingActions {
        gate: Arc<CaptureModeGate>,
        events: Arc<Mutex<Vec<String>>>,
        fail_operation: Mutex<Option<&'static str>>,
        control_target: RecordingControlTarget,
    }

    impl RecordingActions {
        fn new(gate: Arc<CaptureModeGate>, events: Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                gate,
                events,
                fail_operation: Mutex::new(None),
                control_target: RecordingControlTarget::NoNativeWindow,
            }
        }

        fn with_control_target(mut self, control_target: RecordingControlTarget) -> Self {
            self.control_target = control_target;
            self
        }

        fn fail(&self, operation: &'static str) {
            *self.fail_operation.lock().unwrap() = Some(operation);
        }

        fn record(&self, operation: &'static str, detail: impl Into<String>) -> Result<(), String> {
            assert_eq!(
                self.gate.active_mode().unwrap(),
                Some(CaptureMode::Recording),
                "{operation} 执行前必须已经完成 Ordinary → Recording"
            );
            self.events
                .lock()
                .unwrap()
                .push(format!("{operation}:{}", detail.into()));
            if *self.fail_operation.lock().unwrap() == Some(operation) {
                Err(format!("{operation} fixture failure"))
            } else {
                Ok(())
            }
        }
    }

    impl DesktopActions for RecordingActions {
        fn close_overlays(&self, labels: &[String]) -> Result<(), String> {
            self.record("close_overlays", labels.join(","))
        }

        fn restore_pins(&self, labels: &[String]) -> Result<(), String> {
            self.record("restore_pins", labels.join(","))
        }

        fn restore_sources(&self, labels: &[String]) -> Result<(), String> {
            self.record("restore_sources", labels.join(","))
        }

        fn settle_after_restore(&self) -> Result<(), String> {
            self.record("settle_after_restore", "desktop")
        }

        fn prepare_control(
            &self,
            session_id: &str,
            descriptor: &RecordingSourceDescriptor,
        ) -> Result<RecordingControlTarget, String> {
            self.record(
                "prepare_control",
                format!(
                    "{session_id}@{},{}:{}x{}",
                    descriptor.physical_x,
                    descriptor.physical_y,
                    descriptor.width,
                    descriptor.height
                ),
            )?;
            Ok(self.control_target.clone())
        }

        fn bind_control(&self, token: &RecordingToken) -> Result<(), String> {
            self.record(
                "bind_control",
                format!("{}#{}", token.session_id, token.generation),
            )
        }

        fn close_control(&self, session_id: &str) -> Result<(), String> {
            self.record("close_control", session_id)
        }
    }

    fn request(session_id: &str) -> RecordingStartRequest {
        RecordingStartRequest {
            session_id: session_id.to_string(),
            frames_per_second: 10,
            include_cursor: true,
            audio_mode: RecordingAudioMode::None,
            encoder: RecordingEncoder::MjpegDiagnostic { jpeg_quality: 85 },
        }
    }

    fn committed(
        gate: &Arc<CaptureModeGate>,
        events: Arc<Mutex<Vec<String>>>,
    ) -> CommittedRecording<
        impl FnOnce(RecordingControlTarget, RecordingSessionClock) -> Result<FixtureSource, String>
            + Send
            + 'static,
    > {
        let ownership = Arc::clone(gate)
            .try_claim_owned(CaptureMode::Ordinary)
            .unwrap()
            .into_recording()
            .unwrap();
        events.lock().unwrap().push("prepared".to_string());
        let source_events = Arc::clone(&events);
        CommittedRecording {
            source_factory: move |control_target, _clock| {
                assert_eq!(control_target, RecordingControlTarget::NoNativeWindow);
                source_events
                    .lock()
                    .unwrap()
                    .push("initialized".to_string());
                Ok(FixtureSource {
                    events: source_events,
                    sequence: 0,
                    timestamp_ns: 100,
                })
            },
            descriptor: RecordingSourceDescriptor {
                source_id: "fixture-monitor".to_string(),
                physical_x: -120,
                physical_y: 40,
                width: 2,
                height: 2,
            },
            audio_plan: None,
            ownership,
            desktop: DesktopResources {
                overlays: vec!["overlay-a".to_string(), "overlay-b".to_string()],
                pins: vec!["pin-a".to_string()],
                sources: vec!["main".to_string()],
            },
        }
    }

    #[test]
    fn desktop_and_control_are_ready_before_capture_and_stop_releases_gate() {
        let temporary = tempfile::tempdir().unwrap();
        let gate = Arc::new(CaptureModeGate::new());
        let events = Arc::new(Mutex::new(Vec::new()));
        let actions = RecordingActions::new(Arc::clone(&gate), Arc::clone(&events));
        let lifecycle = RecordingLifecycle::new();
        let token = lifecycle
            .start_with(
                temporary.path(),
                request("ordered"),
                || Ok(committed(&gate, Arc::clone(&events))),
                &actions,
            )
            .unwrap();
        let report = lifecycle.stop(&token, &actions).unwrap();
        assert_eq!(report.segment_paths.len(), 1);
        assert!(report.segment_paths[0].exists());
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(
                report.segment_paths[0]
                    .parent()
                    .unwrap()
                    .join("manifest.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["video"]["encoder"], "mjpeg-diagnostic");
        assert_eq!(manifest["video"]["container"], "avi");
        assert_eq!(gate.active_mode().unwrap(), None);
        assert_eq!(
            lifecycle.manager.status().unwrap(),
            RecordingManagerStatus::Idle
        );

        let recorded = events.lock().unwrap().clone();
        let control_index = recorded
            .iter()
            .position(|event| event.starts_with("prepare_control:"))
            .unwrap();
        let capture_index = recorded
            .iter()
            .position(|event| event == "capture")
            .unwrap();
        let initialization_index = recorded
            .iter()
            .position(|event| event == "initialized")
            .unwrap();
        let close_index = recorded
            .iter()
            .position(|event| event == "close_control:ordered")
            .unwrap();
        assert!(control_index < initialization_index);
        assert!(initialization_index < capture_index);
        assert!(capture_index < close_index);
        assert_eq!(
            &recorded[..control_index],
            &[
                "prepared",
                "close_overlays:overlay-a,overlay-b",
                "restore_pins:pin-a",
                "restore_sources:main",
                "settle_after_restore:desktop",
            ]
        );
        let bind_index = recorded
            .iter()
            .position(|event| event.starts_with("bind_control:ordered#"))
            .unwrap();
        assert!(control_index < bind_index);
        assert!(bind_index < close_index);
    }

    #[test]
    fn native_control_target_is_created_before_source_connection() {
        let temporary = tempfile::tempdir().unwrap();
        let gate = Arc::new(CaptureModeGate::new());
        let events = Arc::new(Mutex::new(Vec::new()));
        let target = RecordingControlTarget::native_window(418);
        let actions = RecordingActions::new(Arc::clone(&gate), Arc::clone(&events))
            .with_control_target(target.clone());
        let lifecycle = RecordingLifecycle::new();
        let ownership = Arc::clone(&gate)
            .try_claim_owned(CaptureMode::Ordinary)
            .unwrap()
            .into_recording()
            .unwrap();
        events.lock().unwrap().push("prepared".to_string());
        let source_events = Arc::clone(&events);
        let committed = CommittedRecording {
            source_factory: move |received, _clock| {
                assert_eq!(received, target);
                source_events
                    .lock()
                    .unwrap()
                    .push("initialized-with-control-target".to_string());
                Ok(FixtureSource {
                    events: source_events,
                    sequence: 0,
                    timestamp_ns: 100,
                })
            },
            descriptor: RecordingSourceDescriptor {
                source_id: "fixture-monitor".to_string(),
                physical_x: -120,
                physical_y: 40,
                width: 2,
                height: 2,
            },
            audio_plan: None,
            ownership,
            desktop: DesktopResources {
                overlays: vec!["overlay-a".to_string()],
                pins: Vec::new(),
                sources: Vec::new(),
            },
        };
        let token = lifecycle
            .start_with(
                temporary.path(),
                request("native-target"),
                || Ok(committed),
                &actions,
            )
            .unwrap();
        lifecycle.cancel(&token, &actions).unwrap();

        let recorded = events.lock().unwrap();
        let prepare = recorded
            .iter()
            .position(|event| event.starts_with("prepare_control:"))
            .unwrap();
        let connect = recorded
            .iter()
            .position(|event| event == "initialized-with-control-target")
            .unwrap();
        assert!(prepare < connect);
    }

    #[cfg(feature = "recording-vp9-prototype")]
    #[test]
    fn lifecycle_uses_the_backend_selected_vp9_encoder() {
        let temporary = tempfile::tempdir().unwrap();
        let gate = Arc::new(CaptureModeGate::new());
        let events = Arc::new(Mutex::new(Vec::new()));
        let actions = RecordingActions::new(Arc::clone(&gate), Arc::clone(&events));
        let lifecycle = RecordingLifecycle::new();
        let mut request = request("vp9-policy");
        request.encoder = RecordingEncoder::Vp9Prototype;

        let token = lifecycle
            .start_with(
                temporary.path(),
                request,
                || Ok(committed(&gate, Arc::clone(&events))),
                &actions,
            )
            .unwrap();
        let report = lifecycle.stop(&token, &actions).unwrap();

        let output = report
            .final_output_path
            .as_ref()
            .expect("VP9 生命周期必须提交单一最终文件");
        assert_eq!(
            output.extension().and_then(|value| value.to_str()),
            Some("webm")
        );
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(output.parent().unwrap().join("manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["video"]["encoder"], "vp9-prototype");
        assert_eq!(manifest["video"]["container"], "webm");
        assert_eq!(gate.active_mode().unwrap(), None);
    }

    #[test]
    fn desktop_failure_runs_remaining_cleanup_and_never_starts_session() {
        let temporary = tempfile::tempdir().unwrap();
        let gate = Arc::new(CaptureModeGate::new());
        let events = Arc::new(Mutex::new(Vec::new()));
        let actions = RecordingActions::new(Arc::clone(&gate), Arc::clone(&events));
        actions.fail("restore_pins");
        let lifecycle = RecordingLifecycle::new();
        let error = lifecycle
            .start_with(
                temporary.path(),
                request("desktop-failure"),
                || Ok(committed(&gate, Arc::clone(&events))),
                &actions,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            RecordingLifecycleError::Desktop {
                operation: "restore_pins",
                ..
            }
        ));
        assert_eq!(gate.active_mode().unwrap(), None);
        assert_eq!(
            lifecycle.manager.status().unwrap(),
            RecordingManagerStatus::Idle
        );
        assert_eq!(
            events.lock().unwrap().as_slice(),
            &[
                "prepared",
                "close_overlays:overlay-a,overlay-b",
                "restore_pins:pin-a",
                "restore_sources:main",
                "settle_after_restore:desktop",
            ]
        );
        assert!(!temporary.path().join("recordings").exists());
    }

    #[test]
    fn session_start_failure_closes_control_and_releases_gate() {
        let temporary = tempfile::tempdir().unwrap();
        let gate = Arc::new(CaptureModeGate::new());
        let events = Arc::new(Mutex::new(Vec::new()));
        let actions = RecordingActions::new(Arc::clone(&gate), Arc::clone(&events));
        let lifecycle = RecordingLifecycle::new();
        let mut invalid = request("invalid-fps");
        invalid.frames_per_second = 0;
        let error = lifecycle
            .start_with(
                temporary.path(),
                invalid,
                || Ok(committed(&gate, Arc::clone(&events))),
                &actions,
            )
            .unwrap_err();
        assert!(matches!(error, RecordingLifecycleError::Manager(_)));
        assert_eq!(gate.active_mode().unwrap(), None);
        assert_eq!(
            lifecycle.manager.status().unwrap(),
            RecordingManagerStatus::Idle
        );
        let recorded = events.lock().unwrap();
        assert!(recorded
            .iter()
            .any(|event| event == "close_control:invalid-fps"));
        assert!(!temporary.path().join("recordings").exists());
    }

    #[test]
    fn source_initialization_failure_closes_control_and_releases_gate() {
        let temporary = tempfile::tempdir().unwrap();
        let gate = Arc::new(CaptureModeGate::new());
        let events = Arc::new(Mutex::new(Vec::new()));
        let actions = RecordingActions::new(Arc::clone(&gate), Arc::clone(&events));
        let lifecycle = RecordingLifecycle::new();
        let error = lifecycle
            .start_with(
                temporary.path(),
                request("source-failure"),
                || {
                    let ownership = Arc::clone(&gate)
                        .try_claim_owned(CaptureMode::Ordinary)
                        .unwrap()
                        .into_recording()
                        .unwrap();
                    events.lock().unwrap().push("prepared".to_string());
                    Ok(CommittedRecording {
                        source_factory: |_, _clock| {
                            Err::<FixtureSource, _>("fixture native source failure".to_string())
                        },
                        descriptor: RecordingSourceDescriptor {
                            source_id: "fixture-monitor".to_string(),
                            physical_x: -120,
                            physical_y: 40,
                            width: 2,
                            height: 2,
                        },
                        audio_plan: None,
                        ownership,
                        desktop: DesktopResources {
                            overlays: vec!["overlay-a".to_string()],
                            pins: Vec::new(),
                            sources: Vec::new(),
                        },
                    })
                },
                &actions,
            )
            .unwrap_err();
        assert!(matches!(error, RecordingLifecycleError::Manager(_)));
        assert!(error.to_string().contains("fixture native source failure"));
        assert_eq!(gate.active_mode().unwrap(), None);
        assert_eq!(
            lifecycle.manager.status().unwrap(),
            RecordingManagerStatus::Idle
        );
        let recorded = events.lock().unwrap();
        let prepare = recorded
            .iter()
            .position(|event| event.starts_with("prepare_control:"))
            .unwrap();
        let close = recorded
            .iter()
            .position(|event| event == "close_control:source-failure")
            .unwrap();
        assert!(prepare < close);
        assert!(!recorded.iter().any(|event| event == "capture"));
    }

    #[test]
    fn uncertain_control_prepare_is_closed_before_gate_release() {
        let temporary = tempfile::tempdir().unwrap();
        let gate = Arc::new(CaptureModeGate::new());
        let events = Arc::new(Mutex::new(Vec::new()));
        let actions = RecordingActions::new(Arc::clone(&gate), Arc::clone(&events));
        actions.fail("prepare_control");
        let lifecycle = RecordingLifecycle::new();
        let error = lifecycle
            .start_with(
                temporary.path(),
                request("control-failure"),
                || Ok(committed(&gate, Arc::clone(&events))),
                &actions,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            RecordingLifecycleError::Desktop {
                operation: "prepare_control",
                ..
            }
        ));
        assert_eq!(gate.active_mode().unwrap(), None);
        assert_eq!(
            lifecycle.manager.status().unwrap(),
            RecordingManagerStatus::Idle
        );
        let recorded = events.lock().unwrap();
        let publish = recorded
            .iter()
            .position(|event| event.starts_with("prepare_control:"))
            .unwrap();
        let close = recorded
            .iter()
            .position(|event| event == "close_control:control-failure")
            .unwrap();
        assert!(publish < close);
    }

    #[test]
    fn control_binding_failure_cancels_started_session_before_gate_release() {
        let temporary = tempfile::tempdir().unwrap();
        let gate = Arc::new(CaptureModeGate::new());
        let events = Arc::new(Mutex::new(Vec::new()));
        let actions = RecordingActions::new(Arc::clone(&gate), Arc::clone(&events));
        actions.fail("bind_control");
        let lifecycle = RecordingLifecycle::new();
        let error = lifecycle
            .start_with(
                temporary.path(),
                request("bind-failure"),
                || Ok(committed(&gate, Arc::clone(&events))),
                &actions,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            RecordingLifecycleError::Desktop {
                operation: "bind_control",
                ..
            }
        ));
        assert_eq!(gate.active_mode().unwrap(), None);
        assert_eq!(
            lifecycle.manager.status().unwrap(),
            RecordingManagerStatus::Idle
        );
        let recorded = events.lock().unwrap();
        let bind = recorded
            .iter()
            .position(|event| event.starts_with("bind_control:bind-failure#"))
            .unwrap();
        let close = recorded
            .iter()
            .position(|event| event == "close_control:bind-failure")
            .unwrap();
        assert!(bind < close);
    }

    #[test]
    fn stale_token_cannot_stop_replacement_lifecycle() {
        let temporary = tempfile::tempdir().unwrap();
        let gate = Arc::new(CaptureModeGate::new());
        let events = Arc::new(Mutex::new(Vec::new()));
        let actions = RecordingActions::new(Arc::clone(&gate), Arc::clone(&events));
        let lifecycle = RecordingLifecycle::new();
        let first = lifecycle
            .start_with(
                temporary.path(),
                request("first"),
                || Ok(committed(&gate, Arc::clone(&events))),
                &actions,
            )
            .unwrap();
        let deferred_native_action = lifecycle.active_token_for_native().unwrap();
        assert_eq!(deferred_native_action, first);
        lifecycle.cancel(&first, &actions).unwrap();
        let second = lifecycle
            .start_with(
                temporary.path(),
                request("second"),
                || Ok(committed(&gate, Arc::clone(&events))),
                &actions,
            )
            .unwrap();
        assert!(matches!(
            lifecycle.stop(&deferred_native_action, &actions),
            Err(RecordingLifecycleError::Superseded)
        ));
        assert_eq!(
            lifecycle.manager.status().unwrap(),
            RecordingManagerStatus::Recording(second.clone())
        );
        lifecycle.cancel(&second, &actions).unwrap();
        assert_eq!(gate.active_mode().unwrap(), None);
    }

    #[test]
    fn close_control_failure_still_releases_gate_and_session() {
        let temporary = tempfile::tempdir().unwrap();
        let gate = Arc::new(CaptureModeGate::new());
        let events = Arc::new(Mutex::new(Vec::new()));
        let actions = RecordingActions::new(Arc::clone(&gate), Arc::clone(&events));
        let lifecycle = RecordingLifecycle::new();
        let token = lifecycle
            .start_with(
                temporary.path(),
                request("close-failure"),
                || Ok(committed(&gate, Arc::clone(&events))),
                &actions,
            )
            .unwrap();
        actions.fail("close_control");
        let error = lifecycle.stop(&token, &actions).unwrap_err();
        assert!(matches!(
            error,
            RecordingLifecycleError::Desktop {
                operation: "close_control",
                ..
            }
        ));
        assert_eq!(gate.active_mode().unwrap(), None);
        assert_eq!(
            lifecycle.manager.status().unwrap(),
            RecordingManagerStatus::Idle
        );
    }
}
