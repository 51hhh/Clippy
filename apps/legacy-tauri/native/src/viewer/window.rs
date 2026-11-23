use super::manager::ViewerSession;
use super::model::{ViewerError, ViewerHandle};
use std::sync::Arc;
use tauri::Manager;

/// 原生 getter 可等待 UI；此门槛只读取不可变身份与原子生命周期，不持会话锁。
pub(super) fn with_owned<T>(
    session: &ViewerSession,
    caller: &str,
    handle: &ViewerHandle,
    action: impl FnOnce() -> Result<T, ViewerError>,
) -> Result<T, ViewerError> {
    session.authorize(caller, handle)?;
    ensure_active(session)?;
    let result = action()?;
    // 排队中的查询不能在窗口关闭后返回有效结果。
    ensure_active(session)?;
    Ok(result)
}

fn ensure_active(session: &ViewerSession) -> Result<(), ViewerError> {
    if session.is_active() {
        Ok(())
    } else {
        Err("closed".into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RestoreStep {
    Show,
    Unminimize,
    Focus,
}

pub(super) fn restore_with(
    session: &ViewerSession,
    mut action: impl FnMut(RestoreStep) -> Result<(), ViewerError>,
) -> Result<(), ViewerError> {
    ensure_active(session)?;
    // 首帧未完成仍由 ready 入口显示，重复打开不能提前闪出空白窗口。
    if !session.ready() {
        return Ok(());
    }
    for step in [
        RestoreStep::Show,
        RestoreStep::Unminimize,
        RestoreStep::Focus,
    ] {
        ensure_active(session)?;
        action(step)?;
    }
    ensure_active(session)
}

pub(super) fn restore(
    window: &tauri::WebviewWindow,
    session: &ViewerSession,
) -> Result<(), ViewerError> {
    restore_with(session, |step| {
        match step {
            RestoreStep::Show => window.show(),
            RestoreStep::Unminimize => window.unminimize(),
            RestoreStep::Focus => window.set_focus(),
        }
        .map_err(|_| ViewerError::new("window_failed"))
    })
}

pub(super) fn create(
    app: &tauri::AppHandle,
    session: &Arc<ViewerSession>,
) -> Result<(), ViewerError> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        &session.payload.label,
        tauri::WebviewUrl::App("viewer.html".into()),
    )
    .title("Clippy — Image viewer")
    .inner_size(1000.0, 720.0)
    .min_inner_size(320.0, 240.0)
    .resizable(true)
    .decorations(false)
    .transparent(false)
    .always_on_top(false)
    .skip_taskbar(false)
    .visible(false)
    .center()
    .build()
    .map_err(|_| ViewerError::new("window_failed"))?;
    let app = app.clone();
    let label = window.label().to_string();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let state = app.state::<crate::commands::AppState>();
        if state
            .viewer_manager
            .get(&label)
            .is_ok_and(|session| session.expire_unready())
        {
            state.viewer_manager.remove(&label);
            if let Some(window) = app.get_webview_window(&label) {
                let _ = window.destroy();
            }
        }
    });
    Ok(())
}
