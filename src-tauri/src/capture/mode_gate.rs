//! 截图模式的单槽、代次安全所有权。
//!
//! 该 gate 只做短临界区中的认领与释放；窗口、截图和会话工作由调用方在锁外完成。

use super::CaptureError;
use std::sync::{Arc, Mutex};

/// 共享 gate 管理的截图入口模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureMode {
    Ordinary,
    Longshot,
}

/// 成功认领截图模式后得到的不可复制凭据。
///
/// 字段保持私有，调用方只能把该 lease 借给同一个 gate 进行显式释放。
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CaptureModeLease {
    mode: CaptureMode,
    generation: u64,
}

/// 把 lease 与创建它的 gate 绑定，避免调用方把凭据交给错误的 gate。
///
/// 该所有权不可复制，也不会在 `Drop` 时提前开放 gate；资源恢复完成后必须显式消费释放。
pub(crate) struct CaptureModeOwnership {
    gate: Arc<CaptureModeGate>,
    lease: CaptureModeLease,
}

/// 所有权模式转换失败时返还原凭据，调用方仍可按原生命周期显式收尾。
#[derive(Debug)]
pub(crate) struct CaptureModeTransitionFailure {
    pub(crate) error: CaptureError,
    pub(crate) ownership: CaptureModeOwnership,
}

impl std::fmt::Debug for CaptureModeOwnership {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CaptureModeOwnership")
            .field("lease", &self.lease)
            .finish_non_exhaustive()
    }
}

impl CaptureModeOwnership {
    /// 在同一 gate 临界区内把普通截图所有权移交给长截图。
    ///
    /// 失败时原 ownership 会随错误返还，避免 inert Drop 让 gate 永久保持占用。
    pub(crate) fn into_longshot(self) -> Result<Self, CaptureModeTransitionFailure> {
        match self.gate.transition_to_longshot(&self.lease) {
            Ok(lease) => Ok(Self {
                gate: self.gate,
                lease,
            }),
            Err(error) => Err(CaptureModeTransitionFailure {
                error,
                ownership: self,
            }),
        }
    }

    /// 消费来源绑定所有权。空槽意味着生命周期已被其它终结者取走，不能静默成功。
    pub(crate) fn release(self) -> Result<(), CaptureError> {
        match self.gate.release(&self.lease)? {
            true => Ok(()),
            false => Err(CaptureError::CaptureModeSuperseded),
        }
    }
}

/// 普通截图和长截图共享的原子互斥 gate。
pub(crate) struct CaptureModeGate {
    state: Mutex<GateState>,
}

struct GateState {
    last_generation: u64,
    owner: Option<OwnerRecord>,
}

#[derive(Clone, Copy)]
struct OwnerRecord {
    mode: CaptureMode,
    generation: u64,
}

impl Default for CaptureModeGate {
    fn default() -> Self {
        Self::new()
    }
}

