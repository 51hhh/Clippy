//! 自定义应用命令在未声明 AppManifest 时不受 core/plugin capability 白名单约束。
//! 查看器在总分发入口额外限制业务命令，不能通过旧 IPC 绕过会话资源合同。
use super::model::ViewerError;
use tauri::{ipc::Invoke, Runtime};

pub(super) fn allowed(caller: &str, command: &str) -> bool {
    !caller.starts_with("image-viewer-")
        || matches!(
            command,
            "get_viewer_payload"
                | "get_viewer_settings"
                | "viewer_ready"
                | "close_image_viewer"
                | "get_viewer_fullscreen"
                | "set_viewer_fullscreen"
                | "minimize_image_viewer"
                | "start_viewer_drag"
                | "recognize_viewer"
                | "detect_viewer_codes"
                | "translate_viewer"
                | "sample_viewer_color"
                | "copy_viewer_image"
                | "save_viewer_image"
                | "pin_viewer_image"
                | "copy_viewer_text"
        )
}

pub(crate) fn restrict<R: Runtime>(
    handler: impl Fn(Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        if !allowed(
            invoke.message.webview_ref().label(),
            invoke.message.command(),
        ) {
            invoke.resolver.reject(ViewerError::new("forbidden"));
            return true;
        }
        handler(invoke)
    }
}
