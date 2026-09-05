//! 长截图帧来源、固定裁剪与代次安全会话之间的单槽所有权层。
//!
//! controller 锁只保护轻量 owner；捕获、裁剪、重叠估计、拼接和 PNG 编码均由
//! manager 的事务 lease 在两把状态锁之外完成。

use super::{
    capture_monitor_frame, LongshotAppendOutcome, LongshotFrameAdapter, LongshotManager,
    LongshotSession, LongshotSessionToken, LongshotSnapshot, LongshotStart,
};
use crate::capture::manager::{
    CaptureLongshotCandidate, CaptureLongshotHandoff, OrdinaryCaptureResources,
};
use crate::capture::{CaptureError, CaptureManager, CaptureModeOwnership, CaptureSelection};
use crate::screenshot::CapturedMonitorFrame;
use std::sync::Mutex;

pub(in crate::capture) struct LongshotController {
    manager: LongshotManager,
    slot: Mutex<ControllerSlot>,
}

enum ControllerSlot {
    Empty,
    Starting,
    Active(Owner),
}

#[derive(Debug)]
struct Owner {
    token: LongshotSessionToken,
    monitor_id: u32,
    adapter: LongshotFrameAdapter,
    mode_ownership: CaptureModeOwnership,
}

struct OwnerLease {
    monitor_id: u32,
    adapter: LongshotFrameAdapter,
}

/// 长截图开始结果与普通截图桌面资源的唯一移交包。
#[derive(Debug)]
pub(in crate::capture) struct LongshotControllerStart {
    pub start: LongshotStart,
    pub resources: OrdinaryCaptureResources,
}

impl Default for LongshotController {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)] // 下一切片接入 AppState/IPC；当前先固定同步领域合同。
impl LongshotController {
    pub(in crate::capture) fn new() -> Self {
        Self {
            manager: LongshotManager::new(),
            slot: Mutex::new(ControllerSlot::Empty),
        }
    }

    /// 准备首帧后原子消费普通截图会话，并接管 Longshot 模式所有权。
    pub(in crate::capture) fn begin(
        &self,
        capture: &CaptureManager,
        selection: &CaptureSelection,
    ) -> Result<LongshotControllerStart, CaptureError> {
        self.begin_with(
            || capture.prepare_longshot(selection),
            |candidate| capture.commit_longshot(candidate),
            |token| self.manager.cancel(token),
        )
    }

