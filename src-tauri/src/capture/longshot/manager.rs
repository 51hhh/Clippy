//! 长截图单槽会话的代次安全管理器。
//!
//! 重的首帧验证、重叠估计、拼接与 PNG 编码都在状态锁外运行。锁内只转移会话的
//! 所有权和比较完整 token，因而取消能立即使正在执行的 lease 失效而不会发生 ABA。

use super::{CaptureError, LongshotAppendOutcome, LongshotSession, LongshotSnapshot};
use image::RgbaImage;
use std::sync::Mutex;

/// 一个逻辑长截图会话的不可伪造认领标识。
///
/// 字段不向 capture 域以外暴露；字符串 id 只供诊断，generation 才是防 ABA 的
/// 单调版本号，所有状态转换必须同时比较两者。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::capture) struct LongshotSessionToken {
    id: String,
    generation: u64,
}

impl LongshotSessionToken {
    pub(super) fn wire_parts(&self) -> (&str, u64) {
        (&self.id, self.generation)
    }

    pub(super) fn from_wire_parts(id: String, generation: u64) -> Self {
        Self { id, generation }
    }
}

/// `begin` 成功后交给调用方的会话标识与首帧快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::capture) struct LongshotStart {
    pub(in crate::capture) token: LongshotSessionToken,
    pub(in crate::capture) snapshot: LongshotSnapshot,
}

/// 同时最多托管一个会话的线程安全生命周期核心。
pub(in crate::capture) struct LongshotManager {
    state: Mutex<ManagerState>,
    id_supplier: fn() -> String,
}

struct ManagerState {
    last_generation: u64,
    slot: Slot,
}

