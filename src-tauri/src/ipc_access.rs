//! 按调用窗口限制自定义应用命令。
//!
//! Tauri capability 继续负责 core/plugin 权限；这里覆盖 `generate_handler!` 注册的业务
//! command，防止一个功能岛借用共享 `api.ts` 调用另一窗口的领域接口。

use serde::Serialize;
use tauri::{ipc::Invoke, Runtime};

const SETTINGS_COMMANDS: &[&str] = &[
    "check_app_update",
    "check_shortcut_conflict",
    "clear_translation_history",
    "close_settings",
    "copy_text",
    "delete_translation_api_key",
    "get_app_update_state",
    "get_config",
    "get_paste_status",
    "get_platform_info",
    "get_shortcut_failures",
    "get_stats",
    "get_window_probe_status",
    "has_translation_api_key",
    "install_app_update",
    "install_window_probe_extension",
    "is_dev_binary",
    "ocr_available",
    "ocr_install",
    "pause_shortcuts",
    "pick_screenshot_directory",
    "request_paste_permission",
    "restart_app",
    "resume_shortcuts",
    "run_capture_diagnostics",
    "set_translation_api_key",
    "tmux_available",
    "toggle_tmux_capture",
    "uninstall_window_probe_extension",
    "update_config",
];

const PIN_COMMANDS: &[&str] = &[
    "close_pin",
    "copy_pin",
    "copy_pin_canvas",
    "get_pin_payload",
    "get_pin_source_image",
    "get_pin_toolbar_bounds",
    "get_platform_info",
    "pin_ready",
    "save_pin",
    "save_pin_canvas",
    "update_pin",
];

const CAPTURE_OVERLAY_COMMANDS: &[&str] = &[
    "cancel_capture_overlay",
    "commit_capture_action",
    "copy_text",
    "get_capture_frame",
    "get_capture_overlay",
    "mark_capture_overlay_ready",
    "open_longshot_controller",
    "retry_capture_action",
    "scan_capture_selection",
    "translate_capture_selection",
];

const LONGSHOT_CONTROLLER_COMMANDS: &[&str] = &[
    "activate_longshot_controller",
    "append_longshot_controller",
    "cancel_longshot_controller",
    "finish_longshot_controller",
    "mark_longshot_controller_ready",
    "preview_longshot_controller",
];