impl CaptureModeGate {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(GateState {
                last_generation: 0,
                owner: None,
            }),
        }
    }

    /// 原子认领一个截图模式。已占用时 Busy 优先于代次耗尽。
    pub(crate) fn try_claim(&self, mode: CaptureMode) -> Result<CaptureModeLease, CaptureError> {
        let mut state = self.state.lock().map_err(CaptureError::state_lock)?;
        if state.owner.is_some() {
            return Err(CaptureError::CaptureModeBusy);
        }
        let generation = state
            .last_generation
            .checked_add(1)
            .ok_or(CaptureError::CaptureModeGenerationExhausted)?;
        state.last_generation = generation;
        state.owner = Some(OwnerRecord { mode, generation });
        Ok(CaptureModeLease { mode, generation })
    }

    /// 原子认领并把 lease 绑定到这一个共享 gate。
    pub(crate) fn try_claim_owned(
        self: &Arc<Self>,
        mode: CaptureMode,
    ) -> Result<CaptureModeOwnership, CaptureError> {
        let lease = self.try_claim(mode)?;
        Ok(CaptureModeOwnership {
            gate: Arc::clone(self),
            lease,
        })
    }

    /// 只允许完整匹配的 Ordinary lease 原子地变为下一代 Longshot lease。
    fn transition_to_longshot(
        &self,
        lease: &CaptureModeLease,
    ) -> Result<CaptureModeLease, CaptureError> {
        let mut state = self.state.lock().map_err(CaptureError::state_lock)?;
        let Some(owner) = state.owner else {
            return Err(CaptureError::CaptureModeSuperseded);
        };
        if lease.mode != CaptureMode::Ordinary
            || owner.mode != CaptureMode::Ordinary
            || owner.generation != lease.generation
        {
            return Err(CaptureError::CaptureModeSuperseded);
        }
        let generation = state
            .last_generation
            .checked_add(1)
            .ok_or(CaptureError::CaptureModeGenerationExhausted)?;
        state.last_generation = generation;
        state.owner = Some(OwnerRecord {
            mode: CaptureMode::Longshot,
            generation,
        });
        Ok(CaptureModeLease {
            mode: CaptureMode::Longshot,
            generation,
        })
    }

    /// 只允许完整匹配的 lease 清除当前 owner。
    pub(crate) fn release(&self, lease: &CaptureModeLease) -> Result<bool, CaptureError> {
        let mut state = self.state.lock().map_err(CaptureError::state_lock)?;
        let Some(owner) = state.owner else {
            return Ok(false);
        };
        if owner.mode != lease.mode || owner.generation != lease.generation {
            return Err(CaptureError::CaptureModeSuperseded);
        }
        state.owner = None;
        Ok(true)
    }

    /// 返回当前占用事实，不泄露内部 owner 或 generation。
    pub(crate) fn active_mode(&self) -> Result<Option<CaptureMode>, CaptureError> {
        let state = self.state.lock().map_err(CaptureError::state_lock)?;
        Ok(state.owner.map(|owner| owner.mode))
    }
}

