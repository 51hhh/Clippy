//! 独立长截图控制窗的两阶段接管与补偿状态机。

mod finish;
mod lifecycle;
mod output;
mod registry;
mod workers;

use super::LongshotSessionToken;
use crate::capture::manager::LongshotGuideSpec;
use crate::capture::types::OverlaySpec;
use crate::capture::{CaptureError, CaptureSelection};
use crate::commands::AppState;
use finish::{
    execute_finish_with_ops, run_finish_worker, run_output_worker, FinishOperations, OutputValue,
    OutputWorkerError,
};
#[cfg(test)]
use finish::{FinishBoundary, FinishClaim, FinishWorkerError, OutputFailureAction};
#[cfg(test)]
use lifecycle::{complete_build_attempt, execute_deadline_action, execute_deadline_window_action};
use lifecycle::{
    emergency_decision, execute_cancel_action, execute_ready_action, execute_token_cleanup,
    notify_handoff, open_with_ops, recover_cancel_failure_with_ops, reveal_cleanup_fallback,
    spawn_deadline, ControlWindowActions, TauriControlWindowActions,
};
use output::{copy_longshot_artifact, pin_longshot_artifact};
use std::sync::Arc;
use std::time::Duration;
use tauri::Manager;
#[cfg(any(test, all(target_os = "linux", feature = "longshot-wayland-auto")))]
use workers::execute_visible_operation_with_ops;
use workers::{
    execute_append_with_ops, execute_preview_with_ops, execute_visible_mutation,
    run_activation_worker, run_append_worker, run_preview_worker,
};
#[cfg(test)]
use workers::{AppendBoundary, VisibleOperationBoundary};

const CONTROLLER_PREFIX: &str = "longshot-controller-";
const CONTROLLER_PAGE: &str = "/longshot-controller.html";
const GUIDE_PREFIX: &str = "longshot-guide-";
const LOAD_DEADLINE_MS: u64 = 5_000;

fn guide_label(controller_label: &str) -> Option<String> {
    let suffix = controller_label.strip_prefix(CONTROLLER_PREFIX)?;
    if suffix.is_empty()
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return None;
    }
    Some(format!("{GUIDE_PREFIX}{suffix}"))
}

fn build_guide_window(
    app: &tauri::AppHandle,
    controller_label: &str,
    guide: LongshotGuideSpec,
) -> Result<(), String> {
    let label = guide_label(controller_label).ok_or_else(|| "长截图控制窗标签无效".to_string())?;
    let url = format!(
        "longshot-guide.html?x={}&y={}&width={}&height={}",
        guide.selection_x, guide.selection_y, guide.selection_width, guide.selection_height
    );
    let builder = tauri::WebviewWindowBuilder::new(app, &label, tauri::WebviewUrl::App(url.into()))
        .title("")
        .position(guide.monitor_x as f64, guide.monitor_y as f64)
        .inner_size(guide.monitor_width as f64, guide.monitor_height as f64)
        .decorations(false)
        .transparent(true)
        .background_color(tauri::window::Color(0, 0, 0, 0))
        .shadow(false)
        .resizable(false)
        .skip_taskbar(true)
        .focusable(false)
        .focused(false)
        .visible(false);
    // 与截图覆盖层保持同一平台策略：Wayland 不支持客户端声明全局置顶，
    // 目标显示器全屏已足以避免依赖全局坐标；X11/Windows/macOS 仍请求置顶。
    #[cfg(target_os = "linux")]
    let builder = if crate::platform::is_wayland() {
        builder
    } else {
        builder.always_on_top(true)
    };
    #[cfg(not(target_os = "linux"))]
    let builder = builder.always_on_top(true);
    let window = builder.build().map_err(|error| error.to_string())?;
    if let Err(error) = window.set_ignore_cursor_events(true) {
        let _ = window.destroy();
        return Err(error.to_string());
    }
    let overlay = OverlaySpec {
        label,
        x: guide.monitor_x,
        y: guide.monitor_y,
        width: guide.monitor_width,
        height: guide.monitor_height,
    };
    if let Err(error) =
        crate::capture::overlay_windows::configure_platform_overlay(&window, &overlay)
    {
        let _ = window.destroy();
        return Err(error.to_string());
    }
    Ok(())
}

mod model;