    fn begin_with<P, C, X>(
        &self,
        prepare: P,
        commit: C,
        cancel_prepared: X,
    ) -> Result<LongshotControllerStart, CaptureError>
    where
        P: FnOnce() -> Result<CaptureLongshotCandidate, CaptureError>,
        C: FnOnce(CaptureLongshotCandidate) -> Result<CaptureLongshotHandoff, CaptureError>,
        X: FnOnce(&LongshotSessionToken) -> Result<bool, CaptureError>,
    {
        self.claim_starting()?;

        let prepared = (|| {
            let candidate = prepare()?;
            let frame = candidate.frame();
            let selection = candidate.selection();
            let monitor_id = frame.monitor_id;
            let (adapter, first_crop) = LongshotFrameAdapter::from_first(frame, selection)?;
            let started = self.manager.begin(first_crop)?;
            Ok((candidate, started, monitor_id, adapter))
        })();

        let (candidate, started, monitor_id, adapter) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.rollback_starting_after_primary();
                return Err(error);
            }
        };
        let owner_token = started.token.clone();

        let mut slot = match self.slot.lock() {
            Ok(slot) => slot,
            Err(error) => {
                let primary = CaptureError::state_lock(error);
                self.cancel_prepared_after_primary(
                    &owner_token,
                    cancel_prepared,
                    "取得 controller 最终提交锁失败",
                );
                return Err(primary);
            }
        };
        if !matches!(*slot, ControllerSlot::Starting) {
            drop(slot);
            let primary = CaptureError::LongshotSessionSuperseded;
            self.cancel_prepared_after_primary(
                &owner_token,
                cancel_prepared,
                "controller 最终提交槽位已更新",
            );
            return Err(primary);
        }

        let CaptureLongshotHandoff {
            ownership,
            resources,
        } = match commit(candidate) {
            Ok(handoff) => handoff,
            Err(primary) => {
                drop(slot);
                self.cancel_prepared_after_primary(
                    &owner_token,
                    cancel_prepared,
                    "普通截图 handoff 提交失败",
                );
                return Err(primary);
            }
        };

        // gate commit 后不再调用任何可失败逻辑，只移动预先准备好的值。
        *slot = ControllerSlot::Active(Owner {
            token: owner_token,
            monitor_id,
            adapter,
            mode_ownership: ownership,
        });
        Ok(LongshotControllerStart {
            start: started,
            resources,
        })
    }

    /// 重捕获冻结显示器，并在同一个 manager append lease 内完成裁剪与提交。
    pub(in crate::capture) fn append(
        &self,
        token: &LongshotSessionToken,
    ) -> Result<LongshotAppendOutcome, CaptureError> {
        self.append_with(token, capture_monitor_frame)
    }

    fn append_with<F>(
        &self,
        token: &LongshotSessionToken,
        capture: F,
    ) -> Result<LongshotAppendOutcome, CaptureError>
    where
        F: FnOnce(u32) -> Result<CapturedMonitorFrame, CaptureError>,
    {
        let lease = self.owner_lease(token)?;
        self.manager
            .append_with(token, move |session| {
                let frame = capture(lease.monitor_id)?;
                let cropped = lease.adapter.crop_next(&frame)?;
                session.append(cropped)
            })
            .map_err(normalize_claim_race)
    }

    /// 读取最后一次已提交的几何快照。
    pub(in crate::capture) fn snapshot(
        &self,
        token: &LongshotSessionToken,
    ) -> Result<LongshotSnapshot, CaptureError> {
        self.owner_lease(token)?;
        self.manager.snapshot(token).map_err(normalize_claim_race)
    }

    /// 锁外编码 PNG；成功消费 manager 后再按完整 token 条件清除 owner。
    pub(in crate::capture) fn finish_png(
        &self,
        token: &LongshotSessionToken,
    ) -> Result<Vec<u8>, CaptureError> {
        self.finish_with(token, LongshotSession::finish_png)
    }

    fn finish_with<F>(
        &self,
        token: &LongshotSessionToken,
        operation: F,
    ) -> Result<Vec<u8>, CaptureError>
    where
        F: FnOnce(&LongshotSession) -> Result<Vec<u8>, CaptureError>,
    {
        self.owner_lease(token)?;
        let png = self
            .manager
            .finish_with(token, operation)
            .map_err(normalize_claim_race)?;
        let owner = self.take_owner(token)?;
        owner.mode_ownership.release()?;
        Ok(png)
    }

    /// 立即使 matching owner 及其 manager lease 失效，不等待锁外工作结束。
    pub(in crate::capture) fn cancel(
        &self,
        token: &LongshotSessionToken,
    ) -> Result<bool, CaptureError> {
        {
            let slot = self.slot.lock().map_err(CaptureError::state_lock)?;
            match &*slot {
                ControllerSlot::Empty => return Ok(false),
                ControllerSlot::Starting => return Err(CaptureError::LongshotSessionBusy),
                ControllerSlot::Active(owner) if owner.token != *token => {
                    return Err(CaptureError::LongshotSessionSuperseded);
                }
                ControllerSlot::Active(_) => {}
            }
        }
        let cancelled = self.manager.cancel(token)?;
        if cancelled {
            let owner = self.take_owner(token)?;
            owner.mode_ownership.release()?;
        }
        Ok(cancelled)
    }

    /// `Starting` 也属于占用状态，避免准备首帧时另一模式进入。
    pub(in crate::capture) fn is_active(&self) -> Result<bool, CaptureError> {
        let slot = self.slot.lock().map_err(CaptureError::state_lock)?;
        Ok(!matches!(*slot, ControllerSlot::Empty))
    }

    fn claim_starting(&self) -> Result<(), CaptureError> {
        let mut slot = self.slot.lock().map_err(CaptureError::state_lock)?;
        if !matches!(*slot, ControllerSlot::Empty) {
            return Err(CaptureError::LongshotSessionBusy);
        }
        *slot = ControllerSlot::Starting;
        Ok(())
    }

    fn rollback_starting(&self) -> Result<(), CaptureError> {
        let mut slot = self.slot.lock().map_err(CaptureError::state_lock)?;
        if matches!(*slot, ControllerSlot::Starting) {
            *slot = ControllerSlot::Empty;
        }
        Ok(())
    }

    fn rollback_starting_after_primary(&self) {
        if let Err(error) = self.rollback_starting() {
            log::error!("长截图启动失败后回滚 controller Starting 也失败: {error}");
        }
    }

    fn cancel_prepared_after_primary<X>(
        &self,
        token: &LongshotSessionToken,
        cancel_prepared: X,
        context: &str,
    ) where
        X: FnOnce(&LongshotSessionToken) -> Result<bool, CaptureError>,
    {
        match cancel_prepared(token) {
            Ok(_) => self.rollback_starting_after_primary(),
            Err(error) => {
                log::error!("{context}，撤销已准备长截图会话也失败: {error}");
            }
        }
    }

    fn owner_lease(&self, token: &LongshotSessionToken) -> Result<OwnerLease, CaptureError> {
        let slot = self.slot.lock().map_err(CaptureError::state_lock)?;
        match &*slot {
            ControllerSlot::Empty => Err(CaptureError::LongshotSessionMissing),
            ControllerSlot::Starting => Err(CaptureError::LongshotSessionBusy),
            ControllerSlot::Active(owner) if owner.token != *token => {
                Err(CaptureError::LongshotSessionSuperseded)
            }
            ControllerSlot::Active(owner) => Ok(OwnerLease {
                monitor_id: owner.monitor_id,
                adapter: owner.adapter,
            }),
        }
    }

    fn take_owner(&self, token: &LongshotSessionToken) -> Result<Owner, CaptureError> {
        let mut slot = self.slot.lock().map_err(CaptureError::state_lock)?;
        match &*slot {
            ControllerSlot::Empty => return Err(CaptureError::LongshotSessionMissing),
            ControllerSlot::Starting => return Err(CaptureError::LongshotSessionBusy),
            ControllerSlot::Active(owner) if owner.token != *token => {
                return Err(CaptureError::LongshotSessionSuperseded);
            }
            ControllerSlot::Active(_) => {}
        }
        match std::mem::replace(&mut *slot, ControllerSlot::Empty) {
            ControllerSlot::Active(owner) => Ok(owner),
            ControllerSlot::Empty | ControllerSlot::Starting => {
                unreachable!("已在同一把锁内验证 matching owner")
            }
        }
    }
}

