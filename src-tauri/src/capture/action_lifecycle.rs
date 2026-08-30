use std::fmt::Display;

/// 会话错误 `E` 与动作错误 `X` 分开：裁剪/结束会话来自 capture 领域，
/// 而动作本身会失败在剪贴板、文件、贴图等其他领域，最终统一收敛成 `X`。
pub(super) fn complete_capture_action<P, S, R, E, X, F, C, T, A>(
    crop_result: Result<P, E>,
    finish: F,
    close_overlays: C,
    restore_sources: T,
    execute: A,
) -> Result<R, X>
where
    E: Display + Into<X>,
    F: FnOnce() -> Result<S, E>,
    C: FnOnce(&S),
    T: FnOnce(&S),
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
            return Err(error.into());
        }
    };

    let result = execute(payload);
    // 标注已经在覆盖层里完成，提交后没有任何窗口要接管焦点，
    // 所以每条路径都必须把截图前的源窗口还回去。
    restore_sources(&session);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::error::CaptureError;
    use std::cell::RefCell;

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
            assert_eq!(*events.borrow(), ["finish", "close", "action", "restore"]);
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
                events.borrow_mut().push("action");
                Ok(())
            },
        );

        assert_eq!(result.unwrap_err(), "decode error");
        assert_eq!(*events.borrow(), ["finish", "close", "restore"]);
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
            |_| panic!("载荷无效时不应执行动作"),
        );

        assert_eq!(result.unwrap_err(), "提交的截图数据无效");
    }
}
