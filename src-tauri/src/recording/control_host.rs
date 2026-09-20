//! 录屏控制窗的 Tauri 宿主与受 caller 约束的控制命令。
//!
//! 开始入口只接受独立 Recording 覆盖层交出的可信选区；控制命令继续只认后端绑定的 generation
//! token。默认构建和没有完成产品验收的平台不会展示入口。

use super::control_exclusion::{configure_control_exclusion, control_exclusion_capability};
use super::control_registry::{RecordingControlClose, RecordingControlRegistryError};
use super::control_window::{
    plan_control_window, ControlSize, ControlWindowPlan, PhysicalRect, WindowExclusionCapability,
};
use super::lifecycle::{DesktopActions, RecordingLifecycleError};
#[cfg(feature = "recording-vp9-prototype")]
use super::lifecycle::{RecordingStartContext, RecordingStartRequest};
use super::manager::RecordingToken;
use super::platform::RecordingSourceDescriptor;
#[cfg(feature = "recording-vp9-prototype")]
use super::segmenting::RecordingEncoder;
use crate::capture::CaptureSelection;
use crate::commands::AppState;
use serde::Serialize;
use std::time::Duration;
use tauri::{Manager, Position};

const CONTROL_PAGE: &str = "recording-control.html";
const CONTROL_SIZE: ControlSize = ControlSize {
    width: 280,
    height: 56,
};
const CONTROL_MARGIN: u32 = 12;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordingIpcError {
    code: &'static str,
    message: String,
}

impl RecordingIpcError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new("recording_internal", message)
    }

    fn unavailable() -> Self {
        Self::new(
            "recording_unavailable",
            "当前构建或桌面会话尚未开放录屏入口",
        )
    }
}

impl From<RecordingControlRegistryError> for RecordingIpcError {
    fn from(error: RecordingControlRegistryError) -> Self {
        let code = match error {
            RecordingControlRegistryError::Busy => "recording_busy",
            RecordingControlRegistryError::Missing => "recording_missing",
            RecordingControlRegistryError::Superseded => "recording_superseded",
            RecordingControlRegistryError::Poisoned => "recording_internal",
        };
        Self::new(code, error.to_string())
    }
}

impl From<RecordingLifecycleError> for RecordingIpcError {
    fn from(error: RecordingLifecycleError) -> Self {
        let code = match error {
            RecordingLifecycleError::Busy => "recording_busy",
            RecordingLifecycleError::Missing => "recording_missing",
            RecordingLifecycleError::Superseded => "recording_superseded",
            _ => "recording_failed",
        };
        Self::new(code, error.to_string())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecordingStopResult {
    output_available: bool,
    duration_ms: u64,
    captured_frames: u64,
    encoded_frames: u64,
    dropped_by_backpressure: u64,
}

pub(super) struct TauriRecordingDesktopActions<'a> {
    app: &'a tauri::AppHandle,
    state: &'a AppState,
}

impl<'a> TauriRecordingDesktopActions<'a> {
    pub(super) fn new(app: &'a tauri::AppHandle, state: &'a AppState) -> Self {
        Self { app, state }
    }

    fn rollback_prepared_control(&self, session_id: &str) {
        let Ok(close) = self.state.recording_controls.begin_close(session_id) else {
            return;
        };
        if let Some(window) = self.app.get_webview_window(&close.label) {
            let _ = window.destroy();
        }
        let _ = self
            .state
            .recording_controls
            .settle_close(&close.label, true);
    }
}

impl DesktopActions for TauriRecordingDesktopActions<'_> {
    fn close_overlays(&self, labels: &[String]) -> Result<(), String> {
        crate::capture::overlay_windows::close(self.app, labels);
        Ok(())
    }

    fn restore_pins(&self, labels: &[String]) -> Result<(), String> {
        crate::pin::restore_pins_after_capture(self.app, self.state, labels);
        Ok(())
    }

    fn restore_sources(&self, labels: &[String]) -> Result<(), String> {
        crate::capture::overlay_windows::restore(self.app, labels);
        Ok(())
    }

    fn settle_after_restore(&self) -> Result<(), String> {
        std::thread::sleep(Duration::from_millis(crate::capture::HIDE_SETTLE_MS));
        Ok(())
    }

    fn prepare_control(
        &self,
        session_id: &str,
        descriptor: &RecordingSourceDescriptor,
    ) -> Result<(), String> {
        ensure_control_positioning_supported()?;
        let label = self
            .state
            .recording_controls
            .reserve(session_id)
            .map_err(|error| error.to_string())?;
        let result = build_control_window(self.app, &label, descriptor);
        if result.is_err() {
            self.rollback_prepared_control(session_id);
        }
        result
    }