#[cfg(test)]
use model::{CancelAction, DeadlineAction, ReadyAction, Slot, TerminationOrigin};
use model::{EmergencyDecision, LongshotOutputArtifact};
pub(crate) use model::{
    LongshotActivation, LongshotAutoCapability, LongshotControllerHandle, LongshotControllerLaunch,
    LongshotControllerRegistry, LongshotIpcError, LongshotOutputAction, LongshotOutputResult,
    LongshotSnapshotDto,
};

#[cfg(all(target_os = "linux", feature = "longshot-wayland-auto"))]
fn wayland_parent_identifier(
    window: &tauri::WebviewWindow,
) -> Result<ashpd::WindowIdentifier, LongshotIpcError> {
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

    if !crate::platform::is_wayland() {
        return Err(CaptureError::LongshotAutoUnsupported.into());
    }
    let window_handle = window
        .window_handle()
        .map_err(|error| LongshotIpcError::internal(error.to_string()))?;
    let display_handle = window
        .display_handle()
        .map_err(|error| LongshotIpcError::internal(error.to_string()))?;
    let raw_window = window_handle.as_raw();
    let raw_display = display_handle.as_raw();
    if !matches!(
        (&raw_window, &raw_display),
        (
            raw_window_handle::RawWindowHandle::Wayland(_),
            raw_window_handle::RawDisplayHandle::Wayland(_)
        )
    ) {
        return Err(CaptureError::LongshotAutoUnsupported.into());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| LongshotIpcError::internal(error.to_string()))?;
    runtime
        .block_on(ashpd::WindowIdentifier::from_raw_handle(
            &raw_window,
            Some(&raw_display),
        ))
        .ok_or_else(|| LongshotIpcError::internal("Wayland 合成器无法导出长截图授权窗标识"))
}

pub(crate) fn open(
    app: &tauri::AppHandle,
    state: &AppState,
    caller_label: &str,
    selection: CaptureSelection,
) -> Result<LongshotControllerLaunch, LongshotIpcError> {
    let registry = state.longshot_windows.clone();
    let guide_selection = selection.clone();
    open_with_ops(
        &state.longshot_windows,
        caller_label,
        selection,
        &TauriControlWindowActions { app },
        |label, selection| {
            state
                .capture_manager
                .validate_longshot_open(label, selection)
        },
        |label| spawn_deadline(app.clone(), label.to_string()),
        |label| {
            let guide = state
                .capture_manager
                .longshot_guide_spec(caller_label, &guide_selection)
                .map_err(|error| error.to_string())?;
            build_guide_window(app, label, guide)?;
            let callback_label = label.to_string();
            let callback_registry = registry.clone();
            tauri::WebviewWindowBuilder::new(
                app,
                label,
                tauri::WebviewUrl::App("longshot-controller.html".into()),
            )
            .title("")
            .inner_size(400.0, 560.0)
            .decorations(false)
            .resizable(false)
            .skip_taskbar(true)
            .always_on_top(true)
            .focused(false)
            .visible(false)
            .center()
            .on_page_load(move |window, payload| {
                if payload.event() == tauri::webview::PageLoadEvent::Started
                    && window.label() == callback_label
                {
                    callback_registry.publish_started(&callback_label, payload.url().path());
                }
            })
            .build()
            .map(|_| ())
            .map_err(|error| error.to_string())
        },
    )
}

pub(crate) async fn activate(
    app: tauri::AppHandle,
    state: &AppState,
    caller_label: &str,
) -> Result<LongshotActivation, LongshotIpcError> {
    let result = activate_inner(app.clone(), state, caller_label).await;
    notify_handoff(&app, state, caller_label, result.is_ok());
    result
}