enum Slot {
    Empty,
    Active {
        token: LongshotSessionToken,
        session: LongshotSession,
    },
    InFlight {
        token: LongshotSessionToken,
        operation: Operation,
        committed_snapshot: LongshotSnapshot,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operation {
    Append,
    Finish,
}

struct Lease {
    token: LongshotSessionToken,
    operation: Operation,
    session: LongshotSession,
}

impl Default for LongshotManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LongshotManager {
    pub(in crate::capture) fn new() -> Self {
        Self::with_id_supplier(crate::image_io::unique_image_id)
    }

    fn with_id_supplier(id_supplier: fn() -> String) -> Self {
        Self {
            state: Mutex::new(ManagerState {
                last_generation: 0,
                slot: Slot::Empty,
            }),
            id_supplier,
        }
    }

    /// 锁外构造首帧会话，只有回到空槽的线程才会提交它。
    pub(in crate::capture) fn begin(
        &self,
        first_frame: RgbaImage,
    ) -> Result<LongshotStart, CaptureError> {
        self.preflight_begin()?;
        let session = LongshotSession::start(first_frame)?;
        let snapshot = session.snapshot();
        let id = (self.id_supplier)();

        let token = {
            let mut state = self.state.lock().map_err(CaptureError::state_lock)?;
            if !matches!(state.slot, Slot::Empty) {
                return Err(CaptureError::LongshotSessionBusy);
            }
            let generation = state
                .last_generation
                .checked_add(1)
                .ok_or(CaptureError::LongshotGenerationExhausted)?;
            let token = LongshotSessionToken { id, generation };
            state.last_generation = generation;
            state.slot = Slot::Active {
                token: token.clone(),
                session,
            };
            token
        };

        Ok(LongshotStart { token, snapshot })
    }

    /// 锁外估计和拼接一帧；真实业务错误也会把原会话回交为 Active。
    pub(in crate::capture) fn append(
        &self,
        token: &LongshotSessionToken,
        incoming: RgbaImage,
    ) -> Result<LongshotAppendOutcome, CaptureError> {
        self.append_with(token, move |session| session.append(incoming))
    }

    /// 返回最后一次已提交的快照，不编码或复制会话像素。
    pub(in crate::capture) fn snapshot(
        &self,
        token: &LongshotSessionToken,
    ) -> Result<LongshotSnapshot, CaptureError> {
        let state = self.state.lock().map_err(CaptureError::state_lock)?;
        match &state.slot {
            Slot::Empty => Err(CaptureError::LongshotSessionMissing),
            Slot::Active {
                token: current,
                session,
            } if current == token => Ok(session.snapshot()),
            Slot::InFlight {
                token: current,
                committed_snapshot,
                ..
            } if current == token => Ok(*committed_snapshot),
            Slot::Active { .. } | Slot::InFlight { .. } => {
                Err(CaptureError::LongshotSessionSuperseded)
            }
        }
    }

    /// 锁外编码并在成功后消费会话；编码错误会恢复为 Active 供调用方重试。
    pub(in crate::capture) fn finish_png(
        &self,
        token: &LongshotSessionToken,
    ) -> Result<Vec<u8>, CaptureError> {
        self.finish_with(token, LongshotSession::finish_png)
    }

    /// 使匹配会话立即失效。正在锁外运行的 lease 不会被强杀，但不能再回交。
    pub(in crate::capture) fn cancel(
        &self,
        token: &LongshotSessionToken,
    ) -> Result<bool, CaptureError> {
        let discarded = {
            let mut state = self.state.lock().map_err(CaptureError::state_lock)?;
            match &state.slot {
                Slot::Empty => return Ok(false),
                Slot::Active { token: current, .. } | Slot::InFlight { token: current, .. }
                    if current != token =>
                {
                    return Err(CaptureError::LongshotSessionSuperseded);
                }
                Slot::Active { .. } | Slot::InFlight { .. } => {}
            }

            match std::mem::replace(&mut state.slot, Slot::Empty) {
                Slot::Active { session, .. } => Some(session),
                Slot::InFlight { .. } => None,
                Slot::Empty => unreachable!("已在同一把锁内确认槽位非空"),
            }
        };
        if let Some(session) = discarded {
            session.cancel();
        }
        Ok(true)
    }

    fn preflight_begin(&self) -> Result<(), CaptureError> {
        let state = self.state.lock().map_err(CaptureError::state_lock)?;
        if !matches!(state.slot, Slot::Empty) {
            return Err(CaptureError::LongshotSessionBusy);
        }
        if state.last_generation == u64::MAX {
            return Err(CaptureError::LongshotGenerationExhausted);
        }
        Ok(())
    }

    pub(super) fn append_with<F>(
        &self,
        token: &LongshotSessionToken,
        operation: F,
    ) -> Result<LongshotAppendOutcome, CaptureError>
    where
        F: FnOnce(&mut LongshotSession) -> Result<LongshotAppendOutcome, CaptureError>,
    {
        let mut lease = self.claim(token, Operation::Append)?;
        let result = operation(&mut lease.session);
        self.complete_append(lease, result)
    }

    pub(super) fn finish_with<F>(
        &self,
        token: &LongshotSessionToken,
        operation: F,
    ) -> Result<Vec<u8>, CaptureError>
    where
        F: FnOnce(&LongshotSession) -> Result<Vec<u8>, CaptureError>,
    {
        let lease = self.claim(token, Operation::Finish)?;
        let result = operation(&lease.session);
        self.complete_finish(lease, result)
    }

    fn claim(
        &self,
        token: &LongshotSessionToken,
        operation: Operation,
    ) -> Result<Lease, CaptureError> {
        let mut state = self.state.lock().map_err(CaptureError::state_lock)?;
        match &state.slot {
            Slot::Empty => return Err(CaptureError::LongshotSessionMissing),
            Slot::InFlight { token: current, .. } if current == token => {
                return Err(CaptureError::LongshotSessionBusy);
            }
            Slot::Active { token: current, .. } | Slot::InFlight { token: current, .. }
                if current != token =>
            {
                return Err(CaptureError::LongshotSessionSuperseded);
            }
            Slot::Active { .. } => {}
            Slot::InFlight { .. } => unreachable!("token 比较已覆盖所有 InFlight 分支"),
        }

        let previous = std::mem::replace(&mut state.slot, Slot::Empty);
        let Slot::Active { token, session } = previous else {
            unreachable!("同一把锁内已经确认 Active")
        };
        let committed_snapshot = session.snapshot();
        state.slot = Slot::InFlight {
            token: token.clone(),
            operation,
            committed_snapshot,
        };
        Ok(Lease {
            token,
            operation,
            session,
        })
    }

    fn complete_append(
        &self,
        lease: Lease,
        result: Result<LongshotAppendOutcome, CaptureError>,
    ) -> Result<LongshotAppendOutcome, CaptureError> {
        let Lease {
            token,
            operation,
            session,
        } = lease;
        let mut state = self.state.lock().map_err(CaptureError::state_lock)?;
        if operation == Operation::Append && in_flight_matches(&state.slot, &token, operation) {
            state.slot = Slot::Active { token, session };
            drop(state);
            result
        } else {
            drop(state);
            session.cancel();
            drop(result);
            Err(CaptureError::LongshotSessionSuperseded)
        }
    }

    fn complete_finish(
        &self,
        lease: Lease,
        result: Result<Vec<u8>, CaptureError>,
    ) -> Result<Vec<u8>, CaptureError> {
        let Lease {
            token,
            operation,
            session,
        } = lease;
        let mut state = self.state.lock().map_err(CaptureError::state_lock)?;
        if operation != Operation::Finish || !in_flight_matches(&state.slot, &token, operation) {
            drop(state);
            session.cancel();
            drop(result);
            return Err(CaptureError::LongshotSessionSuperseded);
        }

        if result.is_ok() {
            state.slot = Slot::Empty;
            drop(state);
            session.cancel();
            result
        } else {
            state.slot = Slot::Active { token, session };
            drop(state);
            result
        }
    }
}

fn in_flight_matches(slot: &Slot, token: &LongshotSessionToken, operation: Operation) -> bool {
    matches!(
        slot,
        Slot::InFlight {
            token: current,
            operation: current_operation,
            ..
        } if current == token && *current_operation == operation
    )
}

#[cfg(test)]
impl LongshotManager {
    pub(super) fn with_test_state(id_supplier: fn() -> String, last_generation: u64) -> Self {
        Self {
            state: Mutex::new(ManagerState {
                last_generation,
                slot: Slot::Empty,
            }),
            id_supplier,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::imageops;
    use std::sync::{Arc, Barrier};

    fn repeated_id() -> String {
        "repeated-id".to_string()
    }

    fn panorama(width: u32, height: u32, seed: u32) -> RgbaImage {
        let mut bytes = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let mut value = seed
                    ^ x.wrapping_mul(0x9e37_79b9)
                    ^ y.wrapping_mul(0x85eb_ca6b)
                    ^ (x ^ y).wrapping_mul(0xc2b2_ae35);
                value ^= value >> 16;
                value = value.wrapping_mul(0x7feb_352d);
                value ^= value >> 15;
                value = value.wrapping_mul(0x846c_a68b);
                value ^= value >> 16;
                bytes.extend_from_slice(&[
                    value as u8,
                    (value >> 8) as u8,
                    (value >> 16) as u8,
                    (value >> 24) as u8,
                ]);
            }
        }
        RgbaImage::from_raw(width, height, bytes).expect("测试图像尺寸固定")
    }

    fn frame(panorama: &RgbaImage, top: u32, height: u32) -> RgbaImage {
        imageops::crop_imm(panorama, 0, top, panorama.width(), height).to_image()
    }

    fn start(manager: &LongshotManager) -> (LongshotStart, RgbaImage) {
        let source = panorama(64, 128, 0x1234_5678);
        let initial = manager
            .begin(frame(&source, 0, 72))
            .expect("有效首帧应可创建");
        (initial, source)
    }

    fn wrong(token: &LongshotSessionToken) -> LongshotSessionToken {
        LongshotSessionToken {
            id: format!("wrong-{}", token.id),
            generation: token.generation,
        }
    }

    #[test]
    fn begin_validates_and_preserves_active_session_when_busy() {
        let manager = LongshotManager::with_id_supplier(repeated_id);
        assert!(matches!(
            manager.begin(RgbaImage::new(0, 72)),
            Err(CaptureError::LongshotFrameEmpty)
        ));
        let (started, source) = start(&manager);
        assert_eq!(started.snapshot.frame_count, 1);
        assert!(matches!(
            manager.begin(frame(&source, 24, 72)),
            Err(CaptureError::LongshotSessionBusy)
        ));
        assert_eq!(
            manager
                .snapshot(&started.token)
                .expect("活跃会话快照应可读"),
            started.snapshot
        );
        assert!(manager.cancel(&started.token).expect("活跃会话可取消"));
        assert!(!manager.cancel(&started.token).expect("空槽取消应幂等"));
        assert!(matches!(
            manager.snapshot(&started.token),
            Err(CaptureError::LongshotSessionMissing)
        ));
    }

    #[test]
    fn append_commits_and_business_error_restores_session() {
        let manager = LongshotManager::with_id_supplier(repeated_id);
        let (started, source) = start(&manager);
        let before_lease = manager
            .claim(&started.token, Operation::Finish)
            .expect("首帧应可认领以物化基线 PNG");
        assert!(matches!(
            manager.append(&started.token, frame(&source, 24, 72)),
            Err(CaptureError::LongshotSessionBusy)
        ));
        assert!(matches!(
            manager.finish_png(&started.token),
            Err(CaptureError::LongshotSessionBusy)
        ));
        let before_png = before_lease.session.finish_png().expect("首帧应可物化 PNG");
        assert!(matches!(
            manager.complete_finish(
                before_lease,
                Err(CaptureError::Codec("restore-before-append".into()))
            ),
            Err(CaptureError::Codec(message)) if message == "restore-before-append"
        ));
        assert!(matches!(
            manager.append(
                &started.token,
                RgbaImage::from_pixel(64, 72, image::Rgba([7, 7, 7, 255]))
            ),
            Err(CaptureError::LongshotEstimateLowTexture)
        ));
        assert_eq!(
            manager
                .snapshot(&started.token)
                .expect("业务错误后快照应可读"),
            started.snapshot
        );
        let after_lease = manager
            .claim(&started.token, Operation::Finish)
            .expect("业务错误后应可再次物化 PNG");
        let after_png = after_lease
            .session
            .finish_png()
            .expect("业务错误后 PNG 应可物化");
        assert!(matches!(
            manager.complete_finish(
                after_lease,
                Err(CaptureError::Codec("restore-after-append".into()))
            ),
            Err(CaptureError::Codec(message)) if message == "restore-after-append"
        ));
        assert_eq!(after_png, before_png, "业务错误不能改变已提交 PNG");

        let outcome = manager
            .append(&started.token, frame(&source, 24, 72))
            .expect("失败后可真实重试");
        assert_eq!(outcome.snapshot.frame_count, 2);
        assert_eq!(outcome.snapshot.total_height, 96);
    }

    #[test]
    fn finish_error_restores_active_and_real_finish_consumes_session() {
        let manager = LongshotManager::with_id_supplier(repeated_id);
        let (started, _) = start(&manager);
        assert!(matches!(
            manager.finish_with(&started.token, |_| Err(CaptureError::Codec("injected".into()))),
            Err(CaptureError::Codec(message)) if message == "injected"
        ));
        assert_eq!(
            manager
                .snapshot(&started.token)
                .expect("编码错误后快照应可读"),
            started.snapshot
        );
        let png = manager.finish_png(&started.token).expect("恢复后应可完成");
        assert_eq!(
            crate::screenshot::validate_png(&png).expect("PNG 应可验证"),
            (64, 72)
        );
        assert!(matches!(
            manager.snapshot(&started.token),
            Err(CaptureError::LongshotSessionMissing)
        ));
    }

    #[test]
    fn in_flight_keeps_snapshot_readable_and_rejects_second_mutation() {
        let manager = LongshotManager::with_id_supplier(repeated_id);
        let (started, source) = start(&manager);
        let lease = manager
            .claim(&started.token, Operation::Append)
            .expect("首个 append 应认领成功");
        assert_eq!(
            manager
                .snapshot(&started.token)
                .expect("in-flight 快照应可读"),
            started.snapshot
        );
        assert!(matches!(
            manager.append(&started.token, frame(&source, 24, 72)),
            Err(CaptureError::LongshotSessionBusy)
        ));
        assert!(matches!(
            manager.finish_png(&started.token),
            Err(CaptureError::LongshotSessionBusy)
        ));
        assert!(manager
            .cancel(&started.token)
            .expect("in-flight 可立即取消"));
        assert!(matches!(
            manager.complete_append(lease, Err(CaptureError::LongshotEstimateLowTexture)),
            Err(CaptureError::LongshotSessionSuperseded)
        ));
        assert!(!manager.cancel(&started.token).expect("空槽取消应幂等"));
    }

    #[test]
    fn stale_append_cannot_overwrite_new_generation_even_with_repeated_id() {
        let manager = LongshotManager::with_id_supplier(repeated_id);
        let (old, source) = start(&manager);
        let mut lease = manager
            .claim(&old.token, Operation::Append)
            .expect("旧 append 应可认领");
        assert!(manager.cancel(&old.token).expect("旧 lease 可取消"));
        let new = manager
            .begin(frame(&source, 0, 72))
            .expect("取消后新会话应可开始");
        assert_eq!(old.token.id, new.token.id);
        assert_eq!(new.token.generation, old.token.generation + 1);
        let old_result = lease.session.append(frame(&source, 24, 72));
        assert!(matches!(
            manager.complete_append(lease, old_result),
            Err(CaptureError::LongshotSessionSuperseded)
        ));
        assert_eq!(
            manager.snapshot(&new.token).expect("新会话快照应可读"),
            new.snapshot
        );
        manager
            .append(&new.token, frame(&source, 24, 72))
            .expect("新会话不能被旧回交破坏");
    }

    #[test]
    fn stale_finish_cannot_consume_new_generation() {
        let manager = LongshotManager::with_id_supplier(repeated_id);
        let (old, source) = start(&manager);
        let lease = manager
            .claim(&old.token, Operation::Finish)
            .expect("旧 finish 应可认领");
        assert!(manager.cancel(&old.token).expect("旧 finish 可取消"));
        let new = manager
            .begin(frame(&source, 0, 72))
            .expect("取消后新会话应可开始");
        let old_png = lease.session.finish_png();
        assert!(matches!(
            manager.complete_finish(lease, old_png),
            Err(CaptureError::LongshotSessionSuperseded)
        ));
        assert_eq!(
            manager.snapshot(&new.token).expect("新会话快照应可读"),
            new.snapshot
        );
        assert!(manager.finish_png(&new.token).is_ok());
    }

    #[test]
    fn invalid_tokens_never_consume_active_or_in_flight_sessions() {
        let manager = LongshotManager::with_id_supplier(repeated_id);
        let (started, source) = start(&manager);
        let invalid = wrong(&started.token);
        assert!(matches!(
            manager.append(&invalid, frame(&source, 24, 72)),
            Err(CaptureError::LongshotSessionSuperseded)
        ));
        assert!(matches!(
            manager.finish_png(&invalid),
            Err(CaptureError::LongshotSessionSuperseded)
        ));
        assert!(matches!(
            manager.snapshot(&invalid),
            Err(CaptureError::LongshotSessionSuperseded)
        ));
        assert!(matches!(
            manager.cancel(&invalid),
            Err(CaptureError::LongshotSessionSuperseded)
        ));
        let mut lease = manager
            .claim(&started.token, Operation::Append)
            .expect("真实 token 可认领 append");
        assert!(matches!(
            manager.append(&invalid, frame(&source, 24, 72)),
            Err(CaptureError::LongshotSessionSuperseded)
        ));
        assert!(matches!(
            manager.finish_png(&invalid),
            Err(CaptureError::LongshotSessionSuperseded)
        ));
        assert!(matches!(
            manager.snapshot(&invalid),
            Err(CaptureError::LongshotSessionSuperseded)
        ));
        assert!(matches!(
            manager.cancel(&invalid),
            Err(CaptureError::LongshotSessionSuperseded)
        ));
        let result = lease.session.append(frame(&source, 24, 72));
        assert!(manager.complete_append(lease, result).is_ok());

        let lease = manager
            .claim(&started.token, Operation::Finish)
            .expect("真实 token 可认领 finish");
        assert!(matches!(
            manager.append(&invalid, frame(&source, 48, 72)),
            Err(CaptureError::LongshotSessionSuperseded)
        ));
        assert!(matches!(
            manager.finish_png(&invalid),
            Err(CaptureError::LongshotSessionSuperseded)
        ));
        assert!(matches!(
            manager.snapshot(&invalid),
            Err(CaptureError::LongshotSessionSuperseded)
        ));
        assert!(matches!(
            manager.cancel(&invalid),
            Err(CaptureError::LongshotSessionSuperseded)
        ));
        let result = lease.session.finish_png();
        assert!(manager.complete_finish(lease, result).is_ok());
        assert!(matches!(
            manager.begin(frame(&source, 0, 72)),
            Ok(LongshotStart { .. })
        ));
    }

    #[test]
    fn concurrent_begins_have_one_commit_and_one_busy_result() {
        let manager = Arc::new(LongshotManager::with_id_supplier(repeated_id));
        let barrier = Arc::new(Barrier::new(3));
        let source = panorama(64, 72, 4);
        let handles = (0..2)
            .map(|_| {
                let manager = Arc::clone(&manager);
                let barrier = Arc::clone(&barrier);
                let image = source.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    manager.begin(image)
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().expect("begin 线程不应 panic"))
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(CaptureError::LongshotSessionBusy)))
                .count(),
            1
        );
    }

    #[test]
    fn generation_exhaustion_and_poison_are_structured_errors() {
        let exhausted = LongshotManager::with_test_state(repeated_id, u64::MAX);
        assert!(matches!(
            exhausted.begin(panorama(64, 72, 9)),
            Err(CaptureError::LongshotGenerationExhausted)
        ));

        let manager = LongshotManager::with_id_supplier(repeated_id);
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = manager.state.lock().expect("新 mutex 应可锁定");
            panic!("刻意 poison manager 状态锁");
        }));
        assert!(poisoned.is_err());
        let token = LongshotSessionToken {
            id: "poisoned".to_string(),
            generation: 1,
        };
        assert!(matches!(
            manager.begin(panorama(64, 72, 10)),
            Err(CaptureError::StateLock(_))
        ));
        assert!(matches!(
            manager.append(&token, panorama(64, 72, 11)),
            Err(CaptureError::StateLock(_))
        ));
        assert!(matches!(
            manager.snapshot(&token),
            Err(CaptureError::StateLock(_))
        ));
        assert!(matches!(
            manager.finish_png(&token),
            Err(CaptureError::StateLock(_))
        ));
        assert!(matches!(
            manager.cancel(&token),
            Err(CaptureError::StateLock(_))
        ));
    }
}