#[cfg(test)]
impl CaptureModeGate {
    pub(super) fn with_last_generation(last_generation: u64) -> Self {
        Self {
            state: Mutex::new(GateState {
                last_generation,
                owner: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc, Barrier};
    use std::thread;

    fn state_snapshot(gate: &CaptureModeGate) -> (u64, Option<(CaptureMode, u64)>) {
        let state = gate.state.lock().expect("测试读取 gate state");
        (
            state.last_generation,
            state.owner.map(|owner| (owner.mode, owner.generation)),
        )
    }

    #[test]
    fn capture_domain_reexports_keep_the_gate_available_to_next_slice() {
        let gate = crate::capture::CaptureModeGate::new();
        let lease: crate::capture::CaptureModeLease = gate
            .try_claim(crate::capture::CaptureMode::Ordinary)
            .expect("受限重导出应可供截图领域使用");
        assert!(gate.release(&lease).expect("重导出的 lease 可释放"));
    }

    #[test]
    fn claim_release_matrix_reports_the_exact_active_mode() {
        for mode in [CaptureMode::Ordinary, CaptureMode::Longshot] {
            let gate = CaptureModeGate::new();
            assert_eq!(gate.active_mode().expect("读取空 gate"), None);

            let lease = gate.try_claim(mode).expect("空 gate 应可认领");
            assert_eq!(gate.active_mode().expect("读取 owner"), Some(mode));
            assert!(gate.release(&lease).expect("匹配 lease 应可释放"));
            assert_eq!(gate.active_mode().expect("释放后读取"), None);
        }
    }

    #[test]
    fn either_mode_is_busy_without_replacing_the_owner() {
        for owner_mode in [CaptureMode::Ordinary, CaptureMode::Longshot] {
            let gate = CaptureModeGate::new();
            let lease = gate.try_claim(owner_mode).expect("首次认领");

            for requested_mode in [CaptureMode::Ordinary, CaptureMode::Longshot] {
                let error = gate
                    .try_claim(requested_mode)
                    .expect_err("任何第二个认领都应 busy");
                assert_eq!(error.code(), "capture_mode_busy");
                assert_eq!(
                    gate.active_mode().expect("失败后 owner 不变"),
                    Some(owner_mode)
                );
            }
            assert!(gate.release(&lease).expect("原 owner 仍能释放"));
        }
    }

    #[test]
    fn empty_and_repeated_release_are_idempotent() {
        let gate = CaptureModeGate::new();
        let lease = gate.try_claim(CaptureMode::Ordinary).expect("首次认领");

        assert!(gate.release(&lease).expect("首次释放"));
        assert!(!gate.release(&lease).expect("重复释放"));
        assert_eq!(gate.active_mode().expect("重复释放后读取"), None);
    }

    #[test]
    fn stale_lease_cannot_clear_a_new_owner_of_any_mode() {
        for new_mode in [CaptureMode::Ordinary, CaptureMode::Longshot] {
            let gate = CaptureModeGate::new();
            let old = gate.try_claim(CaptureMode::Ordinary).expect("旧 owner");
            assert!(gate.release(&old).expect("释放旧 owner"));
            let current = gate.try_claim(new_mode).expect("新 owner");

            let error = gate.release(&old).expect_err("旧 lease 不能清除新 owner");
            assert_eq!(error.code(), "capture_mode_superseded");
            assert_eq!(
                gate.active_mode().expect("ABA 后 owner 仍在"),
                Some(new_mode)
            );
            assert!(gate.release(&current).expect("新 lease 仍可释放"));
        }
    }

    #[test]
    fn failed_operations_do_not_advance_or_replace_generation() {
        let gate = CaptureModeGate::new();
        let first = gate.try_claim(CaptureMode::Ordinary).expect("首次认领");
        assert_eq!(first.generation, 1);
        assert_eq!(
            gate.try_claim(CaptureMode::Longshot)
                .expect_err("占用时失败")
                .code(),
            "capture_mode_busy"
        );
        assert_eq!(gate.state.lock().expect("检查 state").last_generation, 1);
        assert!(gate.release(&first).expect("释放首个 lease"));

        let second = gate.try_claim(CaptureMode::Longshot).expect("第二次认领");
        assert_eq!(second.generation, 2);
        assert_eq!(
            gate.release(&first).expect_err("迟到 release 失败").code(),
            "capture_mode_superseded"
        );
        assert_eq!(
            gate.state.lock().expect("失败不推进代次").last_generation,
            2
        );
        assert_eq!(
            gate.active_mode().expect("失败不替换 owner"),
            Some(CaptureMode::Longshot)
        );
    }

    #[test]
    fn generation_never_wraps_and_busy_wins_at_the_maximum() {
        let gate = CaptureModeGate::with_last_generation(u64::MAX - 1);
        let last = gate
            .try_claim(CaptureMode::Ordinary)
            .expect("MAX 仍可认领一次");
        assert_eq!(last.generation, u64::MAX);
        assert_eq!(
            gate.try_claim(CaptureMode::Longshot)
                .expect_err("MAX 占用时 Busy 优先")
                .code(),
            "capture_mode_busy"
        );
        assert_eq!(
            gate.state.lock().expect("Busy 不改代次").last_generation,
            u64::MAX
        );
        assert!(gate.release(&last).expect("释放 MAX lease"));
        assert_eq!(
            gate.try_claim(CaptureMode::Longshot)
                .expect_err("空槽 MAX 必须拒绝")
                .code(),
            "capture_mode_generation_exhausted"
        );
        assert_eq!(gate.active_mode().expect("耗尽不写 owner"), None);
    }

    #[test]
    fn dropping_a_lease_does_not_release_the_gate() {
        let gate = CaptureModeGate::new();
        let lease = gate.try_claim(CaptureMode::Ordinary).expect("首次认领");
        drop(lease);

        assert_eq!(
            gate.try_claim(CaptureMode::Longshot)
                .expect_err("Drop 不应释放")
                .code(),
            "capture_mode_busy"
        );
        assert_eq!(
            gate.active_mode().expect("Drop 后 owner 仍在"),
            Some(CaptureMode::Ordinary)
        );
    }

    #[test]
    fn owned_claim_releases_only_its_source_gate_and_drop_is_inert() {
        let first = Arc::new(CaptureModeGate::new());
        let second = Arc::new(CaptureModeGate::new());
        let first_owner = first
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("第一 gate 可认领");
        let second_owner = second
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("第二 gate 可独立认领");

        first_owner.release().expect("只释放来源 gate");
        assert_eq!(first.active_mode().unwrap(), None);
        assert_eq!(second.active_mode().unwrap(), Some(CaptureMode::Ordinary));
        drop(second_owner);
        assert_eq!(second.active_mode().unwrap(), Some(CaptureMode::Ordinary));
    }

    #[test]
    fn owned_release_normalizes_an_already_empty_slot_to_superseded() {
        let gate = Arc::new(CaptureModeGate::new());
        let ownership = gate
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("应取得绑定所有权");
        assert!(gate.release(&ownership.lease).expect("测试预先清槽"));

        assert_eq!(
            ownership
                .release()
                .expect_err("重复终结不能静默成功")
                .code(),
            "capture_mode_superseded"
        );
    }

    #[test]
    fn ordinary_ownership_transitions_to_next_generation_longshot() {
        let gate = Arc::new(CaptureModeGate::new());
        let ordinary = gate
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("空 gate 应可认领普通截图");
        assert_eq!(ordinary.lease.generation, 1);

        let longshot = ordinary
            .into_longshot()
            .expect("普通截图所有权应可原子转换");
        assert_eq!(longshot.lease.mode, CaptureMode::Longshot);
        assert_eq!(longshot.lease.generation, 2);
        assert_eq!(state_snapshot(&gate), (2, Some((CaptureMode::Longshot, 2))));
        longshot.release().expect("转换后的 ownership 应可释放");
        assert_eq!(gate.active_mode().expect("释放后读取"), None);
    }

    #[test]
    fn transition_invalidates_the_late_ordinary_lease() {
        let gate = Arc::new(CaptureModeGate::new());
        let ordinary = gate
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("先认领普通截图");
        let late_ordinary = CaptureModeLease {
            mode: ordinary.lease.mode,
            generation: ordinary.lease.generation,
        };
        let longshot = ordinary.into_longshot().expect("转换成功");
        let snapshot = state_snapshot(&gate);

        assert_eq!(
            gate.release(&late_ordinary)
                .expect_err("旧 Ordinary lease 不可释放 Longshot owner")
                .code(),
            "capture_mode_superseded"
        );
        assert_eq!(state_snapshot(&gate), snapshot);
        longshot.release().expect("当前 Longshot owner 应可释放");
    }

    #[test]
    fn exhausted_transition_returns_releasable_ordinary_ownership_without_mutation() {
        let gate = Arc::new(CaptureModeGate::with_last_generation(u64::MAX - 1));
        let ordinary = gate
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("MAX 代次仍可认领普通截图");
        assert_eq!(ordinary.lease.generation, u64::MAX);
        let before = state_snapshot(&gate);

        let CaptureModeTransitionFailure { error, ownership } =
            ordinary.into_longshot().expect_err("MAX 代次不得回绕转换");
        assert_eq!(error.code(), "capture_mode_generation_exhausted");
        assert_eq!(ownership.lease.mode, CaptureMode::Ordinary);
        assert_eq!(ownership.lease.generation, u64::MAX);
        assert_eq!(state_snapshot(&gate), before);
        ownership
            .release()
            .expect("返还的 Ordinary ownership 仍应可释放");
        assert_eq!(gate.active_mode().expect("释放后读取"), None);
    }

    #[test]
    fn stale_transition_returns_ownership_without_changing_the_replacement_owner() {
        let gate = Arc::new(CaptureModeGate::new());
        let ordinary = gate
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("先认领普通截图");
        assert!(gate.release(&ordinary.lease).expect("测试预先清空 owner"));
        let replacement = gate
            .try_claim(CaptureMode::Longshot)
            .expect("空槽可建立新的 owner");
        let before = state_snapshot(&gate);

        let CaptureModeTransitionFailure { error, ownership } = ordinary
            .into_longshot()
            .expect_err("已失效 ownership 不得转换");
        assert_eq!(error.code(), "capture_mode_superseded");
        assert_eq!(state_snapshot(&gate), before);
        assert_eq!(
            ownership
                .release()
                .expect_err("stale ownership 不得伤害新 owner")
                .code(),
            "capture_mode_superseded"
        );
        assert_eq!(state_snapshot(&gate), before);
        assert!(gate.release(&replacement).expect("替换 owner 仍可释放"));
    }

    #[test]
    fn empty_gate_transition_returns_the_original_ownership_without_mutation() {
        let gate = Arc::new(CaptureModeGate::new());
        let ordinary = gate
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("先认领普通截图");
        assert!(gate.release(&ordinary.lease).expect("测试预先清空 owner"));
        let before = state_snapshot(&gate);

        let CaptureModeTransitionFailure { error, ownership } = ordinary
            .into_longshot()
            .expect_err("空 gate 不得伪造转换成功");
        assert_eq!(error.code(), "capture_mode_superseded");
        assert_eq!(state_snapshot(&gate), before);
        assert_eq!(
            ownership
                .release()
                .expect_err("返还的空槽 ownership 不能静默成功")
                .code(),
            "capture_mode_superseded"
        );
        assert_eq!(state_snapshot(&gate), before);
    }

    #[test]
    fn longshot_ownership_cannot_use_the_ordinary_only_transition() {
        let gate = Arc::new(CaptureModeGate::new());
        let longshot = gate
            .try_claim_owned(CaptureMode::Longshot)
            .expect("先认领长截图模式");
        let before = state_snapshot(&gate);

        let failure: crate::capture::CaptureModeTransitionFailure = longshot
            .into_longshot()
            .expect_err("Longshot ownership 不属于 Ordinary 转换来源");
        let CaptureModeTransitionFailure { error, ownership } = failure;
        assert_eq!(error.code(), "capture_mode_superseded");
        assert_eq!(state_snapshot(&gate), before);
        ownership
            .release()
            .expect("返还的原 Longshot ownership 仍可释放");
    }

    #[test]
    fn transition_only_changes_its_source_gate() {
        let first = Arc::new(CaptureModeGate::new());
        let second = Arc::new(CaptureModeGate::new());
        let first_ordinary = first
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("第一 gate 可认领普通截图");
        let second_owner = second
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("第二 gate 可独立认领");
        let second_before = state_snapshot(&second);

        let first_longshot = first_ordinary.into_longshot().expect("第一 gate 可转换");
        assert_eq!(
            first.active_mode().expect("读取第一 gate"),
            Some(CaptureMode::Longshot)
        );
        assert_eq!(state_snapshot(&second), second_before);
        first_longshot.release().expect("只释放第一 gate");
        assert_eq!(state_snapshot(&second), second_before);
        second_owner.release().expect("第二 gate 独立释放");
    }

    #[test]
    fn poisoned_transition_returns_the_original_ordinary_ownership() {
        let gate = Arc::new(CaptureModeGate::new());
        let ordinary = gate
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("先取得普通截图 ownership");
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = gate.state.lock().expect("先锁住 gate");
            panic!("仅用于制造 poison");
        }));

        let CaptureModeTransitionFailure { error, ownership } = ordinary
            .into_longshot()
            .expect_err("poison 后不得伪造转换成功");
        assert_eq!(error.code(), "state_lock");
        assert_eq!(ownership.lease.mode, CaptureMode::Ordinary);
        assert_eq!(ownership.lease.generation, 1);
        assert_eq!(
            ownership
                .release()
                .expect_err("poison 后返还 ownership 仍无法释放")
                .code(),
            "state_lock"
        );
    }