    fn bind_control(&self, token: &RecordingToken) -> Result<(), String> {
        let binding = self
            .state
            .recording_controls
            .bind(token)
            .map_err(|error| error.to_string())?;
        if binding.reveal {
            show_control_window(self.app, &binding.label)?;
        }
        Ok(())
    }

    fn close_control(&self, session_id: &str) -> Result<(), String> {
        let close = self
            .state
            .recording_controls
            .begin_close(session_id)
            .map_err(|error| error.to_string())?;
        close_control_window(self.app, &self.state.recording_controls, close)
    }
}

fn control_placement_with_capability(
    app: &tauri::AppHandle,
    descriptor: &RecordingSourceDescriptor,
    size: ControlSize,
    capability: WindowExclusionCapability,
) -> Result<PhysicalRect, String> {
    let monitors = app
        .available_monitors()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|monitor| PhysicalRect {
            x: monitor.position().x,
            y: monitor.position().y,
            width: monitor.size().width,
            height: monitor.size().height,
        })
        .collect::<Vec<_>>();
    let selection = PhysicalRect {
        x: descriptor.physical_x,
        y: descriptor.physical_y,
        width: descriptor.width,
        height: descriptor.height,
    };
    match plan_control_window(selection, &monitors, size, CONTROL_MARGIN, capability) {
        ControlWindowPlan::Visible(rect) => Ok(rect),
        ControlWindowPlan::TrayAndShortcutsOnly => {
            Err("选区外没有安全的录屏控制窗位置，托盘控制尚未实现".to_string())
        }
    }
}

fn ensure_control_positioning_supported() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if crate::platform::is_wayland() {
        return Err("Wayland 尚无可验证的录屏控制窗定位合同".to_string());
    }
    Ok(())
}

fn build_control_window(
    app: &tauri::AppHandle,
    label: &str,
    descriptor: &RecordingSourceDescriptor,
) -> Result<(), String> {
    let exclusion = control_exclusion_capability();
    let window =
        tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App(CONTROL_PAGE.into()))
            .title("")
            .inner_size(
                f64::from(CONTROL_SIZE.width),
                f64::from(CONTROL_SIZE.height),
            )
            .decorations(false)
            .resizable(false)
            .shadow(false)
            .skip_taskbar(true)
            .always_on_top(true)
            .focused(false)
            .visible(false)
            .build()
            .map_err(|error| error.to_string())?;
    let configure = (|| {
        configure_control_exclusion(&window, exclusion)?;
        let actual = window.outer_size().map_err(|error| error.to_string())?;
        let placement = control_placement_with_capability(
            app,
            descriptor,
            ControlSize {
                width: actual.width,
                height: actual.height,
            },
            exclusion,
        )?;
        window
            .set_position(Position::Physical(tauri::PhysicalPosition::new(
                placement.x,
                placement.y,
            )))
            .map_err(|error| error.to_string())
    })();
    if let Err(error) = configure {
        let _ = window.destroy();
        return Err(error);
    }
    Ok(())
}

fn show_control_window(app: &tauri::AppHandle, label: &str) -> Result<(), String> {
    app.get_webview_window(label)
        .ok_or_else(|| "录屏控制窗不存在".to_string())?
        .show()
        .map_err(|error| error.to_string())
}

fn close_control_window(
    app: &tauri::AppHandle,
    registry: &super::control_registry::RecordingControlRegistry,
    close: RecordingControlClose,
) -> Result<(), String> {
    let destroyed = app
        .get_webview_window(&close.label)
        .map_or(Ok(()), |window| {
            window.destroy().map_err(|error| error.to_string())
        });
    registry
        .settle_close(&close.label, destroyed.is_ok())
        .map_err(|error| error.to_string())?;
    destroyed
}

fn token_for_caller(
    state: &AppState,
    caller_label: &str,
) -> Result<RecordingToken, RecordingIpcError> {
    state
        .recording_controls
        .token_for_caller(caller_label)
        .map_err(RecordingIpcError::from)
}

/// 只有独立的 Recording 选区覆盖层能调用。前端只提交同一冻结会话中的逻辑选区；会话身份、物理
/// crop、帧源、帧率、光标策略和编码器都由后端核验或固定，不能从 IPC 注入。
#[tauri::command]
pub(crate) async fn start_capture_recording(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    selection: CaptureSelection,
) -> Result<(), RecordingIpcError> {
    if !super::product_entry_available() {
        return Err(RecordingIpcError::unavailable());
    }

    #[cfg(not(feature = "recording-vp9-prototype"))]
    {
        let _ = (window, app, state, selection);
        Err(RecordingIpcError::unavailable())
    }

    #[cfg(feature = "recording-vp9-prototype")]
    {
        let caller = window.label().to_string();
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|error| RecordingIpcError::internal(error.to_string()))?;
        let lifecycle = state.recording_lifecycle.clone();
        let session_id = format!("recording-{}", crate::image_io::unique_image_id());
        tauri::async_runtime::spawn_blocking(move || {
            let state = app
                .try_state::<AppState>()
                .ok_or_else(|| RecordingIpcError::internal("AppState 已不可用"))?;
            lifecycle.start_platform(
                RecordingStartContext {
                    capture: &state.capture_manager,
                    caller: &caller,
                    selection: &selection,
                    app_data_dir: &app_data_dir,
                },
                RecordingStartRequest {
                    session_id,
                    frames_per_second: 30,
                    include_cursor: true,
                    encoder: RecordingEncoder::Vp9Prototype,
                },
                &TauriRecordingDesktopActions::new(&app, &state),
            )?;
            Ok(())
        })
        .await
        .map_err(|error| RecordingIpcError::internal(error.to_string()))?
    }
}