fn normalize_claim_race(error: CaptureError) -> CaptureError {
    if matches!(error, CaptureError::LongshotSessionMissing) {
        CaptureError::LongshotSessionSuperseded
    } else {
        error
    }
}

#[cfg(test)]
impl LongshotController {
    fn with_test_state(id_supplier: fn() -> String, last_generation: u64) -> Self {
        Self {
            manager: LongshotManager::with_test_state(id_supplier, last_generation),
            slot: Mutex::new(ControllerSlot::Empty),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::manager::StageTimings;
    use crate::capture::{CaptureMode, CaptureModeGate};
    use image::{imageops, RgbaImage};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(3);

    fn repeated_id() -> String {
        "controller-repeated-id".to_string()
    }

    fn panorama(width: u32, height: u32, seed: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            let mut value = seed
                ^ x.wrapping_mul(0x9e37_79b9)
                ^ y.wrapping_mul(0x85eb_ca6b)
                ^ (x ^ y).wrapping_mul(0xc2b2_ae35);
            value ^= value >> 16;
            value = value.wrapping_mul(0x7feb_352d);
            value ^= value >> 15;
            image::Rgba([value as u8, (value >> 8) as u8, (value >> 16) as u8, 255])
        })
    }

    fn captured(image: RgbaImage, monitor_id: u32) -> CapturedMonitorFrame {
        let (pixel_width, pixel_height) = image.dimensions();
        CapturedMonitorFrame {
            monitor_id,
            x: -20,
            y: 30,
            logical_width: pixel_width,
            logical_height: pixel_height,
            pixel_width,
            pixel_height,
            scale_x: 1.0,
            scale_y: 1.0,
            rgba: Arc::from(image.into_raw()),
        }
    }

    fn selection(session_id: &str, monitor_id: u32) -> CaptureSelection {
        CaptureSelection {
            session_id: session_id.to_string(),
            monitor_id,
            x: 0.0,
            y: 0.0,
            width: 64.0,
            height: 72.0,
        }
    }

    fn ordinary_capture(
        first: RgbaImage,
        gate: Arc<CaptureModeGate>,
        restore_labels: Vec<String>,
        lowered_pins: Vec<String>,
    ) -> (CaptureManager, CaptureSelection, Vec<String>) {
        ordinary_capture_frame(captured(first, 7), gate, restore_labels, lowered_pins)
    }

