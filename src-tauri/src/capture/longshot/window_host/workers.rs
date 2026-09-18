//! activation、append 与 preview 的 blocking worker 及可测试编排。

use super::lifecycle::ControlWindowActions;
use super::model::{
    LongshotControllerHandle, LongshotControllerRegistry, LongshotIpcError, LongshotSnapshotDto,
};
use crate::capture::longshot::{LongshotSessionToken, LongshotSnapshot, LongshotStart};
use crate::capture::CaptureError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AppendBoundary {
    AfterClaim,
    AfterHide,
    AfterSettle,
    AfterWorker,
    AfterShow,
    BeforeCommit,
}

pub(super) async fn terminate_hidden_append<A, C, F>(
    registry: &LongshotControllerRegistry,
    label: &str,
    token: &LongshotSessionToken,
    windows: &A,
    cancel: C,
    visibility_error: LongshotIpcError,
) -> LongshotIpcError
where
    A: ControlWindowActions,
    C: FnOnce(LongshotSessionToken) -> F,
    F: std::future::Future<Output = Result<(), LongshotIpcError>>,
{
    let claimed = registry.claim_append_visibility_failure(label, token);
    let cleanup = if let Some(claimed) = claimed {
        cancel(claimed).await
    } else {
        return LongshotIpcError::superseded();
    };
    windows.destroy(label);
    match cleanup {
        Ok(()) => visibility_error,
        Err(error) => {
            let _ = registry.settle_termination_cleanup_failed(label, token);
            LongshotIpcError::cleanup_failed(error.message)
        }
    }
}

// 编排 seam 显式注入窗口、等待、worker、cleanup 与边界钩子，避免测试绕开生产路径。
#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_append_with_ops<A, S, SF, W, WF, C, CF, H>(
    registry: &LongshotControllerRegistry,
    label: &str,
    handle: &LongshotControllerHandle,
    windows: &A,
    settle: S,
    worker: W,
    cancel: C,
    mut boundary: H,
) -> Result<LongshotSnapshotDto, LongshotIpcError>
where
    A: ControlWindowActions,
    S: FnOnce() -> SF,
    SF: std::future::Future<Output = ()>,
    W: FnOnce(LongshotSessionToken) -> WF,
    WF: std::future::Future<Output = Result<LongshotSnapshot, LongshotIpcError>>,
    C: FnOnce(LongshotSessionToken) -> CF,
    CF: std::future::Future<Output = Result<(), LongshotIpcError>>,
    H: FnMut(AppendBoundary),
{
    let claim = registry.claim_append(label, handle)?;
    boundary(AppendBoundary::AfterClaim);
    if !registry.owns_append(label, &claim.token) {
        return Err(LongshotIpcError::superseded());
    }
    if let Err(hide_error) = windows.hide(label) {
        if !registry.owns_append(label, &claim.token) {
            return Err(LongshotIpcError::superseded());
        }
        if let Err(show_error) = windows.show(label) {
            return Err(terminate_hidden_append(
                registry,
                label,
                &claim.token,
                windows,
                cancel,
                show_error,
            )
            .await);
        }
        if !registry.owns_append(label, &claim.token) {
            windows.destroy(label);
            return Err(LongshotIpcError::superseded());
        }
        windows.focus(label);
        if let Err(error) =
            registry.complete_append_visible(label, &claim.token, claim.old_snapshot)
        {
            windows.destroy(label);
            return Err(error);
        }
        return Err(LongshotIpcError::new(
            "longshot_controller_hide_failed",
            hide_error.message,
        ));
    }

    boundary(AppendBoundary::AfterHide);
    if !registry.owns_append(label, &claim.token) {
        return Err(LongshotIpcError::superseded());
    }
    settle().await;
    boundary(AppendBoundary::AfterSettle);
    if !registry.owns_append(label, &claim.token) {
        return Err(LongshotIpcError::superseded());
    }

    let worker_result = worker(claim.token.clone()).await;
    boundary(AppendBoundary::AfterWorker);
    if !registry.owns_append(label, &claim.token) {
        return Err(LongshotIpcError::superseded());
    }
    if let Err(show_error) = windows.show(label) {
        return Err(terminate_hidden_append(
            registry,
            label,
            &claim.token,
            windows,
            cancel,
            show_error,
        )
        .await);
    }
    boundary(AppendBoundary::AfterShow);
    if !registry.owns_append(label, &claim.token) {
        // show 与 ownership 核验之间若被取消，强销毁可能被迟到 show 暴露的旧窗口。
        windows.destroy(label);
        return Err(LongshotIpcError::superseded());
    }
    windows.focus(label);
    boundary(AppendBoundary::BeforeCommit);
    if !registry.owns_append(label, &claim.token) {
        windows.destroy(label);
        return Err(LongshotIpcError::superseded());
    }

    match worker_result {
        Ok(snapshot) => registry.complete_append_visible(label, &claim.token, snapshot),
        Err(error) => {
            registry.complete_append_visible(label, &claim.token, claim.old_snapshot)?;
            Err(error)
        }
    }
}

