use std::fmt::Display;

/// 统一按“关闭覆盖层 → 恢复桌面资源 → 最后释放模式”终结会话。
pub(super) fn cleanup_capture_session<S, E, C, T, Z>(
    session: S,
    close_overlays: C,
    restore_sources: T,
    finalize: Z,
) -> Result<(), E>
where
    C: FnOnce(&S),
    T: FnOnce(&S),
    Z: FnOnce(S) -> Result<(), E>,
{
    close_overlays(&session);
    restore_sources(&session);
    finalize(session)
}

/// 用一个精确身份认领会话并完成统一 cleanup；`None` 表示其它终结者已经胜出。
pub(super) fn finish_capture_session<S, E, F, C, T, Z>(
    finish: F,
    close_overlays: C,
    restore_sources: T,
    finalize: Z,
) -> Result<bool, E>
where
    F: FnOnce() -> Result<Option<S>, E>,
    C: FnOnce(&S),
    T: FnOnce(&S),
    Z: FnOnce(S) -> Result<(), E>,
{
    let Some(session) = finish()? else {
        return Ok(false);
    };
    cleanup_capture_session(session, close_overlays, restore_sources, finalize)?;
    Ok(true)
}

/// cancel 只有成功认领精确 session id 后才执行 cleanup；输出中由 worker 最终认领。
pub(super) fn complete_capture_cancel<S, E, F, C, T, Z>(
    finish: F,
    close_overlays: C,
    restore_sources: T,
    finalize: Z,
) -> Result<(), E>
where
    F: FnOnce() -> Result<Option<S>, E>,
    C: FnOnce(&S),
    T: FnOnce(&S),
    Z: FnOnce(S) -> Result<(), E>,
{
    finish_capture_session(finish, close_overlays, restore_sources, finalize)?;
    Ok(())
}

/// build/configure 主错误保持不变；只在精确会话仍可认领时 cleanup。
pub(super) fn complete_overlay_create<S, E, F, C, T, Z>(
    create_result: Result<(), E>,
    finish: F,
    close_overlays: C,
    restore_sources: T,
    finalize: Z,
) -> Result<(), E>
where
    E: Display,
    F: FnOnce() -> Result<Option<S>, E>,
    C: FnOnce(&S),
    T: FnOnce(&S),
    Z: FnOnce(S) -> Result<(), E>,
{
    match create_result {
        Ok(()) => Ok(()),
        Err(primary) => {
            if let Err(cleanup_error) =
                finish_capture_session(finish, close_overlays, restore_sources, finalize)
            {
                log::error!("创建截图覆盖层失败后终结会话也失败: {cleanup_error}");
            }
            Err(primary)
        }
    }
}

