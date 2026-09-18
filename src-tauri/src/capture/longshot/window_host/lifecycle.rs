//! 控制窗创建、显示、隐藏、销毁、deadline 与取消补偿副作用。

use super::model::{
    CancelAction, CancelFailureRecovery, DeadlineAction, EmergencyDecision,
    LongshotControllerLaunch, LongshotControllerRegistry, LongshotIpcError, ReadyAction,
};
use super::{cancel_claimed, LOAD_DEADLINE_MS};
use crate::capture::longshot::LongshotSessionToken;
use crate::capture::{CaptureError, CaptureSelection};
use crate::commands::AppState;
use std::time::Duration;
use tauri::{Emitter, Manager};

pub(super) trait ControlWindowActions {
    fn destroy(&self, label: &str);
    fn hide(&self, label: &str) -> Result<(), LongshotIpcError>;
    fn show(&self, label: &str) -> Result<(), LongshotIpcError>;
    fn focus(&self, label: &str);
    fn exists(&self, label: &str) -> bool;
}

pub(super) struct TauriControlWindowActions<'a> {
    pub(super) app: &'a tauri::AppHandle,
}

impl ControlWindowActions for TauriControlWindowActions<'_> {
    fn destroy(&self, label: &str) {
        if let Some(guide_label) = super::guide_label(label) {
            if let Some(window) = self.app.get_webview_window(&guide_label) {
                if let Err(error) = window.destroy() {
                    log::warn!("销毁长截图 guide {guide_label} 失败: {error}");
                }
            }
        }
        if let Some(window) = self.app.get_webview_window(label) {
            if let Err(error) = window.destroy() {
                log::warn!("销毁长截图控制窗 {label} 失败: {error}");
            }
        }
    }

    fn hide(&self, label: &str) -> Result<(), LongshotIpcError> {
        let guide_label = super::guide_label(label).ok_or_else(LongshotIpcError::missing)?;
        let guide = self
            .app
            .get_webview_window(&guide_label)
            .ok_or_else(LongshotIpcError::missing)?;
        guide.hide().map_err(|error| {
            LongshotIpcError::new("longshot_guide_hide_failed", error.to_string())
        })?;
        let window = self
            .app
            .get_webview_window(label)
            .ok_or_else(LongshotIpcError::missing)?;
        window.hide().map_err(|error| {
            LongshotIpcError::new("longshot_controller_hide_failed", error.to_string())
        })
    }

    fn show(&self, label: &str) -> Result<(), LongshotIpcError> {
        let guide_label = super::guide_label(label).ok_or_else(LongshotIpcError::missing)?;
        let guide = self
            .app
            .get_webview_window(&guide_label)
            .ok_or_else(LongshotIpcError::missing)?;
        guide.show().map_err(|error| {
            LongshotIpcError::new("longshot_guide_show_failed", error.to_string())
        })?;
        let window = self
            .app
            .get_webview_window(label)
            .ok_or_else(LongshotIpcError::missing)?;
        if let Err(error) = window.show() {
            let _ = guide.hide();
            return Err(LongshotIpcError::new(
                "longshot_controller_show_failed",
                error.to_string(),
            ));
        }
        Ok(())
    }

    fn focus(&self, label: &str) {
        if let Some(window) = self.app.get_webview_window(label) {
            let _ = window.set_focus();
        }
    }

    fn exists(&self, label: &str) -> bool {
        self.app.get_webview_window(label).is_some()
    }
}

pub(super) fn emergency_decision(
    cleanup_succeeded: bool,
    registry_settled: bool,
) -> EmergencyDecision {
    if cleanup_succeeded && registry_settled {
        EmergencyDecision::Destroy
    } else if registry_settled {
        EmergencyDecision::AwaitReady
    } else {
        EmergencyDecision::Reveal
    }
}