const IMAGE_VIEWER_COMMANDS: &[&str] = &[
    "close_image_viewer",
    "copy_viewer_image",
    "copy_viewer_text",
    "detect_viewer_codes",
    "get_viewer_fullscreen",
    "get_viewer_payload",
    "get_viewer_settings",
    "minimize_image_viewer",
    "pin_viewer_image",
    "recognize_viewer",
    "sample_viewer_color",
    "save_viewer_image",
    "set_viewer_fullscreen",
    "start_viewer_drag",
    "translate_viewer",
    "viewer_ready",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallerKind {
    Main,
    Settings,
    Pin,
    CaptureOverlay,
    LongshotController,
    ImageViewer,
    Unknown,
}

fn safe_dynamic_label(label: &str, prefix: &str, max_len: usize) -> bool {
    label.len() > prefix.len()
        && label.len() <= max_len
        && label.starts_with(prefix)
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn caller_kind(label: &str) -> CallerKind {
    match label {
        "main" => CallerKind::Main,
        "settings" => CallerKind::Settings,
        _ if safe_dynamic_label(label, "pin-", 96) => CallerKind::Pin,
        _ if safe_dynamic_label(label, "capture-overlay-", 128) => CallerKind::CaptureOverlay,
        _ if safe_dynamic_label(label, "longshot-controller-", 128) => {
            CallerKind::LongshotController
        }
        _ if safe_dynamic_label(label, "image-viewer-", 96) => CallerKind::ImageViewer,
        _ => CallerKind::Unknown,
    }
}

pub(crate) fn allowed(caller: &str, command: &str) -> bool {
    let commands = match caller_kind(caller) {
        CallerKind::Main => return true,
        CallerKind::Settings => SETTINGS_COMMANDS,
        CallerKind::Pin => PIN_COMMANDS,
        CallerKind::CaptureOverlay => CAPTURE_OVERLAY_COMMANDS,
        CallerKind::LongshotController => LONGSHOT_CONTROLLER_COMMANDS,
        CallerKind::ImageViewer => IMAGE_VIEWER_COMMANDS,
        CallerKind::Unknown => return false,
    };
    commands.contains(&command)
}

#[derive(Serialize)]
struct Forbidden {
    code: &'static str,
}

pub(crate) fn restrict<R: Runtime>(
    handler: impl Fn(Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        if !allowed(
            invoke.message.webview_ref().label(),
            invoke.message.command(),
        ) {
            invoke.resolver.reject(Forbidden { code: "forbidden" });
            return true;
        }
        handler(invoke)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_restricted_window_accepts_its_complete_declared_domain() {
        for (label, commands) in [
            ("settings", SETTINGS_COMMANDS),
            ("pin-image-1", PIN_COMMANDS),
            ("capture-overlay-session-1", CAPTURE_OVERLAY_COMMANDS),
            ("longshot-controller-1", LONGSHOT_CONTROLLER_COMMANDS),
            ("image-viewer-1", IMAGE_VIEWER_COMMANDS),
        ] {
            let mut unique = commands.to_vec();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(unique.len(), commands.len(), "{label} 的命令清单不能重复");
            for command in commands {
                assert!(allowed(label, command), "{label} 应允许 {command}");
            }
        }
    }

    #[test]
    fn every_restricted_window_rejects_multiple_cross_domain_commands() {
        let cases = [
            (
                "settings",
                [
                    "get_pin_payload",
                    "get_capture_overlay",
                    "get_viewer_payload",
                ],
            ),
            (
                "pin-image-1",
                ["get_config", "get_capture_overlay", "get_viewer_payload"],
            ),
            (
                "capture-overlay-session-1",
                ["get_config", "get_pin_payload", "get_viewer_payload"],
            ),
            (
                "longshot-controller-1",
                ["get_config", "get_pin_payload", "get_viewer_payload"],
            ),
            (
                "image-viewer-1",
                ["get_config", "get_pin_payload", "get_capture_overlay"],
            ),
        ];
        for (label, commands) in cases {
            for command in commands {
                assert!(!allowed(label, command), "{label} 不应允许 {command}");
            }
        }
    }

    #[test]
    fn main_keeps_the_complete_business_surface() {
        for command in [
            "get_clips",
            "get_config",
            "get_pin_payload",
            "get_capture_overlay",
            "append_longshot_controller",
            "get_viewer_payload",
            "future_command_is_not_blocked_here",
        ] {
            assert!(allowed("main", command), "{command}");
        }
    }

    #[test]
    fn unknown_old_and_malformed_labels_are_denied_before_command_matching() {
        for label in [
            "",
            "legacy-viewer-1",
            "pin-",
            "pin-../../main",
            "capture-overlay-one?x=1",
            "longshot-controller-one/two",
            "image-viewer-一",
            "unknown-window",
        ] {
            assert!(!allowed(label, "get_config"), "{label:?}");
            assert!(!allowed(label, "get_viewer_payload"), "{label:?}");
        }
    }

    #[test]
    fn command_names_are_exact_and_typos_do_not_fall_through() {
        assert!(allowed("settings", "get_config"));
        assert!(!allowed("settings", "get-config"));
        assert!(!allowed("settings", "GET_CONFIG"));
        assert!(!allowed("settings", "get_config "));
    }

    #[test]
    fn viewer_keeps_its_existing_forbidden_error_shape() {
        assert_eq!(
            serde_json::to_value(Forbidden { code: "forbidden" }).unwrap(),
            serde_json::json!({"code": "forbidden"})
        );
    }
}
