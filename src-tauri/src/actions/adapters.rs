use super::{ActionError, ActionInput, ActionRuntime, PreparedAction};
use crate::commands::AppState;
use thiserror::Error;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(super) enum ActionRunError {
    #[error("{0}")]
    Lifecycle(ActionError),
    #[error("动作与领域适配器不匹配")]
    WrongAction,
    #[error("写入剪贴板失败")]
    ClipboardFailed,
}

impl ActionRunError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Lifecycle(error) => error.code(),
            Self::WrongAction => "action_wrong_adapter",
            Self::ClipboardFailed => "action_clipboard_failed",
        }
    }
}

/// 复用产品唯一的文本剪贴板路径：同样的 watcher 抑制、平台重试和 wake 语义。
pub(super) fn copy_text(
    runtime: &ActionRuntime,
    caller_label: &str,
    prepared: &PreparedAction,
    state: &AppState,
) -> Result<(), ActionRunError> {
    execute_text_copy(runtime, caller_label, prepared, |text| {
        crate::commands::copy_text_suppressed(text, state).map_err(|error| {
            log::warn!("动作写入剪贴板失败: {error}");
            ActionRunError::ClipboardFailed
        })
    })
}

fn execute_text_copy(
    runtime: &ActionRuntime,
    caller_label: &str,
    prepared: &PreparedAction,
    write: impl FnOnce(&str) -> Result<(), ActionRunError>,
) -> Result<(), ActionRunError> {
    if prepared.descriptor().id != "text.copy" {
        return Err(ActionRunError::WrongAction);
    }
    let ActionInput::Text(text) = prepared.input() else {
        return Err(ActionRunError::WrongAction);
    };
    runtime
        .commit_noncancellable(caller_label, prepared.handle(), || write(text))
        .map_err(ActionRunError::Lifecycle)??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::Cell;

    #[test]
    fn text_copy_uses_the_validated_exact_text_and_retires_the_slot() {
        let runtime = ActionRuntime::default();
        let prepared = runtime
            .begin(
                "launcher",
                "text.copy",
                "copy",
                json!({"text": "private text\nwith spacing"}),
            )
            .unwrap();
        let writes = Cell::new(0);
        execute_text_copy(&runtime, "launcher", &prepared, |text| {
            assert_eq!(text, "private text\nwith spacing");
            writes.set(writes.get() + 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(writes.get(), 1);
        assert_eq!(
            runtime.ensure_current("launcher", prepared.handle()),
            Err(ActionError::Superseded)
        );
    }

    #[test]
    fn wrong_adapter_and_domain_failure_have_stable_redacted_errors() {
        let runtime = ActionRuntime::default();
        let wrong = runtime
            .begin(
                "launcher",
                "text.translate",
                "translate",
                json!({"text": "secret", "targetLanguage": "zh-CN"}),
            )
            .unwrap();
        assert_eq!(
            execute_text_copy(&runtime, "launcher", &wrong, |_| Ok(())),
            Err(ActionRunError::WrongAction)
        );

        let copy = runtime
            .begin(
                "launcher",
                "text.copy",
                "copy",
                json!({"text": "do-not-log"}),
            )
            .unwrap();
        let error = execute_text_copy(&runtime, "launcher", &copy, |_| {
            Err(ActionRunError::ClipboardFailed)
        })
        .unwrap_err();
        assert_eq!(error.code(), "action_clipboard_failed");
        assert!(!format!("{error:?}").contains("do-not-log"));
        assert_eq!(
            runtime.ensure_current("launcher", copy.handle()),
            Err(ActionError::Superseded)
        );
    }
}
