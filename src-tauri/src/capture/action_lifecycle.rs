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

/// cancel 只有成功认领精确 session id 后才执行 cleanup。
pub(super) fn complete_capture_cancel<S, E, F, C, T, Z>(
    finish: F,
    close_overlays: C,
    restore_sources: T,
    finalize: Z,
) -> Result<(), E>
where
    F: FnOnce() -> Result<S, E>,
    C: FnOnce(&S),
    T: FnOnce(&S),
    Z: FnOnce(S) -> Result<(), E>,
{
    finish_capture_session(
        || finish().map(Some),
        close_overlays,
        restore_sources,
        finalize,
    )?;
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

/// 会话错误 `E` 与动作错误 `X` 分开：裁剪/结束会话来自 capture 领域，
/// 而动作本身会失败在剪贴板、文件、贴图等其他领域，最终统一收敛成 `X`。
pub(super) fn complete_capture_action<P, S, R, E, X, F, C, T, Z, A>(
    crop_result: Result<P, E>,
    finish: F,
    close_overlays: C,
    restore_sources: T,
    finalize: Z,
    execute: A,
) -> Result<R, X>
where
    E: Display + Into<X>,
    F: FnOnce() -> Result<S, E>,
    C: FnOnce(&S),
    T: FnOnce(&S),
    Z: FnOnce(S) -> Result<(), E>,
    A: FnOnce(P) -> Result<R, X>,
{
    // 先认领会话再执行动作，避免并发取消后仍产生复制、保存或开窗副作用。
    let session = match finish() {
        Ok(session) => session,
        Err(finish_error) => {
            return match crop_result {
                Ok(_) => Err(finish_error.into()),
                Err(crop_error) => {
                    log::warn!("截图裁剪失败后结束会话也失败: {finish_error}");
                    Err(crop_error.into())
                }
            };
        }
    };

    close_overlays(&session);
    let payload = match crop_result {
        Ok(payload) => payload,
        Err(error) => {
            restore_sources(&session);
            if let Err(finalize_error) = finalize(session) {
                log::error!("截图主错误后释放模式所有权也失败: {finalize_error}");
            }
            return Err(error.into());
        }
    };

    let result = execute(payload);
    // 标注已经在覆盖层里完成，提交后没有任何窗口要接管焦点，
    // 所以每条路径都必须把截图前的源窗口还回去。
    restore_sources(&session);
    let finalize_result = finalize(session);
    match (result, finalize_result) {
        (Err(primary), Err(finalize_error)) => {
            log::error!("截图动作失败后释放模式所有权也失败: {finalize_error}");
            Err(primary)
        }
        (Err(primary), Ok(())) => Err(primary),
        (Ok(_), Err(finalize_error)) => Err(finalize_error.into()),
        (Ok(value), Ok(())) => Ok(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::error::CaptureError;
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
                    || Ok::<_, &str>(slot.borrow_mut().take().expect("cancel 胜出")),
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
                    || slot.borrow_mut().take().ok_or("missing"),
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
                Err::<OwnedSession, _>("superseded")
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
            || Ok::<_, &str>(slot.borrow_mut().take().expect("正确 id 应取走")),
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

    #[test]
    fn capture_action_always_closes_overlays_and_restores_sources() {
        for action_succeeds in [true, false] {
            let events = RefCell::new(Vec::new());
            let result: Result<&str, String> = complete_capture_action(
                Ok::<_, String>("png"),
                || {
                    events.borrow_mut().push("finish");
                    Ok("session")
                },
                |_| events.borrow_mut().push("close"),
                |_| events.borrow_mut().push("restore"),
                |_| {
                    events.borrow_mut().push("finalize");
                    Ok(())
                },
                |_| {
                    events.borrow_mut().push("action");
                    if action_succeeds {
                        Ok("done")
                    } else {
                        Err("action error".to_string())
                    }
                },
            );

            assert_eq!(result.is_ok(), action_succeeds);
            // 编辑器窗口删掉之后不再有"由编辑器接管焦点"的例外分支。
            assert_eq!(
                *events.borrow(),
                ["finish", "close", "action", "restore", "finalize"]
            );
            if !action_succeeds {
                assert_eq!(result.unwrap_err(), "action error");
            }
        }
    }

    #[test]
    fn failed_payload_still_claims_closes_and_restores_its_session() {
        let events = RefCell::new(Vec::new());
        let result: Result<(), String> = complete_capture_action(
            Err::<(), String>("decode error".to_string()),
            || {
                events.borrow_mut().push("finish");
                Ok("session")
            },
            |_| events.borrow_mut().push("close"),
            |_| events.borrow_mut().push("restore"),
            |_| {
                events.borrow_mut().push("finalize");
                Ok(())
            },
            |_| {
                events.borrow_mut().push("action");
                Ok(())
            },
        );

        assert_eq!(result.unwrap_err(), "decode error");
        assert_eq!(*events.borrow(), ["finish", "close", "restore", "finalize"]);
    }

    #[test]
    fn finish_race_prevents_action_and_reports_finish_error() {
        let events = RefCell::new(Vec::new());
        let result: Result<(), String> = complete_capture_action(
            Ok::<_, String>("png"),
            || {
                events.borrow_mut().push("finish");
                Err::<(), _>("finish error".to_string())
            },
            |_| events.borrow_mut().push("close"),
            |_| events.borrow_mut().push("restore"),
            |_| {
                events.borrow_mut().push("finalize");
                Ok(())
            },
            |_| {
                events.borrow_mut().push("action");
                Ok(())
            },
        );

        assert_eq!(result.unwrap_err(), "finish error");
        assert_eq!(*events.borrow(), ["finish"]);
    }

    #[test]
    fn payload_error_remains_primary_when_finish_also_fails() {
        let result = complete_capture_action(
            Err::<(), _>("decode error".to_string()),
            || Err::<(), _>("finish error".to_string()),
            |_| panic!("未认领会话时不应关闭覆盖层"),
            |_| panic!("未认领会话时不应恢复源窗口"),
            |_| -> Result<(), String> { panic!("未认领会话时不应终结") },
            |_| -> Result<(), String> { panic!("载荷无效时不应执行动作") },
        );

        assert_eq!(result.unwrap_err(), "decode error");
    }

    #[test]
    fn session_error_converts_into_action_error_type() {
        // 会话错误是结构化的 CaptureError，动作错误是 IPC 边界的 String，
        // 两者必须能在同一次调用里收敛。
        let result: Result<(), String> = complete_capture_action(
            Err::<(), CaptureError>(CaptureError::CommitPayloadInvalid),
            || Ok("session"),
            |_| {},
            |_| {},
            |_| Ok(()),
            |_| panic!("载荷无效时不应执行动作"),
        );

        assert_eq!(result.unwrap_err(), "提交的截图数据无效");
    }

    #[test]
    fn primary_errors_win_but_success_surfaces_finalizer_error() {
        let action_error: Result<(), String> = complete_capture_action(
            Ok::<_, String>("png"),
            || Ok("session"),
            |_| {},
            |_| {},
            |_| Err("finalizer error".to_string()),
            |_| Err("action error".to_string()),
        );
        assert_eq!(action_error.unwrap_err(), "action error");

        let render_error: Result<(), String> = complete_capture_action(
            Err::<(), _>("render error".to_string()),
            || Ok("session"),
            |_| {},
            |_| {},
            |_| Err("finalizer error".to_string()),
            |_| panic!("渲染失败不执行动作"),
        );
        assert_eq!(render_error.unwrap_err(), "render error");

        let finalize_error: Result<(), String> = complete_capture_action(
            Ok::<_, String>("png"),
            || Ok("session"),
            |_| {},
            |_| {},
            |_| Err("finalizer error".to_string()),
            |_| Ok(()),
        );
        assert_eq!(finalize_error.unwrap_err(), "finalizer error");
    }
}
