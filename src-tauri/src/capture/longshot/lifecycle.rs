//! 长截图像素会话与桌面清理资源之间的唯一进程级协调器。
//!
//! 状态锁只移动轻量 token、资源清单与模式所有权。重捕获、编码、窗口动作和 gate
//! 释放都在锁外执行，避免桌面调用反向阻塞会话状态机。

use super::controller::{LongshotController, LongshotControllerFinish, LongshotControllerStart};
use super::LongshotSnapshot;
use super::{
    LongshotAppendOutcome, LongshotArtifact, LongshotAutoDirection, LongshotSessionToken,
    LongshotStart,
};
use crate::capture::manager::OrdinaryCaptureResources;
use crate::capture::{CaptureError, CaptureManager, CaptureModeOwnership, CaptureSelection};
use crate::commands::AppState;
use std::sync::Mutex;

pub(crate) struct LongshotLifecycle {
    controller: LongshotController,
    slot: Mutex<LifecycleSlot>,
}

#[derive(Debug)]
enum LifecycleSlot {
    Empty,
    Starting,
    Publishing(SessionResources),
    Active(SessionResources),
    Terminating(SessionResources),
    Restoring(RestoringSession),
    Releasing(LongshotSessionToken),
    TerminalFailed,
}

#[derive(Debug)]
struct SessionResources {
    token: LongshotSessionToken,
    resources: OrdinaryCaptureResources,
}

#[derive(Debug)]
struct RestoringSession {
    session: SessionResources,
    ownership: CaptureModeOwnership,
}

/// 现有桌面 API 均为 best-effort `()`；该 seam 只固定责任、参数与调用顺序。
trait DesktopActions {
    fn close_overlays(&self, labels: &[String]);
    fn restore_pins(&self, labels: &[String]);
    fn restore_sources(&self, labels: &[String]);
}

struct TauriDesktopActions<'a> {
    app: &'a tauri::AppHandle,
    state: &'a AppState,
}

impl DesktopActions for TauriDesktopActions<'_> {
    fn close_overlays(&self, labels: &[String]) {
        crate::capture::overlay_windows::close(self.app, labels);
    }

    fn restore_pins(&self, labels: &[String]) {
        crate::pin::restore_pins_after_capture(self.app, self.state, labels);
    }

    fn restore_sources(&self, labels: &[String]) {
        crate::capture::overlay_windows::restore(self.app, labels);
    }
}

impl Default for LongshotLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl LongshotLifecycle {
    pub(in crate::capture) fn new() -> Self {
        Self {
            controller: LongshotController::new(),
            slot: Mutex::new(LifecycleSlot::Empty),
        }
    }

    pub(in crate::capture) fn begin(
        &self,
        capture: &CaptureManager,
        selection: &CaptureSelection,
        app: &tauri::AppHandle,
    ) -> Result<LongshotStart, CaptureError> {
        self.begin_with(
            || self.controller.begin(capture, selection),
            |labels| crate::capture::overlay_windows::close(app, labels),
        )
    }

    pub(in crate::capture) fn append(
        &self,
        token: &LongshotSessionToken,
    ) -> Result<LongshotAppendOutcome, CaptureError> {
        self.append_with(token, || self.controller.append(token))
    }

    pub(in crate::capture) fn auto_append(
        &self,
        token: &LongshotSessionToken,
        direction: LongshotAutoDirection,
    ) -> Result<LongshotAppendOutcome, CaptureError> {
        self.append_with(token, || self.controller.auto_append(token, direction))
    }

    #[cfg(all(target_os = "linux", feature = "longshot-wayland-auto"))]
    pub(in crate::capture) fn authorize_wayland_auto(
        &self,
        token: &LongshotSessionToken,
        parent: ashpd::WindowIdentifier,
    ) -> Result<(), CaptureError> {
        self.require_active(token)?;
        let result = self.controller.authorize_wayland_auto(token, parent);
        self.require_still_active(token)?;
        result.map_err(normalize_controller_race)
    }

    #[cfg(all(target_os = "linux", feature = "longshot-wayland-auto"))]
    pub(in crate::capture) fn wayland_auto_authorized(
        &self,
        token: &LongshotSessionToken,
    ) -> Result<bool, CaptureError> {
        self.require_active(token)?;
        self.controller.wayland_auto_authorized(token)
    }

    pub(in crate::capture) fn undo(
        &self,
        token: &LongshotSessionToken,
    ) -> Result<LongshotSnapshot, CaptureError> {
        self.snapshot_with(token, || self.controller.undo(token))
    }

    #[cfg(test)]
    pub(in crate::capture) fn snapshot(
        &self,
        token: &LongshotSessionToken,
    ) -> Result<LongshotSnapshot, CaptureError> {
        self.snapshot_with(token, || self.controller.snapshot(token))
    }

    pub(in crate::capture) fn preview_tail_png(
        &self,
        token: &LongshotSessionToken,
    ) -> Result<Vec<u8>, CaptureError> {
        self.preview_with(token, || self.controller.preview_tail_png(token))
    }