    #[test]
    fn concurrent_claimants_observe_busy_during_the_atomic_transition() {
        const CLAIMANTS: usize = 8;
        let gate = Arc::new(CaptureModeGate::new());
        let ordinary = gate
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("先认领普通截图");
        let barrier = Arc::new(Barrier::new(CLAIMANTS + 1));
        let (sender, receiver) = mpsc::channel();
        let mut workers = Vec::with_capacity(CLAIMANTS);

        for index in 0..CLAIMANTS {
            let gate = Arc::clone(&gate);
            let barrier = Arc::clone(&barrier);
            let sender = sender.clone();
            workers.push(thread::spawn(move || {
                barrier.wait();
                sender
                    .send(gate.try_claim(if index % 2 == 0 {
                        CaptureMode::Ordinary
                    } else {
                        CaptureMode::Longshot
                    }))
                    .expect("主线程接收结果");
            }));
        }
        drop(sender);

        barrier.wait();
        let longshot = ordinary.into_longshot().expect("转换应成功");
        for _ in 0..CLAIMANTS {
            assert_eq!(
                receiver
                    .recv()
                    .expect("每个竞争者都应回报")
                    .expect_err("转换前后 gate 都不可被第二个 claimant 认领")
                    .code(),
                "capture_mode_busy"
            );
        }
        for worker in workers {
            worker.join().expect("竞争线程不应 panic");
        }
        assert_eq!(
            gate.active_mode().expect("所有 claimant 完成后读取"),
            Some(CaptureMode::Longshot)
        );
        longshot.release().expect("转换结果最终释放 gate");
        assert!(gate.try_claim(CaptureMode::Ordinary).is_ok());
    }