pub(super) async fn execute_cancel_action<A, C, F>(
    action: CancelAction,
    label: &str,
    windows: &A,
    cancel: C,
) -> Result<(), LongshotIpcError>
where
    A: ControlWindowActions,
    C: FnOnce(LongshotSessionToken) -> F,
    F: std::future::Future<Output = Result<(), LongshotIpcError>>,
{
    match action {
        CancelAction::Close | CancelAction::Requested => {
            windows.destroy(label);
            Ok(())
        }
        CancelAction::Terminate(token) => {
            let result = cancel(token).await;
            if result.is_ok() {
                windows.destroy(label);
            }
            result
        }
    }
}

pub(super) async fn execute_deadline_action<A, C, F>(
    action: DeadlineAction,
    label: &str,
    windows: &A,
    cancel: C,
) where
    A: ControlWindowActions,
    C: FnOnce(LongshotSessionToken) -> F,
    F: std::future::Future<Output = Result<(), LongshotIpcError>>,
{
    if let Some(token) = execute_deadline_window_action(action, label, windows) {
        let _ = cancel(token).await;
        windows.destroy(label);
    }
}

pub(super) fn execute_deadline_window_action<A: ControlWindowActions>(
    action: DeadlineAction,
    label: &str,
    windows: &A,
) -> Option<LongshotSessionToken> {
    match action {
        DeadlineAction::None => None,
        DeadlineAction::Close => {
            windows.destroy(label);
            None
        }
        DeadlineAction::Terminate(token) => Some(token),
    }
}

pub(super) async fn execute_token_cleanup<C, F>(token: Option<LongshotSessionToken>, cancel: C)
where
    C: FnOnce(LongshotSessionToken) -> F,
    F: std::future::Future<Output = Result<(), LongshotIpcError>>,
{
    if let Some(token) = token {
        let _ = cancel(token).await;
    }
}

pub(super) async fn execute_ready_action<A, C, F>(
    registry: &LongshotControllerRegistry,
    action: ReadyAction,
    label: &str,
    windows: &A,
    cancel: C,
) -> Result<(), LongshotIpcError>
where
    A: ControlWindowActions,
    C: FnOnce(LongshotSessionToken) -> F,
    F: std::future::Future<Output = Result<(), LongshotIpcError>>,
{
    let (token, cleanup) = match action {
        ReadyAction::None => return Ok(()),
        ReadyAction::ShowFailed => (None, false),
        ReadyAction::ShowCleanup => (None, true),
        ReadyAction::ShowActive(token) => (Some(token), false),
    };
    if let Err(error) = windows.show(label) {
        if cleanup {
            registry.rollback_cleanup_reveal(label);
            return Err(LongshotIpcError::cleanup_failed(error.message));
        }
        if let Some(token) = token {
            let claimed = registry.claim_forced_termination(label, &token);
            execute_token_cleanup(claimed, cancel).await;
        } else {
            let _ = registry.claim_cancel(label, None);
        }
        windows.destroy(label);
        return Err(error);
    }
    windows.focus(label);
    Ok(())
}

pub(super) fn recover_cancel_failure_with_ops<A, P>(
    registry: &LongshotControllerRegistry,
    label: &str,
    token: &LongshotSessionToken,
    windows: &A,
    primary: CaptureError,
    probe_exact_active: P,
) -> Result<(), LongshotIpcError>
where
    A: ControlWindowActions,
    P: FnOnce() -> bool,
{
    let recovery = registry.begin_cancel_failure_recovery(label, token);
    match recovery {
        CancelFailureRecovery::Revealed => {
            let retryable = windows.exists(label) && probe_exact_active();
            if registry.complete_cancel_failure(label, token, retryable)
                == CancelFailureRecovery::Revealed
            {
                Err(primary.into())
            } else {
                Err(LongshotIpcError::cleanup_failed(primary.to_string()))
            }
        }
        CancelFailureRecovery::Hidden => {
            let retryable = windows.exists(label) && probe_exact_active();
            if !retryable || !registry.owns_hidden_cancel_reveal(label, token) {
                registry.fail_hidden_cancel_reveal(label, token);
                return Err(LongshotIpcError::cleanup_failed(primary.to_string()));
            }
            if windows.show(label).is_err() {
                registry.fail_hidden_cancel_reveal(label, token);
                return Err(LongshotIpcError::cleanup_failed(primary.to_string()));
            }
            if !registry.owns_hidden_cancel_reveal(label, token) {
                windows.destroy(label);
                registry.fail_hidden_cancel_reveal(label, token);
                return Err(LongshotIpcError::cleanup_failed(primary.to_string()));
            }
            windows.focus(label);
            if registry.complete_hidden_cancel_reveal(label, token) {
                Err(primary.into())
            } else {
                windows.destroy(label);
                registry.fail_hidden_cancel_reveal(label, token);
                Err(LongshotIpcError::cleanup_failed(primary.to_string()))
            }
        }
        CancelFailureRecovery::CleanupFailed => {
            log::error!("长截图控制窗 {label} 清理失败且状态不可安全重试: {primary}");
            Err(LongshotIpcError::cleanup_failed(primary.to_string()))
        }
    }
}