async fn activate_inner(
    app: tauri::AppHandle,
    state: &AppState,
    caller_label: &str,
) -> Result<LongshotActivation, LongshotIpcError> {
    let selection = state.longshot_windows.claim_activation(caller_label)?;
    let lifecycle = state.longshot_lifecycle.clone();
    let capture = state.capture_manager.clone();
    let worker_app = app.clone();
    let result = match run_activation_worker(&state.longshot_windows, caller_label, move || {
        lifecycle.begin(&capture, &selection, &worker_app)
    })
    .await
    {
        Ok(result) => result,
        Err(failure) => {
            if !failure.cleanup_recorded && reveal_cleanup_fallback(&app, caller_label) {
                state.longshot_windows.mark_cleanup_revealed(caller_label);
            }
            return Err(failure.error);
        }
    };
    let started_token = result.as_ref().ok().map(|start| start.token.clone());
    let (activation, compensation) = match state
        .longshot_windows
        .complete_activation(caller_label, result)
    {
        Ok(completed) => completed,
        Err(error) => {
            // begin 已经成功时，registry 提交失败也不能遗失唯一 token。
            let _ = state
                .longshot_windows
                .mark_cleanup_failed(caller_label, started_token.clone());
            let decision = if let Some(token) = started_token {
                let lifecycle = state.longshot_lifecycle.clone();
                let cleanup_app = app.clone();
                let cleanup_token = token.clone();
                let cleanup = tauri::async_runtime::spawn_blocking(move || {
                    let cleanup_state = cleanup_app
                        .try_state::<AppState>()
                        .ok_or_else(|| CaptureError::StateLock("AppState 已不可用".to_string()))?;
                    lifecycle.cancel(&cleanup_token, &cleanup_app, &cleanup_state)
                })
                .await;
                let cleanup_succeeded = matches!(cleanup, Ok(Ok(())));
                let settled = state.longshot_windows.settle_emergency_cleanup(
                    caller_label,
                    &token,
                    cleanup_succeeded,
                );
                if !cleanup_succeeded {
                    log::error!("长截图启动完成但 registry 提交失败，补偿清理也失败");
                }
                if !settled {
                    log::error!("长截图补偿后无法确认控制窗 registry 已收敛，只能等待进程重启");
                }
                emergency_decision(cleanup_succeeded, settled)
            } else {
                EmergencyDecision::Reveal
            };
            match decision {
                EmergencyDecision::Destroy => {
                    TauriControlWindowActions { app: &app }.destroy(caller_label)
                }
                EmergencyDecision::AwaitReady => {}
                EmergencyDecision::Reveal => {
                    if reveal_cleanup_fallback(&app, caller_label) {
                        state.longshot_windows.mark_cleanup_revealed(caller_label);
                    }
                }
            }
            return Err(LongshotIpcError::cleanup_failed(error.message));
        }
    };
    if let Some(token) = compensation {
        execute_token_cleanup(Some(token), |token| {
            cancel_claimed(&app, state, caller_label, token)
        })
        .await;
        return Err(LongshotIpcError::missing());
    }
    activation.ok_or_else(LongshotIpcError::missing)
}

pub(crate) async fn ready(
    app: tauri::AppHandle,
    state: &AppState,
    caller_label: &str,
) -> Result<(), LongshotIpcError> {
    let action = state.longshot_windows.claim_ready(caller_label)?;
    execute_ready_action(
        &state.longshot_windows,
        action,
        caller_label,
        &TauriControlWindowActions { app: &app },
        |token| cancel_claimed(&app, state, caller_label, token),
    )
    .await
}

pub(crate) async fn append(
    app: tauri::AppHandle,
    state: &AppState,
    caller_label: &str,
    handle: LongshotControllerHandle,
) -> Result<LongshotSnapshotDto, LongshotIpcError> {
    let longshot_lifecycle = state.longshot_lifecycle.clone();
    execute_append_with_ops(
        &state.longshot_windows,
        caller_label,
        &handle,
        &TauriControlWindowActions { app: &app },
        || async {
            tokio::time::sleep(Duration::from_millis(crate::capture::HIDE_SETTLE_MS)).await;
        },
        move |token| {
            let longshot_lifecycle = longshot_lifecycle.clone();
            async move {
                run_append_worker(move || {
                    let outcome = longshot_lifecycle.append(&token)?;
                    Ok(outcome.snapshot)
                })
                .await
            }
        },
        |token| cancel_claimed(&app, state, caller_label, token),
        |_| {},
    )
    .await
}

pub(crate) async fn auto_append(
    app: tauri::AppHandle,
    state: &AppState,
    caller_label: &str,
    handle: LongshotControllerHandle,
    direction: crate::capture::longshot::LongshotAutoDirection,
) -> Result<LongshotSnapshotDto, LongshotIpcError> {
    let longshot_lifecycle = state.longshot_lifecycle.clone();
    execute_append_with_ops(
        &state.longshot_windows,
        caller_label,
        &handle,
        &TauriControlWindowActions { app: &app },
        || async {
            tokio::time::sleep(Duration::from_millis(crate::capture::HIDE_SETTLE_MS)).await;
        },
        move |token| {
            let longshot_lifecycle = longshot_lifecycle.clone();
            async move {
                run_append_worker(move || {
                    let outcome = longshot_lifecycle.auto_append(&token, direction)?;
                    Ok(outcome.snapshot)
                })
                .await
            }
        },
        |token| cancel_claimed(&app, state, caller_label, token),
        |_| {},
    )
    .await
}