    #[test]
    fn dropping_transition_results_keeps_the_gate_occupied() {
        let gate = Arc::new(CaptureModeGate::new());
        let longshot = gate
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("先认领普通截图")
            .into_longshot()
            .expect("转换成功");
        let longshot_lease = CaptureModeLease {
            mode: longshot.lease.mode,
            generation: longshot.lease.generation,
        };
        drop(longshot);
        assert_eq!(
            gate.active_mode().expect("Drop 后读取"),
            Some(CaptureMode::Longshot)
        );
        assert!(gate
            .release(&longshot_lease)
            .expect("测试收尾 Longshot owner"));

        let exhausted_gate = Arc::new(CaptureModeGate::with_last_generation(u64::MAX - 1));
        let exhausted_ordinary = exhausted_gate
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("MAX 代次仍可认领一次");
        let ordinary_lease = CaptureModeLease {
            mode: exhausted_ordinary.lease.mode,
            generation: exhausted_ordinary.lease.generation,
        };
        let CaptureModeTransitionFailure { ownership, .. } = exhausted_ordinary
            .into_longshot()
            .expect_err("MAX 代次转换失败");
        drop(ownership);
        assert_eq!(
            exhausted_gate
                .active_mode()
                .expect("返还 ownership Drop 后读取"),
            Some(CaptureMode::Ordinary)
        );
        assert!(exhausted_gate
            .release(&ordinary_lease)
            .expect("测试收尾 Ordinary owner"));
    }