    pub(in crate::capture) fn finish_png(
        &self,
        token: &LongshotSessionToken,
        app: &tauri::AppHandle,
        state: &AppState,
    ) -> Result<LongshotArtifact, CaptureError> {
        let actions = TauriDesktopActions { app, state };
        self.finish_with(token, || self.controller.finish_png(token), &actions)
    }

    pub(in crate::capture) fn cancel(
        &self,
        token: &LongshotSessionToken,
        app: &tauri::AppHandle,
        state: &AppState,
    ) -> Result<(), CaptureError> {
        let actions = TauriDesktopActions { app, state };
        self.cancel_with(token, || self.controller.cancel(token), &actions)
    }

    #[cfg(test)]
    pub(in crate::capture) fn is_active(&self) -> Result<bool, CaptureError> {
        let slot = self.slot.lock().map_err(CaptureError::state_lock)?;
        Ok(!matches!(*slot, LifecycleSlot::Empty))
    }

    /// cleanup 失败后只读确认 exact token 是否仍处于可重试 Active。
    pub(in crate::capture) fn is_exact_active(
        &self,
        token: &LongshotSessionToken,
    ) -> Result<bool, CaptureError> {
        let slot = self.slot.lock().map_err(CaptureError::state_lock)?;
        Ok(matches!(&*slot, LifecycleSlot::Active(session) if session.token == *token))
    }

    fn begin_with<B, C>(&self, begin: B, close_overlays: C) -> Result<LongshotStart, CaptureError>
    where
        B: FnOnce() -> Result<LongshotControllerStart, CaptureError>,
        C: FnOnce(&[String]),
    {
        self.claim_starting()?;
        let LongshotControllerStart { start, resources } = match begin() {
            Ok(started) => started,
            Err(error) => {
                self.rollback_starting_after_primary();
                return Err(error);
            }
        };
        let token = start.token.clone();
        let overlay_labels = resources.overlay_labels();
        {
            let mut slot = self.slot.lock().map_err(CaptureError::state_lock)?;
            if !matches!(*slot, LifecycleSlot::Starting) {
                return Err(CaptureError::LongshotSessionSuperseded);
            }
            *slot = LifecycleSlot::Publishing(SessionResources {
                token: token.clone(),
                resources,
            });
        }

        close_overlays(&overlay_labels);

        let mut slot = self.slot.lock().map_err(CaptureError::state_lock)?;
        let publishing = match std::mem::replace(&mut *slot, LifecycleSlot::Starting) {
            LifecycleSlot::Publishing(session) if session.token == token => session,
            other => {
                *slot = other;
                return Err(CaptureError::LongshotSessionSuperseded);
            }
        };
        *slot = LifecycleSlot::Active(publishing);
        Ok(start)
    }

    fn append_with<F>(
        &self,
        token: &LongshotSessionToken,
        operation: F,
    ) -> Result<LongshotAppendOutcome, CaptureError>
    where
        F: FnOnce() -> Result<LongshotAppendOutcome, CaptureError>,
    {
        self.require_active(token)?;
        let result = operation();
        self.require_still_active(token)?;
        result.map_err(normalize_controller_race)
    }

    fn snapshot_with<F>(
        &self,
        token: &LongshotSessionToken,
        operation: F,
    ) -> Result<LongshotSnapshot, CaptureError>
    where
        F: FnOnce() -> Result<LongshotSnapshot, CaptureError>,
    {
        self.require_active(token)?;
        operation().map_err(normalize_controller_race)
    }

    fn preview_with<F>(
        &self,
        token: &LongshotSessionToken,
        operation: F,
    ) -> Result<Vec<u8>, CaptureError>
    where
        F: FnOnce() -> Result<Vec<u8>, CaptureError>,
    {
        self.require_active(token)?;
        let result = operation();
        self.require_still_active(token)?;
        result.map_err(normalize_controller_race)
    }

    fn finish_with<F, A>(
        &self,
        token: &LongshotSessionToken,
        operation: F,
        actions: &A,
    ) -> Result<LongshotArtifact, CaptureError>
    where
        F: FnOnce() -> Result<LongshotControllerFinish, CaptureError>,
        A: DesktopActions,
    {
        self.claim_terminating(token)?;
        let finished = match operation() {
            Ok(finished) => finished,
            Err(error) => {
                self.rollback_terminating_after_primary(token);
                return Err(error);
            }
        };
        let artifact = finished.artifact;
        self.restore_and_release(token, finished.ownership, actions)?;
        Ok(artifact)
    }

    fn cancel_with<F, A>(
        &self,
        token: &LongshotSessionToken,
        operation: F,
        actions: &A,
    ) -> Result<(), CaptureError>
    where
        F: FnOnce() -> Result<CaptureModeOwnership, CaptureError>,
        A: DesktopActions,
    {
        self.claim_terminating(token)?;
        let ownership = match operation() {
            Ok(ownership) => ownership,
            Err(error) => {
                self.rollback_terminating_after_primary(token);
                return Err(error);
            }
        };
        self.restore_and_release(token, ownership, actions)
    }