pub(crate) async fn authorize_auto(
    state: &AppState,
    window: &tauri::WebviewWindow,
    handle: LongshotControllerHandle,
) -> Result<LongshotAutoCapability, LongshotIpcError> {
    #[cfg(not(all(target_os = "linux", feature = "longshot-wayland-auto")))]
    {
        let _ = (state, window, handle);
        Err(CaptureError::LongshotAutoUnsupported.into())
    }
    #[cfg(all(target_os = "linux", feature = "longshot-wayland-auto"))]
    {
        let lifecycle = state.longshot_lifecycle.clone();
        let parent_window = window.clone();
        let authorized = execute_visible_operation_with_ops(
            &state.longshot_windows,
            window.label(),
            &handle,
            move |token| async move {
                let parent = tauri::async_runtime::spawn_blocking(move || {
                    wayland_parent_identifier(&parent_window)
                })
                .await
                .map_err(|error| {
                    LongshotIpcError::internal(format!("Wayland 父窗口线程异常: {error}"))
                })??;
                match tauri::async_runtime::spawn_blocking(move || {
                    lifecycle.authorize_wayland_auto(&token, parent)?;
                    lifecycle.wayland_auto_authorized(&token)
                })
                .await
                {
                    Ok(Err(CaptureError::LongshotAutoPermissionRequired)) => {
                        Err(LongshotIpcError::new(
                            "longshot_auto_wayland_permission_required",
                            "Wayland Portal 未授予本次长截图的指针控制权限",
                        ))
                    }
                    Ok(result) => result.map_err(LongshotIpcError::from),
                    Err(error) => Err(LongshotIpcError::internal(format!(
                        "Wayland 授权线程异常: {error}"
                    ))),
                }
            },
            |_| {},
        )
        .await?;
        Ok(LongshotAutoCapability::current_with_wayland_authorized(
            authorized,
        ))
    }
}

pub(crate) async fn undo(
    state: &AppState,
    caller_label: &str,
    handle: LongshotControllerHandle,
) -> Result<LongshotSnapshotDto, LongshotIpcError> {
    let lifecycle = state.longshot_lifecycle.clone();
    execute_visible_mutation(
        &state.longshot_windows,
        caller_label,
        &handle,
        move |token| {
            let lifecycle = lifecycle.clone();
            async move { run_append_worker(move || lifecycle.undo(&token)).await }
        },
    )
    .await
}

pub(crate) async fn preview(
    state: &AppState,
    caller_label: &str,
    handle: LongshotControllerHandle,
) -> Result<tauri::ipc::Response, LongshotIpcError> {
    let lifecycle = state.longshot_lifecycle.clone();
    let png =
        execute_preview_with_ops(
            &state.longshot_windows,
            caller_label,
            &handle,
            move |token| async move {
                run_preview_worker(move || lifecycle.preview_tail_png(&token)).await
            },
            || {},
        )
        .await?;
    Ok(tauri::ipc::Response::new(png))
}