    #[test]
    fn concurrent_claims_have_exactly_one_winner() {
        const CLAIMANTS: usize = 8;
        let gate = Arc::new(CaptureModeGate::new());
        let barrier = Arc::new(Barrier::new(CLAIMANTS + 1));
        let (sender, receiver) = mpsc::channel();
        let mut workers = Vec::with_capacity(CLAIMANTS);

        for index in 0..CLAIMANTS {
            let gate = Arc::clone(&gate);
            let barrier = Arc::clone(&barrier);
            let sender = sender.clone();
            workers.push(thread::spawn(move || {
                barrier.wait();
                let result = gate.try_claim(if index % 2 == 0 {
                    CaptureMode::Ordinary
                } else {
                    CaptureMode::Longshot
                });
                sender.send(result).expect("主线程接收结果");
            }));
        }
        drop(sender);
        barrier.wait();

        let mut winner = None;
        let mut busy_count = 0;
        for _ in 0..CLAIMANTS {
            match receiver.recv().expect("每个线程都应回报") {
                Ok(lease) => {
                    assert!(winner.replace(lease).is_none(), "只能有一个胜者");
                }
                Err(error) => {
                    assert_eq!(error.code(), "capture_mode_busy");
                    busy_count += 1;
                }
            }
        }
        for worker in workers {
            worker.join().expect("认领线程不应 panic");
        }

        assert_eq!(busy_count, CLAIMANTS - 1);
        let winner = winner.expect("必须恰有一个胜者");
        assert!(gate.release(&winner).expect("胜者 lease 应可释放"));
        assert!(gate.try_claim(CaptureMode::Ordinary).is_ok());
    }

    #[test]
    fn all_apis_report_state_lock_after_mutex_poisoning() {
        let gate = CaptureModeGate::new();
        let lease = gate
            .try_claim(CaptureMode::Ordinary)
            .expect("先取得有效 lease");
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = gate.state.lock().expect("先锁住 gate");
            panic!("仅用于制造 poison");
        }));

        assert_eq!(
            gate.try_claim(CaptureMode::Longshot)
                .expect_err("poison 后认领失败")
                .code(),
            "state_lock"
        );
        assert_eq!(
            gate.release(&lease).expect_err("poison 后释放失败").code(),
            "state_lock"
        );
        assert_eq!(
            gate.active_mode().expect_err("poison 后读取失败").code(),
            "state_lock"
        );
    }
}