    fn ordinary_capture_frame(
        first: CapturedMonitorFrame,
        gate: Arc<CaptureModeGate>,
        restore_labels: Vec<String>,
        lowered_pins: Vec<String>,
    ) -> (CaptureManager, CaptureSelection, Vec<String>) {
        let capture = CaptureManager::new();
        let ownership = Arc::clone(&gate)
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("测试应取得 Ordinary");
        let started = capture
            .begin(
                vec![first],
                restore_labels,
                lowered_pins,
                false,
                StageTimings::default(),
                ownership,
            )
            .expect("普通截图应启动");
        let labels = started
            .overlays
            .iter()
            .map(|overlay| overlay.label.clone())
            .collect();
        (capture, selection(&started.session_id, 7), labels)
    }

    fn begin_direct(
        controller: &LongshotController,
        first: RgbaImage,
    ) -> (LongshotStart, CaptureSelection, Arc<CaptureModeGate>) {
        let gate = Arc::new(CaptureModeGate::new());
        let (capture, selection, _) = ordinary_capture(first, Arc::clone(&gate), vec![], vec![]);
        let result = controller
            .begin(&capture, &selection)
            .expect("有效首帧应完成 handoff");
        (result.start, selection, gate)
    }

    #[test]
    fn begin_handoff_moves_resources_and_longshot_mode() {
        let gate = Arc::new(CaptureModeGate::new());
        let restores = vec!["main".to_string(), "settings".to_string()];
        let pins = vec!["pin-b".to_string(), "pin-a".to_string()];
        let (capture, selection, overlay_labels) = ordinary_capture(
            panorama(64, 72, 1),
            Arc::clone(&gate),
            restores.clone(),
            pins.clone(),
        );
        let controller = LongshotController::new();

        let result = controller
            .begin(&capture, &selection)
            .expect("handoff 成功");
        assert_eq!(result.resources.overlay_labels(), overlay_labels);
        assert_eq!(result.resources.restore_labels, restores);
        assert_eq!(result.resources.lowered_pins, pins);
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Longshot));
        assert_eq!(
            controller
                .snapshot(&result.start.token)
                .unwrap()
                .frame_count,
            1
        );
        assert_eq!(
            capture.crop(&selection).unwrap_err().code(),
            "session_missing"
        );
        assert_eq!(
            capture.finish(&selection.session_id).unwrap_err().code(),
            "session_missing"
        );
        assert!(capture
            .abort_if_overlay(&overlay_labels[0])
            .unwrap()
            .is_none());
        assert!(controller.cancel(&result.start.token).unwrap());
        assert_eq!(gate.active_mode().unwrap(), None);
    }

    #[test]
    fn begin_prepare_and_geometry_failures_preserve_ordinary_session() {
        let capture = CaptureManager::new();
        let controller = LongshotController::new();
        let missing = selection("missing", 7);
        assert_eq!(
            controller.begin(&capture, &missing).unwrap_err().code(),
            "session_missing"
        );
        assert!(!controller.is_active().unwrap());

        let gate = Arc::new(CaptureModeGate::new());
        let (capture, valid, _) =
            ordinary_capture(panorama(64, 72, 2), Arc::clone(&gate), vec![], vec![]);
        let stale = selection("stale", 7);
        assert_eq!(
            controller.begin(&capture, &stale).unwrap_err().code(),
            "session_superseded_retry"
        );
        let wrong_monitor = selection(&valid.session_id, 9);
        assert_eq!(
            controller
                .begin(&capture, &wrong_monitor)
                .unwrap_err()
                .code(),
            "selection_monitor_mismatch"
        );

        let mut invalid = valid.clone();
        invalid.width = f64::NAN;
        assert_eq!(
            controller.begin(&capture, &invalid).unwrap_err().code(),
            "selection_not_finite"
        );
        let mut too_small = valid.clone();
        too_small.width = 1.0;
        assert_eq!(
            controller.begin(&capture, &too_small).unwrap_err().code(),
            "selection_too_small"
        );
        let mut empty = valid.clone();
        empty.x = 1000.0;
        assert_eq!(
            controller.begin(&capture, &empty).unwrap_err().code(),
            "selection_empty"
        );
        assert!(!controller.is_active().unwrap());
        assert!(capture.crop(&valid).is_ok());
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Ordinary));
        capture
            .finish(&valid.session_id)
            .unwrap()
            .finalize_mode()
            .unwrap();

        let malformed_gate = Arc::new(CaptureModeGate::new());
        let mut malformed = captured(panorama(64, 72, 21), 7);
        malformed.rgba = Arc::from(vec![0; 17]);
        let (malformed_capture, malformed_selection, _) =
            ordinary_capture_frame(malformed, Arc::clone(&malformed_gate), vec![], vec![]);
        assert_eq!(
            controller
                .begin(&malformed_capture, &malformed_selection)
                .unwrap_err()
                .code(),
            "longshot_frame_invalid"
        );
        assert_eq!(
            malformed_gate.active_mode().unwrap(),
            Some(CaptureMode::Ordinary)
        );
        malformed_capture
            .finish(&malformed_selection.session_id)
            .unwrap()
            .finalize_mode()
            .unwrap();
    }

    #[test]
    fn starting_rejects_prepare_and_commit_without_touching_ordinary_session() {
        let controller = Arc::new(LongshotController::new());
        let first_gate = Arc::new(CaptureModeGate::new());
        let (first_capture, first_selection, _) =
            ordinary_capture(panorama(64, 72, 3), Arc::clone(&first_gate), vec![], vec![]);
        let first_candidate = first_capture.prepare_longshot(&first_selection).unwrap();
        let (entered_tx, entered_rx) = mpsc::sync_channel(0);
        let (release_tx, release_rx) = mpsc::sync_channel(0);
        let worker_controller = Arc::clone(&controller);
        let worker = std::thread::spawn(move || {
            worker_controller.begin_with(
                || {
                    entered_tx.send(()).expect("主线程应等待进入通知");
                    release_rx.recv().expect("主线程应显式释放 prepare");
                    Ok(first_candidate)
                },
                |candidate| first_capture.commit_longshot(candidate),
                |token| worker_controller.manager.cancel(token),
            )
        });
        entered_rx
            .recv_timeout(WAIT)
            .expect("supplier 应进入锁外阶段");
        assert!(controller.is_active().unwrap());
        let prepare_called = AtomicBool::new(false);
        let commit_called = AtomicBool::new(false);
        let cancel_called = AtomicBool::new(false);
        let busy = controller
            .begin_with(
                || {
                    prepare_called.store(true, Ordering::SeqCst);
                    Err(CaptureError::Screenshot("不应调用 prepare".to_string()))
                },
                |_| {
                    commit_called.store(true, Ordering::SeqCst);
                    Err(CaptureError::SessionSuperseded)
                },
                |_| {
                    cancel_called.store(true, Ordering::SeqCst);
                    Ok(false)
                },
            )
            .unwrap_err();
        assert_eq!(busy.code(), "longshot_session_busy");
        assert!(!prepare_called.load(Ordering::SeqCst));
        assert!(!commit_called.load(Ordering::SeqCst));
        assert!(!cancel_called.load(Ordering::SeqCst));
        let second_gate = Arc::new(CaptureModeGate::new());
        let (second_capture, second_selection, _) = ordinary_capture(
            panorama(64, 72, 4),
            Arc::clone(&second_gate),
            vec![],
            vec![],
        );
        let error = controller
            .begin(&second_capture, &second_selection)
            .unwrap_err();
        assert_eq!(error.code(), "longshot_session_busy");
        assert!(second_capture.crop(&second_selection).is_ok());
        assert_eq!(
            second_gate.active_mode().unwrap(),
            Some(CaptureMode::Ordinary)
        );
        release_tx.send(()).expect("应释放首个 prepare");
        let first = worker.join().expect("begin 线程不应 panic").unwrap();
        assert_eq!(
            controller
                .begin(&second_capture, &second_selection)
                .unwrap_err()
                .code(),
            "longshot_session_busy"
        );
        assert!(controller.cancel(&first.start.token).unwrap());
        second_capture
            .finish(&second_selection.session_id)
            .unwrap()
            .finalize_mode()
            .unwrap();
    }

    #[test]
    fn longshot_generation_failure_preserves_ordinary_for_retry() {
        let controller = LongshotController::with_test_state(repeated_id, u64::MAX);
        let gate = Arc::new(CaptureModeGate::new());
        let (capture, selection, _) =
            ordinary_capture(panorama(64, 72, 10), Arc::clone(&gate), vec![], vec![]);
        assert_eq!(
            controller.begin(&capture, &selection).unwrap_err().code(),
            "longshot_generation_exhausted"
        );
        assert!(!controller.is_active().unwrap());
        assert!(capture.crop(&selection).is_ok());
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Ordinary));
        capture
            .finish(&selection.session_id)
            .unwrap()
            .finalize_mode()
            .unwrap();
    }

    #[test]
    fn append_failures_and_success_keep_longshot_mode_until_finish() {
        let controller = LongshotController::new();
        let source = panorama(64, 128, 5);
        let (started, _, gate) = begin_direct(
            &controller,
            imageops::crop_imm(&source, 0, 0, 64, 72).to_image(),
        );
        let original = controller.snapshot(&started.token).unwrap();
        let seen_monitor = std::cell::Cell::new(0);
        let error = controller
            .append_with(&started.token, |monitor_id| {
                seen_monitor.set(monitor_id);
                Err(CaptureError::Screenshot("injected".into()))
            })
            .unwrap_err();
        assert_eq!(seen_monitor.get(), 7);
        assert_eq!(error.code(), "screenshot");
        assert_eq!(controller.snapshot(&started.token).unwrap(), original);
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Longshot));
        assert_eq!(
            gate.try_claim(CaptureMode::Ordinary).unwrap_err().code(),
            "capture_mode_busy"
        );

        let mut changed = captured(imageops::crop_imm(&source, 0, 24, 64, 72).to_image(), 7);
        changed.x += 1;
        assert_eq!(
            controller
                .append_with(&started.token, |_| Ok(changed))
                .unwrap_err()
                .code(),
            "longshot_frame_geometry_changed"
        );
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Longshot));
        assert_eq!(
            controller
                .append_with(&started.token, |_| {
                    Ok(captured(
                        RgbaImage::from_pixel(64, 72, image::Rgba([3, 3, 3, 255])),
                        7,
                    ))
                })
                .unwrap_err()
                .code(),
            "longshot_estimate_low_texture"
        );
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Longshot));

        let outcome = controller
            .append_with(&started.token, |_| {
                assert_eq!(
                    controller.snapshot(&started.token).unwrap(),
                    original,
                    "捕获闭包内应能读取 manager 的 committed snapshot"
                );
                Ok(captured(
                    imageops::crop_imm(&source, 0, 24, 64, 72).to_image(),
                    7,
                ))
            })
            .expect("合法帧应可重试");
        assert_eq!(outcome.snapshot.frame_count, 2);
        assert_eq!(outcome.snapshot.total_height, 96);
        let png = controller.finish_png(&started.token).expect("两帧应可完成");
        let decoded = image::load_from_memory(&png).unwrap().to_rgba8();
        let expected = imageops::crop_imm(&source, 0, 0, 64, 96).to_image();
        assert_eq!(decoded, expected, "拼接 PNG 应逐像素等于原始全景前 96 行");
        assert_eq!(gate.active_mode().unwrap(), None);
    }

    #[test]
    fn commit_superseded_cancels_prepared_session_and_allows_retry() {
        let controller = LongshotController::new();
        let gate = Arc::new(CaptureModeGate::new());
        let (capture, selection, _) =
            ordinary_capture(panorama(64, 72, 13), Arc::clone(&gate), vec![], vec![]);
        let candidate = capture.prepare_longshot(&selection).unwrap();
        assert_eq!(
            controller
                .begin_with(
                    || Ok(candidate),
                    |_| Err(CaptureError::SessionSuperseded),
                    |token| controller.manager.cancel(token),
                )
                .unwrap_err()
                .code(),
            "session_superseded"
        );
        assert!(!controller.is_active().unwrap());
        assert!(capture.crop(&selection).is_ok());
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Ordinary));
        let retried = controller.begin(&capture, &selection).unwrap();
        assert!(controller.cancel(&retried.start.token).unwrap());
    }

    #[test]
    fn ordinary_finisher_wins_before_commit_and_prepared_session_is_cancelled() {
        let gate = Arc::new(CaptureModeGate::new());
        let (capture, selection, _) =
            ordinary_capture(panorama(64, 72, 22), Arc::clone(&gate), vec![], vec![]);
        let candidate = capture.prepare_longshot(&selection).unwrap();
        let session_id = selection.session_id.clone();
        let controller = LongshotController::new();
        let error = controller
            .begin_with(
                || Ok(candidate),
                |candidate| {
                    capture.finish(&session_id)?.finalize_mode()?;
                    capture.commit_longshot(candidate)
                },
                |token| controller.manager.cancel(token),
            )
            .unwrap_err();
        assert_eq!(error.code(), "session_missing");
        assert!(!controller.is_active().unwrap());
        assert_eq!(gate.active_mode().unwrap(), None);

        let (retry_capture, retry_selection, _) =
            ordinary_capture(panorama(64, 72, 23), Arc::clone(&gate), vec![], vec![]);
        let retry = controller.begin(&retry_capture, &retry_selection).unwrap();
        assert!(controller.cancel(&retry.start.token).unwrap());
    }

    #[test]
    fn capture_mode_generation_exhaustion_rolls_back_prepared_session() {
        let gate = Arc::new(CaptureModeGate::with_last_generation(u64::MAX - 1));
        let (capture, selection, _) =
            ordinary_capture(panorama(64, 72, 6), Arc::clone(&gate), vec![], vec![]);
        let controller = LongshotController::new();
        assert_eq!(
            controller.begin(&capture, &selection).unwrap_err().code(),
            "capture_mode_generation_exhausted"
        );
        assert!(!controller.is_active().unwrap());
        assert!(capture.crop(&selection).is_ok());
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Ordinary));
        capture
            .finish(&selection.session_id)
            .unwrap()
            .finalize_mode()
            .unwrap();
    }

    #[test]
    fn finish_error_retries_png_and_releases_mode_only_after_success() {
        let controller = LongshotController::new();
        let first = panorama(64, 72, 8);
        let expected = first.as_raw().clone();
        let (started, _, gate) = begin_direct(&controller, first);
        let error = controller
            .finish_with(&started.token, |_| {
                Err(CaptureError::Codec("injected".into()))
            })
            .unwrap_err();
        assert_eq!(error.code(), "codec");
        assert!(controller.is_active().unwrap());
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Longshot));
        assert_eq!(
            controller.snapshot(&started.token).unwrap(),
            started.snapshot
        );

        let png = controller
            .finish_png(&started.token)
            .expect("失败后应可真实完成");
        assert_eq!(crate::screenshot::validate_png(&png).unwrap(), (64, 72));
        assert_eq!(
            image::load_from_memory(&png).unwrap().to_rgba8().as_raw(),
            &expected
        );
        assert!(!controller.is_active().unwrap());
        assert_eq!(gate.active_mode().unwrap(), None);
        assert_eq!(
            controller.snapshot(&started.token).unwrap_err().code(),
            "longshot_session_missing"
        );
        assert!(!controller.cancel(&started.token).unwrap());
    }

    #[test]
    fn finish_cancel_race_releases_once_and_old_token_cannot_touch_next_generation() {
        let controller = Arc::new(LongshotController::with_test_state(repeated_id, 0));
        let (started, _, gate) = begin_direct(&controller, panorama(64, 72, 15));
        let (entered_tx, entered_rx) = mpsc::sync_channel(0);
        let (release_tx, release_rx) = mpsc::sync_channel(0);
        let worker_controller = Arc::clone(&controller);
        let worker_token = started.token.clone();
        let worker = std::thread::spawn(move || {
            worker_controller.finish_with(&worker_token, |session| {
                entered_tx.send(()).expect("主线程应等待编码进入通知");
                release_rx.recv().expect("主线程应显式释放编码");
                session.finish_png()
            })
        });
        entered_rx.recv_timeout(WAIT).expect("编码闭包应已进入");
        assert_eq!(
            controller.snapshot(&started.token).unwrap(),
            started.snapshot
        );
        let second_called = AtomicBool::new(false);
        let error = controller
            .finish_with(&started.token, |_| {
                second_called.store(true, Ordering::SeqCst);
                Ok(Vec::new())
            })
            .unwrap_err();
        assert_eq!(error.code(), "longshot_session_busy");
        assert!(!second_called.load(Ordering::SeqCst));
        assert!(controller.cancel(&started.token).unwrap());
        assert_eq!(gate.active_mode().unwrap(), None);
        let (next_capture, next_selection, _) =
            ordinary_capture(panorama(64, 72, 16), Arc::clone(&gate), vec![], vec![]);
        let next = controller.begin(&next_capture, &next_selection).unwrap();
        release_tx.send(()).expect("应释放旧编码");
        assert_eq!(
            worker
                .join()
                .expect("finish 线程不应 panic")
                .unwrap_err()
                .code(),
            "longshot_session_superseded"
        );
        assert_eq!(
            controller.snapshot(&next.start.token).unwrap(),
            next.start.snapshot
        );
        assert_eq!(
            controller.cancel(&started.token).unwrap_err().code(),
            "longshot_session_superseded"
        );
        assert!(controller.cancel(&next.start.token).unwrap());
        assert_eq!(gate.active_mode().unwrap(), None);
    }

    #[test]
    fn cancel_matrix_and_take_owner_are_strict() {
        let controller = LongshotController::new();
        let other = LongshotController::new();
        let (other_start, _, _) = begin_direct(&other, panorama(64, 72, 20));
        let fake = other_start.token.clone();
        assert_eq!(
            controller.take_owner(&fake).unwrap_err().code(),
            "longshot_session_missing"
        );
        {
            *controller.slot.lock().unwrap() = ControllerSlot::Starting;
        }
        assert_eq!(
            controller.take_owner(&fake).unwrap_err().code(),
            "longshot_session_busy"
        );
        assert_eq!(
            controller.cancel(&fake).unwrap_err().code(),
            "longshot_session_busy"
        );
        controller.rollback_starting().unwrap();

        let (started, _, gate) = begin_direct(&controller, panorama(64, 72, 17));
        assert_eq!(
            controller.take_owner(&fake).unwrap_err().code(),
            "longshot_session_superseded"
        );
        assert_eq!(
            controller.cancel(&fake).unwrap_err().code(),
            "longshot_session_superseded"
        );
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Longshot));
        assert!(controller.cancel(&started.token).unwrap());
        assert!(!controller.cancel(&started.token).unwrap());
        assert_eq!(gate.active_mode().unwrap(), None);
        assert!(other.cancel(&other_start.token).unwrap());

        let (false_start, _, false_gate) = begin_direct(&controller, panorama(64, 72, 24));
        assert!(controller.manager.cancel(&false_start.token).unwrap());
        assert!(!controller.cancel(&false_start.token).unwrap());
        assert!(controller.is_active().unwrap());
        assert_eq!(
            false_gate.active_mode().unwrap(),
            Some(CaptureMode::Longshot)
        );
        controller
            .take_owner(&false_start.token)
            .unwrap()
            .mode_ownership
            .release()
            .unwrap();
        assert_eq!(false_gate.active_mode().unwrap(), None);
    }

    #[test]
    fn cancel_compensation_error_preserves_primary_and_starting() {
        let gate = Arc::new(CaptureModeGate::new());
        let (capture, selection, _) =
            ordinary_capture(panorama(64, 72, 18), Arc::clone(&gate), vec![], vec![]);
        let candidate = capture.prepare_longshot(&selection).unwrap();
        let controller = LongshotController::new();
        let captured_token = std::cell::RefCell::new(None);
        let error = controller
            .begin_with(
                || Ok(candidate),
                |_| Err(CaptureError::SessionSuperseded),
                |token| {
                    *captured_token.borrow_mut() = Some(token.clone());
                    Err(CaptureError::StateLock("injected cancel".to_string()))
                },
            )
            .unwrap_err();
        assert_eq!(error.code(), "session_superseded");
        assert!(matches!(
            *controller.slot.lock().unwrap(),
            ControllerSlot::Starting
        ));
        assert_eq!(
            controller.begin(&capture, &selection).unwrap_err().code(),
            "longshot_session_busy"
        );
        let token = captured_token.into_inner().expect("补偿必须收到完整 token");
        assert!(controller.manager.cancel(&token).unwrap());
        controller.rollback_starting().unwrap();
        assert!(capture.crop(&selection).is_ok());
        capture
            .finish(&selection.session_id)
            .unwrap()
            .finalize_mode()
            .unwrap();
        assert_eq!(gate.active_mode().unwrap(), None);
    }

    #[test]
    fn drop_is_inert_for_active_mode_ownership() {
        let gate = Arc::new(CaptureModeGate::new());
        {
            let controller = LongshotController::new();
            let (capture, selection, _) =
                ordinary_capture(panorama(64, 72, 19), Arc::clone(&gate), vec![], vec![]);
            let result = controller.begin(&capture, &selection).unwrap();
            drop(result);
            assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Longshot));
        }
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Longshot));
    }
}