pub(crate) async fn finish(
    app: tauri::AppHandle,
    state: &AppState,
    caller_label: &str,
    handle: LongshotControllerHandle,
    action: LongshotOutputAction,
) -> Result<LongshotOutputResult, LongshotIpcError> {
    let lifecycle = state.longshot_lifecycle.clone();
    let finish_app = app.clone();
    let save_target = state.save_target();
    let pin_origins = Arc::clone(&state.pin_origins);
    let output_app = app.clone();
    execute_finish_with_ops(
        &state.longshot_windows,
        caller_label,
        &handle,
        action,
        &TauriControlWindowActions { app: &app },
        FinishOperations::new(
            move |token| {
                let lifecycle = lifecycle.clone();
                async move {
                    run_finish_worker(move || {
                        let state = finish_app.try_state::<AppState>().ok_or_else(|| {
                            CaptureError::StateLock("AppState 已不可用".to_string())
                        })?;
                        lifecycle.finish_png(&token, &finish_app, &state)
                    })
                    .await
                }
            },
            |token: &LongshotSessionToken| {
                state
                    .longshot_lifecycle
                    .is_exact_active(token)
                    .unwrap_or(false)
            },
            move |requested_action, artifact: Arc<LongshotOutputArtifact>| {
                let output_app = output_app.clone();
                async move {
                    match requested_action {
                        LongshotOutputAction::Copy => {
                            run_output_worker(requested_action, move || {
                                copy_longshot_artifact(&artifact, &pin_origins, |image| {
                                    crate::clipboard_watcher::clipboard_set_image_with_retry(image)
                                })?;
                                Ok(OutputValue::None)
                            })
                            .await
                        }
                        LongshotOutputAction::Save => {
                            run_output_worker(requested_action, move || {
                                let path = crate::image_io::save_png(
                                    artifact.png.as_slice(),
                                    "clippy-screenshot",
                                    &save_target,
                                )?;
                                Ok(OutputValue::SavePath(path.to_string_lossy().into_owned()))
                            })
                            .await
                        }
                        LongshotOutputAction::Pin => {
                            let state = output_app.try_state::<AppState>().ok_or_else(|| {
                                OutputWorkerError::Business("AppState 已不可用".to_string())
                            })?;
                            pin_longshot_artifact(&artifact, |png, origin| {
                                crate::pin::commands::create_screenshot_pin_shared(
                                    png,
                                    Some(origin),
                                    &output_app,
                                    &state,
                                )
                            })
                            .map(OutputValue::PinLabel)
                        }
                    }
                }
            },
            |token| cancel_claimed(&app, state, caller_label, token),
            |_| {},
        ),
    )
    .await
}

pub(crate) async fn cancel(
    app: tauri::AppHandle,
    state: &AppState,
    caller_label: &str,
    handle: Option<LongshotControllerHandle>,
) -> Result<(), LongshotIpcError> {
    let action = state
        .longshot_windows
        .claim_cancel(caller_label, handle.as_ref())?;
    let windows = TauriControlWindowActions { app: &app };
    let result = execute_cancel_action(action, caller_label, &windows, |token| {
        cancel_claimed(&app, state, caller_label, token)
    })
    .await;
    notify_handoff(&app, state, caller_label, false);
    result
}

async fn cancel_claimed(
    app: &tauri::AppHandle,
    state: &AppState,
    label: &str,
    token: LongshotSessionToken,
) -> Result<(), LongshotIpcError> {
    let lifecycle = state.longshot_lifecycle.clone();
    let worker_app = app.clone();
    let worker_token = token.clone();
    let result = match tauri::async_runtime::spawn_blocking(move || {
        let state = worker_app
            .try_state::<AppState>()
            .ok_or_else(|| CaptureError::StateLock("AppState 已不可用".to_string()))?;
        lifecycle.cancel(&worker_token, &worker_app, &state)
    })
    .await
    {
        Ok(result) => result,
        Err(error) => {
            let _ = state
                .longshot_windows
                .mark_cleanup_failed(label, Some(token));
            return Err(LongshotIpcError::cleanup_failed(format!(
                "长截图取消线程异常: {error}"
            )));
        }
    };
    match result {
        Ok(()) => {
            state
                .longshot_windows
                .complete_cancel_success(label, &token);
            Ok(())
        }
        Err(primary) => {
            let windows = TauriControlWindowActions { app };
            recover_cancel_failure_with_ops(
                &state.longshot_windows,
                label,
                &token,
                &windows,
                primary,
                || {
                    state
                        .longshot_lifecycle
                        .is_exact_active(&token)
                        .unwrap_or(false)
                },
            )
        }
    }
}

pub(crate) fn handle_controller_destroyed(app: &tauri::AppHandle, label: &str) {
    if let Some(guide_label) = guide_label(label) {
        if let Some(guide) = app.get_webview_window(&guide_label) {
            let _ = guide.destroy();
        }
    }
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let token = state.longshot_windows.claim_destroyed(label);
    notify_handoff(app, &state, label, false);
    if token.is_none() {
        return;
    }
    let app = app.clone();
    let label = label.to_string();
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        execute_token_cleanup(token, |token| cancel_claimed(&app, &state, &label, token)).await;
    });
}

#[cfg(test)]
mod tests;