    fn restore_and_release<A: DesktopActions>(
        &self,
        token: &LongshotSessionToken,
        ownership: CaptureModeOwnership,
        actions: &A,
    ) -> Result<(), CaptureError> {
        let (overlays, pins, sources) = {
            let mut slot = self.slot.lock().map_err(CaptureError::state_lock)?;
            let terminating = match std::mem::replace(&mut *slot, LifecycleSlot::Starting) {
                LifecycleSlot::Terminating(session) if session.token == *token => session,
                other => {
                    *slot = other;
                    return Err(CaptureError::LongshotSessionSuperseded);
                }
            };
            let overlays = terminating.resources.overlay_labels();
            let pins = terminating.resources.lowered_pins.clone();
            let sources = terminating.resources.restore_labels.clone();
            *slot = LifecycleSlot::Restoring(RestoringSession {
                session: terminating,
                ownership,
            });
            (overlays, pins, sources)
        };

        actions.close_overlays(&overlays);
        actions.restore_pins(&pins);
        actions.restore_sources(&sources);

        let ownership = {
            let mut slot = self.slot.lock().map_err(CaptureError::state_lock)?;
            let restoring =
                match std::mem::replace(&mut *slot, LifecycleSlot::Releasing(token.clone())) {
                    LifecycleSlot::Restoring(restoring) if restoring.session.token == *token => {
                        restoring
                    }
                    other => {
                        *slot = other;
                        return Err(CaptureError::LongshotSessionSuperseded);
                    }
                };
            restoring.ownership
        };

        let release_result = ownership.release();
        let mut slot = self.slot.lock().map_err(CaptureError::state_lock)?;
        match &*slot {
            LifecycleSlot::Releasing(current) if current == token => {}
            _ => return Err(CaptureError::LongshotSessionSuperseded),
        }
        match release_result {
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

    fn claim_starting(&self) -> Result<(), CaptureError> {
        let mut slot = self.slot.lock().map_err(CaptureError::state_lock)?;
        if !matches!(*slot, LifecycleSlot::Empty) {
            return Err(CaptureError::LongshotSessionBusy);
        }
        *slot = LifecycleSlot::Starting;
        Ok(())
    }

    fn rollback_starting_after_primary(&self) {
        match self.slot.lock() {
            Ok(mut slot) if matches!(*slot, LifecycleSlot::Starting) => {
                *slot = LifecycleSlot::Empty;
            }
            Ok(_) => log::error!("长截图启动失败后 lifecycle 已不再是 Starting"),
            Err(error) => log::error!("长截图启动失败后回滚 lifecycle 失败: {error}"),
        }
    }

    fn require_active(&self, token: &LongshotSessionToken) -> Result<(), CaptureError> {
        let slot = self.slot.lock().map_err(CaptureError::state_lock)?;
        match &*slot {
            LifecycleSlot::Empty => Err(CaptureError::LongshotSessionMissing),
            LifecycleSlot::Active(session) if session.token == *token => Ok(()),
            LifecycleSlot::Active(_) => Err(CaptureError::LongshotSessionSuperseded),
            LifecycleSlot::Starting
            | LifecycleSlot::Publishing(_)
            | LifecycleSlot::Terminating(_)
            | LifecycleSlot::Restoring(_)
            | LifecycleSlot::Releasing(_)
            | LifecycleSlot::TerminalFailed => Err(CaptureError::LongshotSessionBusy),
        }
    }

    fn require_still_active(&self, token: &LongshotSessionToken) -> Result<(), CaptureError> {
        let slot = self.slot.lock().map_err(CaptureError::state_lock)?;
        match &*slot {
            LifecycleSlot::Active(session) if session.token == *token => Ok(()),
            _ => Err(CaptureError::LongshotSessionSuperseded),
        }
    }

    fn claim_terminating(&self, token: &LongshotSessionToken) -> Result<(), CaptureError> {
        let mut slot = self.slot.lock().map_err(CaptureError::state_lock)?;
        match &*slot {
            LifecycleSlot::Empty => return Err(CaptureError::LongshotSessionMissing),
            LifecycleSlot::Active(session) if session.token != *token => {
                return Err(CaptureError::LongshotSessionSuperseded);
            }
            LifecycleSlot::Active(_) => {}
            LifecycleSlot::Starting
            | LifecycleSlot::Publishing(_)
            | LifecycleSlot::Terminating(_)
            | LifecycleSlot::Restoring(_)
            | LifecycleSlot::Releasing(_)
            | LifecycleSlot::TerminalFailed => {
                return Err(CaptureError::LongshotSessionBusy);
            }
        }
        let active = match std::mem::replace(&mut *slot, LifecycleSlot::Starting) {
            LifecycleSlot::Active(session) => session,
            _ => unreachable!("已在同一把锁内确认 matching Active"),
        };
        *slot = LifecycleSlot::Terminating(active);
        Ok(())
    }

    fn rollback_terminating_after_primary(&self, token: &LongshotSessionToken) {
        match self.slot.lock() {
            Ok(mut slot) => {
                let previous = std::mem::replace(&mut *slot, LifecycleSlot::Starting);
                match previous {
                    LifecycleSlot::Terminating(session) if session.token == *token => {
                        *slot = LifecycleSlot::Active(session);
                    }
                    other => {
                        *slot = other;
                        log::error!("长截图终结失败后 lifecycle 已不再是 matching Terminating");
                    }
                }
            }
            Err(error) => log::error!("长截图终结失败后回滚 lifecycle 失败: {error}"),
        }
    }
}

fn normalize_controller_race(error: CaptureError) -> CaptureError {
    if matches!(error, CaptureError::LongshotSessionMissing) {
        CaptureError::LongshotSessionSuperseded
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::manager::StageTimings;
    use crate::capture::{CaptureMode, CaptureModeGate};
    use crate::screenshot::CapturedMonitorFrame;
    use image::RgbaImage;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(3);

    fn repeated_id() -> String {
        "lifecycle-repeated-id".to_string()
    }

    fn image(seed: u32) -> RgbaImage {
        RgbaImage::from_fn(64, 72, |x, y| {
            let value = seed
                .wrapping_add(x.wrapping_mul(31))
                .wrapping_add(y.wrapping_mul(17));
            image::Rgba([value as u8, (value >> 2) as u8, (value >> 4) as u8, 255])
        })
    }

    fn captured(seed: u32) -> CapturedMonitorFrame {
        let image = image(seed);
        CapturedMonitorFrame {
            monitor_id: 7,
            x: 0,
            y: 0,
            logical_width: 64,
            logical_height: 72,
            pixel_width: 64,
            pixel_height: 72,
            scale_x: 1.0,
            scale_y: 1.0,
            rgba: Arc::from(image.into_raw()),
        }
    }

    fn ordinary(
        gate: Arc<CaptureModeGate>,
        seed: u32,
    ) -> (CaptureManager, CaptureSelection, Vec<String>) {
        let capture = CaptureManager::new();
        let ownership = Arc::clone(&gate)
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("测试应取得 Ordinary gate");
        let started = capture
            .begin(
                vec![
                    captured(seed),
                    CapturedMonitorFrame {
                        monitor_id: 8,
                        ..captured(seed + 1)
                    },
                ],
                vec!["main".to_string(), "settings".to_string()],
                vec!["pin-b".to_string(), "pin-a".to_string()],
                false,
                StageTimings::default(),
                crate::capture::manager::CaptureBeginAuthorization::new(
                    ownership,
                    crate::capture::CaptureIntent::Screenshot,
                ),
            )
            .expect("普通截图会话应启动");
        let selection = CaptureSelection {
            session_id: started.session_id,
            monitor_id: 7,
            x: 0.0,
            y: 0.0,
            width: 64.0,
            height: 72.0,
        };
        let labels = started
            .overlays
            .into_iter()
            .map(|overlay| overlay.label)
            .collect();
        (capture, selection, labels)
    }

    struct RecordingActions {
        gate: Arc<CaptureModeGate>,
        events: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl RecordingActions {
        fn new(gate: Arc<CaptureModeGate>) -> Self {
            Self {
                gate,
                events: Mutex::new(Vec::new()),
            }
        }

        fn record(&self, kind: &str, labels: &[String]) {
            assert_eq!(
                self.gate.active_mode().expect("读取测试 gate"),
                Some(CaptureMode::Longshot),
                "{kind} 执行时 gate 必须仍为 Longshot"
            );
            self.events
                .lock()
                .expect("记录桌面动作")
                .push((kind.to_string(), labels.to_vec()));
        }
    }

    impl DesktopActions for RecordingActions {
        fn close_overlays(&self, labels: &[String]) {
            self.record("close", labels);
        }

        fn restore_pins(&self, labels: &[String]) {
            self.record("pins", labels);
        }

        fn restore_sources(&self, labels: &[String]) {
            self.record("sources", labels);
        }
    }

    struct RestoringProbeActions<'a> {
        lifecycle: &'a LongshotLifecycle,
        gate: Arc<CaptureModeGate>,
        calls: AtomicUsize,
    }

    impl DesktopActions for RestoringProbeActions<'_> {
        fn close_overlays(&self, _labels: &[String]) {
            assert_eq!(
                self.gate.active_mode().unwrap(),
                Some(CaptureMode::Longshot)
            );
            let error = self
                .lifecycle
                .begin_with(
                    || panic!("Restoring 时不得调用 begin supplier"),
                    |_| panic!("Restoring 时不得调用 begin desktop seam"),
                )
                .unwrap_err();
            assert_eq!(error.code(), "longshot_session_busy");
            self.calls.fetch_add(1, Ordering::SeqCst);
        }

        fn restore_pins(&self, _labels: &[String]) {
            assert_eq!(
                self.gate.active_mode().unwrap(),
                Some(CaptureMode::Longshot)
            );
            self.calls.fetch_add(1, Ordering::SeqCst);
        }

        fn restore_sources(&self, _labels: &[String]) {
            assert_eq!(
                self.gate.active_mode().unwrap(),
                Some(CaptureMode::Longshot)
            );
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn begin_lifecycle(
        lifecycle: &LongshotLifecycle,
        capture: &CaptureManager,
        selection: &CaptureSelection,
        closed: &Mutex<Vec<Vec<String>>>,
    ) -> LongshotStart {
        lifecycle
            .begin_with(
                || lifecycle.controller.begin(capture, selection),
                |labels| closed.lock().unwrap().push(labels.to_vec()),
            )
            .expect("handoff 应成功")
    }

    #[test]
    fn begin_publishes_after_exact_overlay_close_and_keeps_desktop_resources() {
        let lifecycle = LongshotLifecycle::new();
        let gate = Arc::new(CaptureModeGate::new());
        let (capture, selection, labels) = ordinary(Arc::clone(&gate), 1);
        let closed = Mutex::new(Vec::new());
        let supplier_calls = AtomicUsize::new(0);

        let start = lifecycle
            .begin_with(
                || lifecycle.controller.begin(&capture, &selection),
                |observed| {
                    closed.lock().unwrap().push(observed.to_vec());
                    let error = lifecycle
                        .begin_with(
                            || {
                                supplier_calls.fetch_add(1, Ordering::SeqCst);
                                Err(CaptureError::Screenshot("不得调用".into()))
                            },
                            |_| panic!("busy 时不得调用 desktop seam"),
                        )
                        .unwrap_err();
                    assert_eq!(error.code(), "longshot_session_busy");
                },
            )
            .expect("begin 应在 close 后发布");

        assert_eq!(*closed.lock().unwrap(), vec![labels.clone()]);
        assert_eq!(supplier_calls.load(Ordering::SeqCst), 0);
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Longshot));
        assert_eq!(lifecycle.snapshot(&start.token).unwrap().frame_count, 1);
        let stale_restore_calls = AtomicUsize::new(0);
        for label in &labels {
            if capture.abort_if_overlay(label).unwrap().is_some() {
                stale_restore_calls.fetch_add(1, Ordering::SeqCst);
            }
        }
        assert_eq!(stale_restore_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            capture.finish(&selection.session_id).unwrap_err().code(),
            "session_missing"
        );

        let actions = RecordingActions::new(Arc::clone(&gate));
        assert!(actions.events.lock().unwrap().is_empty());
        lifecycle
            .cancel_with(
                &start.token,
                || lifecycle.controller.cancel(&start.token),
                &actions,
            )
            .unwrap();
        assert_eq!(gate.active_mode().unwrap(), None);
    }

    #[test]
    fn append_and_snapshot_business_errors_keep_active_resources_and_gate() {
        let lifecycle = LongshotLifecycle::new();
        let gate = Arc::new(CaptureModeGate::new());
        let (capture, selection, labels) = ordinary(Arc::clone(&gate), 11);
        let start = begin_lifecycle(&lifecycle, &capture, &selection, &Mutex::new(Vec::new()));
        let original = start.snapshot;

        let append_error = lifecycle
            .append_with(&start.token, || {
                lifecycle.controller.append_with(&start.token, |_| {
                    let mut solid = captured(0);
                    solid.rgba = Arc::from(vec![7; 64 * 72 * 4]);
                    Ok(solid)
                })
            })
            .unwrap_err();
        assert_eq!(append_error.code(), "longshot_estimate_low_texture");
        assert_eq!(lifecycle.snapshot(&start.token).unwrap(), original);
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Longshot));

        let snapshot_error = lifecycle
            .snapshot_with(&start.token, || {
                Err(CaptureError::Screenshot("snapshot injected".into()))
            })
            .unwrap_err();
        assert_eq!(snapshot_error.code(), "screenshot");
        assert_eq!(lifecycle.snapshot(&start.token).unwrap(), original);
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Longshot));

        let begin_supplier_calls = AtomicUsize::new(0);
        let busy = lifecycle
            .begin_with(
                || {
                    begin_supplier_calls.fetch_add(1, Ordering::SeqCst);
                    Err(CaptureError::Screenshot("不得调用".into()))
                },
                |_| panic!("Active 时不得调用 begin desktop seam"),
            )
            .unwrap_err();
        assert_eq!(busy.code(), "longshot_session_busy");
        assert_eq!(begin_supplier_calls.load(Ordering::SeqCst), 0);

        let actions = RecordingActions::new(Arc::clone(&gate));
        lifecycle
            .cancel_with(
                &start.token,
                || lifecycle.controller.cancel(&start.token),
                &actions,
            )
            .unwrap();
        assert_eq!(
            *actions.events.lock().unwrap(),
            vec![
                ("close".to_string(), labels),
                (
                    "pins".to_string(),
                    vec!["pin-b".to_string(), "pin-a".to_string()]
                ),
                (
                    "sources".to_string(),
                    vec!["main".to_string(), "settings".to_string()]
                ),
            ]
        );
        assert_eq!(gate.active_mode().unwrap(), None);
    }

    #[test]
    fn preview_checks_active_before_and_after_operation_and_normalizes_race() {
        let lifecycle = LongshotLifecycle::new();
        let gate = Arc::new(CaptureModeGate::new());
        let (capture, selection, _) = ordinary(Arc::clone(&gate), 41);
        let start = begin_lifecycle(&lifecycle, &capture, &selection, &Mutex::new(Vec::new()));

        let stale = LongshotSessionToken::from_wire_parts("stale".to_string(), 999);
        let calls = AtomicUsize::new(0);
        assert!(matches!(
            lifecycle.preview_with(&stale, || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(vec![1])
            }),
            Err(CaptureError::LongshotSessionSuperseded)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let error = lifecycle
            .preview_with(&start.token, || {
                let mut slot = lifecycle.slot.lock().expect("测试 lifecycle 锁");
                let previous = std::mem::replace(&mut *slot, LifecycleSlot::Starting);
                let LifecycleSlot::Active(session) = previous else {
                    panic!("测试开始时必须 Active")
                };
                *slot = LifecycleSlot::Terminating(session);
                Err(CaptureError::Codec("旧 worker 错误".into()))
            })
            .expect_err("阶段变化必须胜过旧 worker 错误");
        assert!(matches!(error, CaptureError::LongshotSessionSuperseded));

        {
            let mut slot = lifecycle.slot.lock().expect("测试 lifecycle 锁");
            let previous = std::mem::replace(&mut *slot, LifecycleSlot::Starting);
            let LifecycleSlot::Terminating(session) = previous else {
                panic!("测试应保留 Terminating")
            };
            *slot = LifecycleSlot::Active(session);
        }
        let preview = lifecycle
            .preview_tail_png(&start.token)
            .expect("恢复后可预览");
        assert_eq!(crate::screenshot::validate_png(&preview).unwrap(), (64, 72));

        let actions = RecordingActions::new(Arc::clone(&gate));
        lifecycle
            .cancel_with(
                &start.token,
                || lifecycle.controller.cancel(&start.token),
                &actions,
            )
            .unwrap();
    }

    #[test]
    fn starting_rejects_second_begin_before_any_supplier_or_desktop_call() {
        let lifecycle = LongshotLifecycle::new();
        let second_supplier_calls = AtomicUsize::new(0);
        let first_error = lifecycle
            .begin_with(
                || {
                    let error = lifecycle
                        .begin_with(
                            || {
                                second_supplier_calls.fetch_add(1, Ordering::SeqCst);
                                Err(CaptureError::Screenshot("不得调用".into()))
                            },
                            |_| panic!("Starting 时不得调用 desktop seam"),
                        )
                        .unwrap_err();
                    assert_eq!(error.code(), "longshot_session_busy");
                    Err(CaptureError::Screenshot("首个启动注入失败".into()))
                },
                |_| panic!("失败的首个启动不得调用 desktop seam"),
            )
            .unwrap_err();
        assert_eq!(first_error.code(), "screenshot");
        assert_eq!(second_supplier_calls.load(Ordering::SeqCst), 0);
        assert!(!lifecycle.is_active().unwrap());
    }

    #[test]
    fn terminal_failure_blocks_begin_before_supplier_and_desktop_calls() {
        let lifecycle = LongshotLifecycle {
            controller: LongshotController::new(),
            slot: Mutex::new(LifecycleSlot::TerminalFailed),
        };
        let supplier_calls = AtomicUsize::new(0);
        let error = lifecycle
            .begin_with(
                || {
                    supplier_calls.fetch_add(1, Ordering::SeqCst);
                    Err(CaptureError::Screenshot("不得调用".into()))
                },
                |_| panic!("TerminalFailed 时不得调用 desktop seam"),
            )
            .unwrap_err();
        assert_eq!(error.code(), "longshot_session_busy");
        assert_eq!(supplier_calls.load(Ordering::SeqCst), 0);
        assert!(lifecycle.is_active().unwrap());
    }

    #[test]
    fn finish_failure_rolls_back_without_desktop_actions_then_retry_restores_in_order() {
        let lifecycle = LongshotLifecycle::new();
        let gate = Arc::new(CaptureModeGate::new());
        let (capture, selection, labels) = ordinary(Arc::clone(&gate), 2);
        let closed = Mutex::new(Vec::new());
        let start = begin_lifecycle(&lifecycle, &capture, &selection, &closed);
        let actions = RecordingActions::new(Arc::clone(&gate));

        let error = lifecycle
            .finish_with(
                &start.token,
                || Err(CaptureError::Codec("injected".into())),
                &actions,
            )
            .unwrap_err();
        assert_eq!(error.code(), "codec");
        assert!(actions.events.lock().unwrap().is_empty());
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Longshot));
        assert_eq!(lifecycle.snapshot(&start.token).unwrap(), start.snapshot);

        let artifact = lifecycle
            .finish_with(
                &start.token,
                || lifecycle.controller.finish_png(&start.token),
                &actions,
            )
            .expect("编码重试应成功");
        assert_eq!(
            crate::screenshot::validate_png(&artifact.png).unwrap(),
            (64, 72)
        );
        assert_eq!(artifact.origin.x, 0.0);
        assert_eq!(artifact.origin.y, 0.0);
        assert_eq!(artifact.origin.width, 64.0);
        assert_eq!(artifact.origin.height, 72.0);
        assert_eq!(
            *actions.events.lock().unwrap(),
            vec![
                ("close".to_string(), labels),
                (
                    "pins".to_string(),
                    vec!["pin-b".to_string(), "pin-a".to_string()]
                ),
                (
                    "sources".to_string(),
                    vec!["main".to_string(), "settings".to_string()]
                ),
            ]
        );
        assert_eq!(gate.active_mode().unwrap(), None);
        assert!(!lifecycle.is_active().unwrap());
    }

    #[test]
    fn restoring_blocks_begin_and_keeps_gate_until_all_desktop_actions_finish() {
        let lifecycle = LongshotLifecycle::new();
        let gate = Arc::new(CaptureModeGate::new());
        let (capture, selection, _) = ordinary(Arc::clone(&gate), 21);
        let start = begin_lifecycle(&lifecycle, &capture, &selection, &Mutex::new(Vec::new()));
        let actions = RestoringProbeActions {
            lifecycle: &lifecycle,
            gate: Arc::clone(&gate),
            calls: AtomicUsize::new(0),
        };

        lifecycle
            .cancel_with(
                &start.token,
                || lifecycle.controller.cancel(&start.token),
                &actions,
            )
            .unwrap();

        assert_eq!(actions.calls.load(Ordering::SeqCst), 3);
        assert_eq!(gate.active_mode().unwrap(), None);
        assert!(!lifecycle.is_active().unwrap());
    }

    #[test]
    fn terminating_claim_makes_competing_cancel_busy_before_operation() {
        let lifecycle = Arc::new(LongshotLifecycle::new());
        let gate = Arc::new(CaptureModeGate::new());
        let (capture, selection, _) = ordinary(Arc::clone(&gate), 3);
        let start = begin_lifecycle(&lifecycle, &capture, &selection, &Mutex::new(Vec::new()));
        let actions = Arc::new(RecordingActions::new(Arc::clone(&gate)));
        let (entered_tx, entered_rx) = mpsc::sync_channel(0);
        let (release_tx, release_rx) = mpsc::sync_channel(0);
        let worker_lifecycle = Arc::clone(&lifecycle);
        let worker_actions = Arc::clone(&actions);
        let worker_token = start.token.clone();
        let worker = std::thread::spawn(move || {
            worker_lifecycle.cancel_with(
                &worker_token,
                || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    worker_lifecycle.controller.cancel(&worker_token)
                },
                worker_actions.as_ref(),
            )
        });
        entered_rx.recv_timeout(WAIT).expect("第一终结者应已认领");

        let calls = AtomicUsize::new(0);
        let error = lifecycle
            .cancel_with(
                &start.token,
                || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    lifecycle.controller.cancel(&start.token)
                },
                actions.as_ref(),
            )
            .unwrap_err();
        assert_eq!(error.code(), "longshot_session_busy");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        release_tx.send(()).unwrap();
        worker.join().unwrap().unwrap();
        assert_eq!(actions.events.lock().unwrap().len(), 3);
        assert_eq!(gate.active_mode().unwrap(), None);

        let finish_gate = Arc::new(CaptureModeGate::new());
        let (finish_capture, finish_selection, _) = ordinary(Arc::clone(&finish_gate), 30);
        let finish_start = begin_lifecycle(
            &lifecycle,
            &finish_capture,
            &finish_selection,
            &Mutex::new(Vec::new()),
        );
        let finish_actions = Arc::new(RecordingActions::new(Arc::clone(&finish_gate)));
        let (finish_entered_tx, finish_entered_rx) = mpsc::sync_channel(0);
        let (finish_release_tx, finish_release_rx) = mpsc::sync_channel(0);
        let finish_lifecycle = Arc::clone(&lifecycle);
        let finish_worker_actions = Arc::clone(&finish_actions);
        let finish_token = finish_start.token.clone();
        let finish_worker = std::thread::spawn(move || {
            finish_lifecycle.finish_with(
                &finish_token,
                || {
                    finish_entered_tx.send(()).unwrap();
                    finish_release_rx.recv().unwrap();
                    finish_lifecycle.controller.finish_png(&finish_token)
                },
                finish_worker_actions.as_ref(),
            )
        });
        finish_entered_rx
            .recv_timeout(WAIT)
            .expect("finish 终结者应已认领");
        let losing_cancel_calls = AtomicUsize::new(0);
        let error = lifecycle
            .cancel_with(
                &finish_start.token,
                || {
                    losing_cancel_calls.fetch_add(1, Ordering::SeqCst);
                    lifecycle.controller.cancel(&finish_start.token)
                },
                finish_actions.as_ref(),
            )
            .unwrap_err();
        assert_eq!(error.code(), "longshot_session_busy");
        assert_eq!(losing_cancel_calls.load(Ordering::SeqCst), 0);
        finish_release_tx.send(()).unwrap();
        let artifact = finish_worker.join().unwrap().unwrap();
        assert_eq!(
            crate::screenshot::validate_png(&artifact.png).unwrap(),
            (64, 72)
        );
        assert_eq!(finish_actions.events.lock().unwrap().len(), 3);
        assert_eq!(finish_gate.active_mode().unwrap(), None);
    }

    #[test]
    fn late_append_and_repeated_id_old_generation_are_superseded() {
        let lifecycle = Arc::new(LongshotLifecycle {
            controller: LongshotController::with_test_state(repeated_id, 0),
            slot: Mutex::new(LifecycleSlot::Empty),
        });
        let first_gate = Arc::new(CaptureModeGate::new());
        let (first_capture, first_selection, _) = ordinary(Arc::clone(&first_gate), 4);
        let first = begin_lifecycle(
            &lifecycle,
            &first_capture,
            &first_selection,
            &Mutex::new(Vec::new()),
        );
        let (entered_tx, entered_rx) = mpsc::sync_channel(0);
        let (release_tx, release_rx) = mpsc::sync_channel(0);
        let append_lifecycle = Arc::clone(&lifecycle);
        let append_token = first.token.clone();
        let append = std::thread::spawn(move || {
            append_lifecycle.append_with(&append_token, || {
                append_lifecycle.controller.append_with(&append_token, |_| {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(captured(4 + 24 * 17))
                })
            })
        });
        entered_rx.recv_timeout(WAIT).unwrap();
        let first_actions = RecordingActions::new(Arc::clone(&first_gate));
        lifecycle
            .cancel_with(
                &first.token,
                || lifecycle.controller.cancel(&first.token),
                &first_actions,
            )
            .unwrap();
        release_tx.send(()).unwrap();
        assert_eq!(
            append.join().unwrap().unwrap_err().code(),
            "longshot_session_superseded"
        );

        let second_gate = Arc::new(CaptureModeGate::new());
        let (second_capture, second_selection, _) = ordinary(Arc::clone(&second_gate), 5);
        let second = begin_lifecycle(
            &lifecycle,
            &second_capture,
            &second_selection,
            &Mutex::new(Vec::new()),
        );
        assert_ne!(first.token, second.token);
        let stale_operation_calls = AtomicUsize::new(0);
        assert_eq!(
            lifecycle
                .append_with(&first.token, || {
                    stale_operation_calls.fetch_add(1, Ordering::SeqCst);
                    Err(CaptureError::Screenshot("不得调用".into()))
                })
                .unwrap_err()
                .code(),
            "longshot_session_superseded"
        );
        assert_eq!(
            lifecycle
                .snapshot_with(&first.token, || {
                    stale_operation_calls.fetch_add(1, Ordering::SeqCst);
                    Err(CaptureError::Screenshot("不得调用".into()))
                })
                .unwrap_err()
                .code(),
            "longshot_session_superseded"
        );
        assert_eq!(
            lifecycle
                .finish_with(
                    &first.token,
                    || {
                        stale_operation_calls.fetch_add(1, Ordering::SeqCst);
                        Err(CaptureError::Screenshot("不得调用".into()))
                    },
                    &RecordingActions::new(Arc::clone(&second_gate)),
                )
                .unwrap_err()
                .code(),
            "longshot_session_superseded"
        );
        let second_actions = RecordingActions::new(Arc::clone(&second_gate));
        assert_eq!(
            lifecycle
                .cancel_with(
                    &first.token,
                    || {
                        stale_operation_calls.fetch_add(1, Ordering::SeqCst);
                        Err(CaptureError::Screenshot("不得调用".into()))
                    },
                    &second_actions,
                )
                .unwrap_err()
                .code(),
            "longshot_session_superseded"
        );
        assert_eq!(stale_operation_calls.load(Ordering::SeqCst), 0);
        assert!(second_actions.events.lock().unwrap().is_empty());
        lifecycle
            .cancel_with(
                &second.token,
                || lifecycle.controller.cancel(&second.token),
                &second_actions,
            )
            .unwrap();
        assert_eq!(second_gate.active_mode().unwrap(), None);
    }
}