pub(super) fn complete_build_attempt<A: ControlWindowActions>(
    registry: &LongshotControllerRegistry,
    label: &str,
    build_result: Result<(), String>,
    windows: &A,
) -> Result<(), LongshotIpcError> {
    if let Err(error) = build_result {
        registry.abort_build(label);
        windows.destroy(label);
        return Err(LongshotIpcError::new(
            "longshot_controller_create_failed",
            error,
        ));
    }
    if !registry.accepts_built_window(label) {
        windows.destroy(label);
        return Err(LongshotIpcError::missing());
    }
    Ok(())
}

pub(super) fn open_with_ops<A, V, D, B>(
    registry: &LongshotControllerRegistry,
    caller_label: &str,
    selection: CaptureSelection,
    windows: &A,
    validate: V,
    arm_deadline: D,
    build: B,
) -> Result<LongshotControllerLaunch, LongshotIpcError>
where
    A: ControlWindowActions,
    V: FnOnce(&str, &CaptureSelection) -> Result<(), CaptureError>,
    D: FnOnce(&str),
    B: FnOnce(&str) -> Result<(), String>,
{
    if !caller_label.starts_with("capture-overlay-") {
        return Err(LongshotIpcError::new(
            "longshot_controller_caller_invalid",
            "只有当前截图覆盖层可以创建长截图控制窗口",
        ));
    }
    validate(caller_label, &selection)?;
    let label = registry.reserve(caller_label.to_string(), selection)?;
    arm_deadline(&label);
    let build_result = build(&label);
    complete_build_attempt(registry, &label, build_result, windows)?;
    Ok(LongshotControllerLaunch { label })
}

pub(super) fn reveal_cleanup_fallback(app: &tauri::AppHandle, label: &str) -> bool {
    let windows = TauriControlWindowActions { app };
    if !windows.exists(label) {
        log::error!("长截图清理失败且控制窗 {label} 已不存在");
        return false;
    }
    if let Err(error) = windows.show(label) {
        log::error!(
            "长截图清理失败后兜底显示控制窗 {label} 失败: {} ({})",
            error.message,
            error.code
        );
        return false;
    }
    windows.focus(label);
    true
}

/// 普通覆盖层必须收到二阶段接管结果；token/窗口标签防迟到结果解锁新的尝试。
pub(super) fn notify_handoff(
    app: &tauri::AppHandle,
    state: &AppState,
    label: &str,
    accepted: bool,
) {
    // 普通会话已消费时，失败不能伪装成能继续编辑；原覆盖层也已由lifecycle关闭。
    if let Some((caller, result)) =
        state
            .longshot_windows
            .take_handoff(label, accepted, |session_id| {
                state.capture_manager.ensure_current(session_id).is_ok()
            })
    {
        let _ = app.emit_to(caller, "capture-longshot-handoff", result);
    }
}

pub(super) fn spawn_deadline(app: tauri::AppHandle, label: String) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(LOAD_DEADLINE_MS)).await;
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let action = state.longshot_windows.claim_deadline(&label);
        let windows = TauriControlWindowActions { app: &app };
        execute_deadline_action(action, &label, &windows, |token| {
            cancel_claimed(&app, &state, &label, token)
        })
        .await;
        notify_handoff(&app, &state, &label, false);
    });
}