/// 原生 show 失败时按 label 精确认领并 cleanup；cleanup 错误只诊断，show 错误保持主错误。
pub(super) fn complete_overlay_reveal<T, E, F>(
    show_result: Result<T, E>,
    terminate: F,
) -> Result<T, E>
where
    E: Display,
    F: FnOnce() -> Result<bool, E>,
{
    match show_result {
        Ok(value) => Ok(value),
        Err(primary) => {
            if let Err(cleanup_error) = terminate() {
                log::error!("显示截图覆盖层失败后终结会话也失败: {cleanup_error}");
            }
            Err(primary)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{CaptureMode, CaptureModeGate, CaptureModeOwnership};
    use std::cell::RefCell;
    use std::sync::Arc;

    #[derive(Debug)]
    struct OwnedSession {
        id: &'static str,
        ownership: CaptureModeOwnership,
    }

    fn owned_session(id: &'static str, gate: &Arc<CaptureModeGate>) -> OwnedSession {
        OwnedSession {
            id,
            ownership: gate
                .try_claim_owned(CaptureMode::Ordinary)
                .expect("测试应取得 Ordinary"),
        }
    }

    #[test]
    fn cleanup_keeps_gate_busy_until_close_and_restore_are_complete() {
        let gate = Arc::new(CaptureModeGate::new());
        let ownership = gate
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("应取得 Ordinary");
        let events = RefCell::new(Vec::new());

        cleanup_capture_session(
            ownership,
            |_| {
                events.borrow_mut().push("close");
                assert_eq!(
                    gate.try_claim(CaptureMode::Longshot).unwrap_err().code(),
                    "capture_mode_busy"
                );
            },
            |_| {
                events.borrow_mut().push("restore");
                assert_eq!(
                    gate.try_claim(CaptureMode::Ordinary).unwrap_err().code(),
                    "capture_mode_busy"
                );
            },
            |ownership| {
                events.borrow_mut().push("finalize");
                ownership.release()
            },
        )
        .expect("终结成功");

        assert_eq!(*events.borrow(), ["close", "restore", "finalize"]);
        let next = gate
            .try_claim_owned(CaptureMode::Ordinary)
            .expect("finalize 后才可再次认领");
        next.release().unwrap();
    }

    #[test]
    fn reveal_failure_terminates_once_and_keeps_show_error_primary() {
        let calls = std::cell::Cell::new(0);
        let error = complete_overlay_reveal(Err::<(), _>("show error"), || {
            calls.set(calls.get() + 1);
            Err("cleanup error")
        })
        .unwrap_err();
        assert_eq!(error, "show error");
        assert_eq!(calls.get(), 1);

        let value = complete_overlay_reveal(Ok::<_, &str>("shown"), || {
            calls.set(calls.get() + 1);
            Ok(true)
        })
        .unwrap();
        assert_eq!(value, "shown");
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn create_error_finishes_exact_session_and_keeps_primary_error() {
        let gate = Arc::new(CaptureModeGate::new());
        let slot = RefCell::new(Some(owned_session("exact", &gate)));
        let events = RefCell::new(Vec::new());

        let result = complete_overlay_create(
            Err::<(), _>("create error"),
            || {
                events.borrow_mut().push("finish:exact");
                Ok(slot.borrow_mut().take())
            },
            |_| events.borrow_mut().push("close"),
            |_| events.borrow_mut().push("restore"),
            |session| {
                events.borrow_mut().push("finalize");
                session.ownership.release().map_err(|_| "finalize error")
            },
        );

        assert_eq!(result.unwrap_err(), "create error");
        assert_eq!(
            *events.borrow(),
            ["finish:exact", "close", "restore", "finalize"]
        );
        assert_eq!(gate.active_mode().unwrap(), None);
    }

    #[test]
    fn late_create_error_does_not_cleanup_a_new_session() {
        let gate = Arc::new(CaptureModeGate::new());
        let current = owned_session("new", &gate);
        let result = complete_overlay_create(
            Err::<(), _>("old create error"),
            || Ok::<Option<OwnedSession>, &str>(None),
            |_| panic!("旧 create 失败不得 close 新 session"),
            |_| panic!("旧 create 失败不得 restore 新 session"),
            |_| -> Result<(), &str> { panic!("旧 create 失败不得 finalize 新 session") },
        );

        assert_eq!(result.unwrap_err(), "old create error");
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Ordinary));
        current.ownership.release().unwrap();
    }

    #[test]
    fn reveal_and_cancel_race_has_one_cleanup_and_show_error_stays_primary() {
        for cancel_first in [true, false] {
            let gate = Arc::new(CaptureModeGate::new());
            let slot = RefCell::new(Some(owned_session("session", &gate)));
            let events = RefCell::new(Vec::new());
            let cleanup = |session: OwnedSession| {
                events.borrow_mut().push("finalize");
                session.ownership.release().map_err(|_| "finalize error")
            };

            if cancel_first {
                complete_capture_cancel(
                    || Ok::<_, &str>(slot.borrow_mut().take()),
                    |_| events.borrow_mut().push("close"),
                    |_| events.borrow_mut().push("restore"),
                    cleanup,
                )
                .unwrap();
            }

            events.borrow_mut().push("show");
            let show_error = complete_overlay_reveal(Err::<(), _>("show error"), || {
                finish_capture_session(
                    || Ok::<_, &str>(slot.borrow_mut().take()),
                    |_| events.borrow_mut().push("close"),
                    |_| events.borrow_mut().push("restore"),
                    cleanup,
                )
            })
            .unwrap_err();
            assert_eq!(show_error, "show error");

            if !cancel_first {
                let cancel = complete_capture_cancel(
                    || slot.borrow_mut().take().ok_or("missing").map(Some),
                    |_| events.borrow_mut().push("unexpected-close"),
                    |_| events.borrow_mut().push("unexpected-restore"),
                    |_| {
                        events.borrow_mut().push("unexpected-finalize");
                        Ok(())
                    },
                );
                assert_eq!(cancel.unwrap_err(), "missing");
            }

            assert_eq!(
                events
                    .borrow()
                    .iter()
                    .filter(|event| **event == "finalize")
                    .count(),
                1
            );
            assert_eq!(gate.active_mode().unwrap(), None);
        }
    }

    #[test]
    fn cancel_old_id_never_cleans_or_releases_the_new_session() {
        let gate = Arc::new(CaptureModeGate::new());
        let slot = RefCell::new(Some(owned_session("new", &gate)));
        let events = RefCell::new(Vec::new());

        let stale = complete_capture_cancel(
            || {
                let slot = slot.borrow();
                if slot.as_ref().is_some_and(|session| session.id == "old") {
                    unreachable!("新 session 不得被旧 id 取走");
                }
                Err::<Option<OwnedSession>, _>("superseded")
            },
            |_| events.borrow_mut().push("close"),
            |_| events.borrow_mut().push("restore"),
            |_| {
                events.borrow_mut().push("finalize");
                Ok(())
            },
        );
        assert_eq!(stale.unwrap_err(), "superseded");
        assert!(events.borrow().is_empty());
        assert_eq!(gate.active_mode().unwrap(), Some(CaptureMode::Ordinary));

        complete_capture_cancel(
            || Ok::<_, &str>(slot.borrow_mut().take()),
            |_| events.borrow_mut().push("close"),
            |_| events.borrow_mut().push("restore"),
            |session| {
                events.borrow_mut().push("finalize");
                session.ownership.release().map_err(|_| "finalize error")
            },
        )
        .unwrap();
        assert_eq!(*events.borrow(), ["close", "restore", "finalize"]);
        assert_eq!(gate.active_mode().unwrap(), None);
    }
}