#[tauri::command]
pub(crate) async fn mark_recording_control_ready(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), RecordingIpcError> {
    let label = window.label().to_string();
    let reveal = state.recording_controls.mark_ready(&label)?;
    if !reveal {
        return Ok(());
    }
    if let Err(error) = show_control_window(&app, &label) {
        let token = token_for_caller(&state, &label)?;
        let lifecycle = state.recording_lifecycle.clone();
        let cleanup_app = app.clone();
        let cleanup = tauri::async_runtime::spawn_blocking(move || {
            let cleanup_state = cleanup_app
                .try_state::<AppState>()
                .ok_or_else(|| "AppState 已不可用".to_string())?;
            lifecycle
                .cancel(
                    &token,
                    &TauriRecordingDesktopActions::new(&cleanup_app, &cleanup_state),
                )
                .map_err(|failure| failure.to_string())
        })
        .await;
        if !matches!(cleanup, Ok(Ok(()))) {
            log::error!("录屏控制窗显示失败后的会话清理也失败: {cleanup:?}");
        }
        return Err(RecordingIpcError::new(
            "recording_control_show_failed",
            error,
        ));
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn pause_recording(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<(), RecordingIpcError> {
    let token = token_for_caller(&state, window.label())?;
    state.recording_lifecycle.pause(&token)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn resume_recording(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<(), RecordingIpcError> {
    let token = token_for_caller(&state, window.label())?;
    state.recording_lifecycle.resume(&token)?;
    Ok(())
}

#[tauri::command]
pub(crate) async fn stop_recording(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<RecordingStopResult, RecordingIpcError> {
    let token = token_for_caller(&state, window.label())?;
    let lifecycle = state.recording_lifecycle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app
            .try_state::<AppState>()
            .ok_or_else(|| RecordingIpcError::internal("AppState 已不可用"))?;
        let report = lifecycle.stop(&token, &TauriRecordingDesktopActions::new(&app, &state))?;
        let output_available = report.final_output_path.is_some();
        if let Err(error) = super::library::open(&app) {
            log::warn!("录屏停止后打开结果库失败: {error}");
        }
        Ok(RecordingStopResult {
            output_available,
            duration_ms: report.duration_ns / 1_000_000,
            captured_frames: report.captured_frames,
            encoded_frames: report.encoded_frames,
            dropped_by_backpressure: report.dropped_by_backpressure,
        })
    })
    .await
    .map_err(|error| RecordingIpcError::internal(error.to_string()))?
}

#[tauri::command]
pub(crate) async fn cancel_recording(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), RecordingIpcError> {
    let token = token_for_caller(&state, window.label())?;
    let lifecycle = state.recording_lifecycle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app
            .try_state::<AppState>()
            .ok_or_else(|| RecordingIpcError::internal("AppState 已不可用"))?;
        lifecycle
            .cancel(&token, &TauriRecordingDesktopActions::new(&app, &state))
            .map_err(RecordingIpcError::from)
    })
    .await
    .map_err(|error| RecordingIpcError::internal(error.to_string()))?
}

pub(crate) fn handle_control_destroyed(app: &tauri::AppHandle, label: &str) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    match state.recording_controls.token_for_caller(label) {
        Ok(token) => {
            let lifecycle = state.recording_lifecycle.clone();
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let Some(state) = app.try_state::<AppState>() else {
                    return;
                };
                if let Err(error) =
                    lifecycle.cancel(&token, &TauriRecordingDesktopActions::new(&app, &state))
                {
                    log::warn!("录屏控制窗意外销毁后取消会话失败: {error}");
                }
            });
        }
        Err(RecordingControlRegistryError::Busy) => {
            if let Ok(close) = state.recording_controls.claim_destroyed(label) {
                let _ = state.recording_controls.settle_close(&close.label, true);
            }
        }
        Err(RecordingControlRegistryError::Missing | RecordingControlRegistryError::Superseded) => {
        }
        Err(RecordingControlRegistryError::Poisoned) => {
            log::error!("录屏控制窗销毁时 registry 锁已损坏");
        }
    }
}