pub(super) async fn run_append_worker<F>(work: F) -> Result<LongshotSnapshot, LongshotIpcError>
where
    F: FnOnce() -> Result<LongshotSnapshot, CaptureError> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(work).await {
        Ok(Ok(snapshot)) => Ok(snapshot),
        Ok(Err(error)) => Err(error.into()),
        Err(error) => Err(LongshotIpcError::internal(format!(
            "长截图追加线程异常: {error}"
        ))),
    }
}

/// 对当前可见控制窗执行不需要重捕获的会话变更，例如显式撤销。
pub(super) async fn execute_visible_mutation<W, WFut>(
    registry: &LongshotControllerRegistry,
    label: &str,
    handle: &LongshotControllerHandle,
    worker: W,
) -> Result<LongshotSnapshotDto, LongshotIpcError>
where
    W: FnOnce(LongshotSessionToken) -> WFut,
    WFut: std::future::Future<Output = Result<LongshotSnapshot, LongshotIpcError>>,
{
    let claim = registry.claim_append(label, handle)?;
    if !registry.owns_append(label, &claim.token) {
        return Err(LongshotIpcError::superseded());
    }
    let result = worker(claim.token.clone()).await;
    if !registry.owns_append(label, &claim.token) {
        return Err(LongshotIpcError::superseded());
    }
    match result {
        Ok(snapshot) => registry.complete_append_visible(label, &claim.token, snapshot),
        Err(error) => {
            registry.complete_append_visible(label, &claim.token, claim.old_snapshot)?;
            Err(error)
        }
    }
}

pub(super) async fn run_preview_worker<F>(work: F) -> Result<Vec<u8>, LongshotIpcError>
where
    F: FnOnce() -> Result<Vec<u8>, CaptureError> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(work).await {
        Ok(Ok(png)) => Ok(png),
        Ok(Err(error)) => Err(error.into()),
        Err(error) => Err(LongshotIpcError::internal(format!(
            "长截图预览线程异常: {error}"
        ))),
    }
}

pub(super) async fn execute_preview_with_ops<W, WFut, H>(
    registry: &LongshotControllerRegistry,
    label: &str,
    handle: &LongshotControllerHandle,
    worker: W,
    after_worker: H,
) -> Result<Vec<u8>, LongshotIpcError>
where
    W: FnOnce(LongshotSessionToken) -> WFut,
    WFut: std::future::Future<Output = Result<Vec<u8>, LongshotIpcError>>,
    H: FnOnce(),
{
    let token = registry.authorize_preview(label, handle)?;
    let worker_result = worker(token.clone()).await;
    after_worker();
    if !registry.confirms_preview(label, &token)? {
        return Err(LongshotIpcError::superseded());
    }
    worker_result
}

pub(super) struct ActivationWorkerJoinFailure {
    pub(super) error: LongshotIpcError,
    pub(super) cleanup_recorded: bool,
}

pub(super) async fn run_activation_worker<F>(
    registry: &LongshotControllerRegistry,
    label: &str,
    work: F,
) -> Result<Result<LongshotStart, CaptureError>, ActivationWorkerJoinFailure>
where
    F: FnOnce() -> Result<LongshotStart, CaptureError> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(work).await {
        Ok(result) => Ok(result),
        Err(error) => {
            let cleanup_recorded = registry.mark_cleanup_failed(label, None);
            Err(ActivationWorkerJoinFailure {
                error: LongshotIpcError::cleanup_failed(format!("长截图启动线程异常: {error}")),
                cleanup_recorded,
            })
        }
    }
}
