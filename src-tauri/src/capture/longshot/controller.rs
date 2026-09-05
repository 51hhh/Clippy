//! 长截图帧来源、固定裁剪与代次安全会话之间的单槽所有权层。
//!
//! controller 锁只保护轻量 owner；捕获、裁剪、重叠估计、拼接和 PNG 编码均由
//! manager 的事务 lease 在两把状态锁之外完成。

use super::{
    capture_monitor_frame, LongshotAppendOutcome, LongshotFrameAdapter, LongshotManager,
    LongshotSession, LongshotSessionToken, LongshotSnapshot, LongshotStart,
};
use crate::capture::{CaptureError, CaptureManager, CaptureSelection};
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

struct Owner {
    token: LongshotSessionToken,
    monitor_id: u32,
    adapter: LongshotFrameAdapter,
}

struct OwnerLease {
    monitor_id: u32,
    adapter: LongshotFrameAdapter,
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

    /// 从普通截图的冻结帧启动长截图，但不消费普通截图会话。
    pub(in crate::capture) fn begin(
        &self,
        capture: &CaptureManager,
        selection: &CaptureSelection,
    ) -> Result<LongshotStart, CaptureError> {
        self.begin_with(selection, || capture.selected_frame(selection))
    }

    fn begin_with<F>(
        &self,
        selection: &CaptureSelection,
        frame_supplier: F,
    ) -> Result<LongshotStart, CaptureError>
    where
        F: FnOnce() -> Result<CapturedMonitorFrame, CaptureError>,
    {
        self.claim_starting()?;

        let prepared = (|| {
            let frame = frame_supplier()?;
            let monitor_id = frame.monitor_id;
            let (adapter, first_crop) = LongshotFrameAdapter::from_first(&frame, selection)?;
            let started = self.manager.begin(first_crop)?;
            Ok((started, monitor_id, adapter))
        })();

        let (started, monitor_id, adapter) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.rollback_starting()?;
                return Err(error);
            }
        };

        let mut slot = match self.slot.lock() {
            Ok(slot) => slot,
            Err(error) => {
                let _ = self.manager.cancel(&started.token);
                return Err(CaptureError::state_lock(error));
            }
        };
        if !matches!(*slot, ControllerSlot::Starting) {
            drop(slot);
            let _ = self.manager.cancel(&started.token);
            return Err(CaptureError::LongshotSessionSuperseded);
        }
        *slot = ControllerSlot::Active(Owner {
            token: started.token.clone(),
            monitor_id,
            adapter,
        });
        Ok(started)
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
        self.clear_owner_if(token)?;
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
            self.clear_owner_if(token)?;
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

    fn clear_owner_if(&self, token: &LongshotSessionToken) -> Result<(), CaptureError> {
        let mut slot = self.slot.lock().map_err(CaptureError::state_lock)?;
        if matches!(&*slot, ControllerSlot::Active(owner) if owner.token == *token) {
            *slot = ControllerSlot::Empty;
        }
        Ok(())
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
    use crate::capture::{CaptureMode, CaptureModeGate, CaptureModeOwnership};
    use image::{imageops, RgbaImage};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(3);

    fn ordinary_ownership() -> CaptureModeOwnership {
        Arc::new(CaptureModeGate::new())
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("测试应取得 Ordinary")
    }

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

    fn begin_direct(
        controller: &LongshotController,
        first: RgbaImage,
    ) -> (LongshotStart, CaptureSelection) {
        let selection = selection("direct", 7);
        let started = controller
            .begin_with(&selection, || Ok(captured(first, 7)))
            .expect("有效首帧应启动 controller");
        (started, selection)
    }

    #[test]
    fn selected_frame_reuses_arc_and_preserves_ordinary_session() {
        let capture = CaptureManager::new();
        let original = captured(panorama(64, 72, 1), 7);
        let pixels = Arc::clone(&original.rgba);
        let overlays = capture
            .begin(
                vec![original],
                Vec::new(),
                Vec::new(),
                false,
                StageTimings::default(),
                ordinary_ownership(),
            )
            .expect("普通截图应启动");
        let payload = capture
            .payload(&overlays.overlays[0].label)
            .expect("payload 应存在");
        let selection = selection(&payload.session_id, 7);
        let selected = capture.selected_frame(&selection).expect("应取得冻结帧");
        assert!(Arc::ptr_eq(&pixels, &selected.rgba));
        assert!(capture.crop(&selection).is_ok());
        assert!(capture.finish(&payload.session_id).is_ok());
    }

    #[test]
    fn begin_uses_capture_manager_error_order_and_rolls_back_every_failure() {
        let capture = CaptureManager::new();
        let controller = LongshotController::new();
        let missing = selection("missing", 7);
        assert_eq!(
            controller.begin(&capture, &missing).unwrap_err().code(),
            "session_missing"
        );
        assert!(!controller.is_active().unwrap());

        let overlays = capture
            .begin(
                vec![captured(panorama(64, 72, 2), 7)],
                Vec::new(),
                Vec::new(),
                false,
                StageTimings::default(),
                ordinary_ownership(),
            )
            .unwrap();
        let payload = capture.payload(&overlays.overlays[0].label).unwrap();
        let stale = selection("stale", 7);
        assert_eq!(
            controller.begin(&capture, &stale).unwrap_err().code(),
            "session_superseded_retry"
        );
        let wrong_monitor = selection(&payload.session_id, 9);
        assert_eq!(
            controller
                .begin(&capture, &wrong_monitor)
                .unwrap_err()
                .code(),
            "selection_monitor_mismatch"
        );

        let mut invalid = selection(&payload.session_id, 7);
        invalid.width = f64::NAN;
        assert_eq!(
            controller.begin(&capture, &invalid).unwrap_err().code(),
            "selection_not_finite"
        );
        let mut too_small = selection(&payload.session_id, 7);
        too_small.width = 1.0;
        assert_eq!(
            controller.begin(&capture, &too_small).unwrap_err().code(),
            "selection_too_small"
        );
        let mut empty = selection(&payload.session_id, 7);
        empty.x = 1000.0;
        assert_eq!(
            controller.begin(&capture, &empty).unwrap_err().code(),
            "selection_empty"
        );
        assert!(!controller.is_active().unwrap());
        let valid = selection(&payload.session_id, 7);
        assert_eq!(
            controller
                .begin(&capture, &valid)
                .unwrap()
                .snapshot
                .frame_count,
            1
        );
    }

    #[test]
    fn starting_is_visible_and_rejects_second_supplier_without_calling_it() {
        let controller = Arc::new(LongshotController::new());
        let selection = selection("direct", 7);
        let (entered_tx, entered_rx) = mpsc::sync_channel(0);
        let (release_tx, release_rx) = mpsc::sync_channel(0);
        let worker_controller = Arc::clone(&controller);
        let worker_selection = selection.clone();
        let worker = std::thread::spawn(move || {
            worker_controller.begin_with(&worker_selection, || {
                entered_tx.send(()).expect("主线程应等待进入通知");
                release_rx.recv().expect("主线程应显式释放 supplier");
                Ok(captured(panorama(64, 72, 3), 7))
            })
        });
        entered_rx
            .recv_timeout(WAIT)
            .expect("supplier 应进入锁外阶段");
        assert!(controller.is_active().unwrap());
        let called = AtomicBool::new(false);
        let error = controller
            .begin_with(&selection, || {
                called.store(true, Ordering::SeqCst);
                Ok(captured(panorama(64, 72, 4), 7))
            })
            .unwrap_err();
        assert_eq!(error.code(), "longshot_session_busy");
        assert!(!called.load(Ordering::SeqCst));
        release_tx.send(()).expect("应释放首个 supplier");
        assert!(worker.join().expect("begin 线程不应 panic").is_ok());
    }

    #[test]
    fn begin_supplier_and_frame_failures_rollback_for_retry() {
        let controller = LongshotController::new();
        let selection = selection("direct", 7);
        let calls = std::cell::Cell::new(0);
        let error = controller
            .begin_with(&selection, || {
                calls.set(calls.get() + 1);
                Err(CaptureError::Screenshot("injected".into()))
            })
            .unwrap_err();
        assert_eq!(calls.get(), 1);
        assert_eq!(error.code(), "screenshot");
        assert!(!controller.is_active().unwrap());

        let mut malformed = captured(panorama(64, 72, 10), 7);
        malformed.scale_x = 0.0;
        assert_eq!(
            controller
                .begin_with(&selection, || Ok(malformed))
                .unwrap_err()
                .code(),
            "longshot_frame_invalid"
        );
        let mut truncated = captured(panorama(64, 72, 11), 7);
        truncated.rgba = Arc::from(vec![0; 17]);
        assert_eq!(
            controller
                .begin_with(&selection, || Ok(truncated))
                .unwrap_err()
                .code(),
            "longshot_frame_invalid"
        );
        assert!(!controller.is_active().unwrap());
        assert!(controller
            .begin_with(&selection, || Ok(captured(panorama(64, 72, 12), 7)))
            .is_ok());
    }

    #[test]
    fn append_pipeline_keeps_committed_snapshot_and_retries_after_errors() {
        let controller = LongshotController::new();
        let source = panorama(64, 128, 5);
        let (started, _) = begin_direct(
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
    }

    #[test]
    fn append_crop_and_overlap_errors_preserve_owner_and_committed_state() {
        let controller = LongshotController::new();
        let source = panorama(64, 128, 13);
        let (started, _) = begin_direct(
            &controller,
            imageops::crop_imm(&source, 0, 0, 64, 72).to_image(),
        );
        let baseline = started.snapshot;

        let mut changed = captured(imageops::crop_imm(&source, 0, 24, 64, 72).to_image(), 7);
        changed.x += 1;
        assert_eq!(
            controller
                .append_with(&started.token, |_| Ok(changed))
                .unwrap_err()
                .code(),
            "longshot_frame_geometry_changed"
        );
        assert_eq!(controller.snapshot(&started.token).unwrap(), baseline);

        let mut malformed = captured(panorama(64, 72, 14), 7);
        malformed.rgba = Arc::from(vec![0; 8]);
        assert_eq!(
            controller
                .append_with(&started.token, |_| Ok(malformed))
                .unwrap_err()
                .code(),
            "longshot_frame_invalid"
        );
        assert_eq!(controller.snapshot(&started.token).unwrap(), baseline);

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
        assert_eq!(controller.snapshot(&started.token).unwrap(), baseline);
        assert!(controller
            .append_with(&started.token, |_| {
                Ok(captured(
                    imageops::crop_imm(&source, 0, 24, 64, 72).to_image(),
                    7,
                ))
            })
            .is_ok());
    }

    #[test]
    fn append_claim_precedes_capture_and_cancel_invalidates_late_aba_result() {
        let controller = Arc::new(LongshotController::with_test_state(repeated_id, 0));
        let source = panorama(64, 128, 6);
        let (old, selection) = begin_direct(
            &controller,
            imageops::crop_imm(&source, 0, 0, 64, 72).to_image(),
        );
        let (entered_tx, entered_rx) = mpsc::sync_channel(0);
        let (release_tx, release_rx) = mpsc::sync_channel(0);
        let worker_controller = Arc::clone(&controller);
        let worker_token = old.token.clone();
        let worker_frame = imageops::crop_imm(&source, 0, 24, 64, 72).to_image();
        let worker = std::thread::spawn(move || {
            worker_controller.append_with(&worker_token, |monitor_id| {
                assert_eq!(monitor_id, 7);
                entered_tx.send(()).expect("主线程应等待捕获进入通知");
                release_rx.recv().expect("主线程应显式释放捕获");
                Ok(captured(worker_frame, 7))
            })
        });
        entered_rx.recv_timeout(WAIT).expect("捕获闭包应已进入");
        assert_eq!(controller.snapshot(&old.token).unwrap(), old.snapshot);
        let second_called = AtomicBool::new(false);
        let second = controller
            .append_with(&old.token, |_| {
                second_called.store(true, Ordering::SeqCst);
                Ok(captured(panorama(64, 72, 7), 7))
            })
            .unwrap_err();
        assert_eq!(second.code(), "longshot_session_busy");
        assert!(!second_called.load(Ordering::SeqCst));
        assert!(controller.cancel(&old.token).unwrap());
        let new = controller
            .begin_with(&selection, || {
                Ok(captured(
                    imageops::crop_imm(&source, 0, 0, 64, 72).to_image(),
                    7,
                ))
            })
            .expect("取消后应立即开始新一代");
        assert_ne!(old.token, new.token);
        release_tx.send(()).expect("应释放旧捕获");
        assert_eq!(
            worker
                .join()
                .expect("append 线程不应 panic")
                .unwrap_err()
                .code(),
            "longshot_session_superseded"
        );
        assert_eq!(controller.snapshot(&new.token).unwrap(), new.snapshot);
    }

    #[test]
    fn finish_error_retries_real_png_and_clears_owner() {
        let controller = LongshotController::new();
        let first = panorama(64, 72, 8);
        let expected = first.as_raw().clone();
        let (started, selection) = begin_direct(&controller, first);
        let error = controller
            .finish_with(&started.token, |_| {
                Err(CaptureError::Codec("injected".into()))
            })
            .unwrap_err();
        assert_eq!(error.code(), "codec");
        assert!(controller.is_active().unwrap());
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
        assert_eq!(
            controller.snapshot(&started.token).unwrap_err().code(),
            "longshot_session_missing"
        );
        assert_eq!(controller.cancel(&started.token).unwrap(), false);
        assert!(controller
            .begin_with(&selection, || Ok(captured(panorama(64, 72, 9), 7)))
            .is_ok());
    }

    #[test]
    fn finish_claim_is_lock_free_and_cancel_makes_late_result_superseded() {
        let controller = Arc::new(LongshotController::with_test_state(repeated_id, 0));
        let (started, selection) = begin_direct(&controller, panorama(64, 72, 15));
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
        let next = controller
            .begin_with(&selection, || Ok(captured(panorama(64, 72, 16), 7)))
            .expect("取消 finish 后应可开始新一代");
        release_tx.send(()).expect("应释放旧编码");
        assert_eq!(
            worker
                .join()
                .expect("finish 线程不应 panic")
                .unwrap_err()
                .code(),
            "longshot_session_superseded"
        );
        assert_eq!(controller.snapshot(&next.token).unwrap(), next.snapshot);
        assert_eq!(
            controller.cancel(&started.token).unwrap_err().code(),
            "longshot_session_superseded"
        );
    }

    #[test]
    fn owner_lease_maps_manager_missing_to_superseded_without_invoking_operations() {
        let controller = LongshotController::new();
        let (started, _) = begin_direct(&controller, panorama(64, 72, 17));
        assert!(controller.manager.cancel(&started.token).unwrap());
        assert_eq!(
            controller.snapshot(&started.token).unwrap_err().code(),
            "longshot_session_superseded"
        );
        let provider_called = AtomicBool::new(false);
        assert_eq!(
            controller
                .append_with(&started.token, |_| {
                    provider_called.store(true, Ordering::SeqCst);
                    Ok(captured(panorama(64, 72, 18), 7))
                })
                .unwrap_err()
                .code(),
            "longshot_session_superseded"
        );
        assert!(!provider_called.load(Ordering::SeqCst));
        let finish_called = AtomicBool::new(false);
        assert_eq!(
            controller
                .finish_with(&started.token, |_| {
                    finish_called.store(true, Ordering::SeqCst);
                    Ok(Vec::new())
                })
                .unwrap_err()
                .code(),
            "longshot_session_superseded"
        );
        assert!(!finish_called.load(Ordering::SeqCst));
    }
}
