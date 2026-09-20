//! 按调用窗口限制自定义应用命令。
//!
//! Tauri capability 继续负责 core/plugin 权限；这里覆盖 `generate_handler!` 注册的业务
//! command，防止一个功能岛借用共享 `api.ts` 调用另一窗口的领域接口。

use serde::Serialize;
use tauri::{ipc::Invoke, Runtime};

const ACTION_COMMANDS: &[&str] = &[
    "cancel_action",
    "discover_actions",
    "prepare_action",
    "prepare_composed_action",
    "run_action",
];

const LAUNCHER_COMMANDS: &[&str] = &[
    "action_launcher_ready",
    "close_action_launcher",
    "get_action_launcher_image_source",
    "get_action_launcher_settings",
    "start_action_launcher_drag",
];

const SETTINGS_COMMANDS: &[&str] = &[
    "check_app_update",
    "check_shortcut_conflict",
    "clear_translation_history",
    "close_settings",
    "copy_text",
    "delete_translation_api_key",
    "export_clippy_archive",
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
    "import_clippy_archive",
    "is_dev_binary",
    "ocr_available",
    "ocr_health_status",
    "ocr_install",
    "pause_shortcuts",
    "pick_screenshot_directory",
    "pick_ocr_manifest",
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
    "save_pin_to_workspace",
    "remove_pin_from_workspace",
    "list_pin_workspace_groups",
    "create_pin_workspace_group",
    "rename_pin_workspace_group",
    "delete_pin_workspace_group",
    "assign_pin_workspace_group",
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

const RECORDING_OVERLAY_COMMANDS: &[&str] = &[
    "cancel_capture_overlay",
    "get_capture_frame",
    "get_capture_overlay",
    "mark_capture_overlay_ready",
    "start_capture_recording",
];

const LONGSHOT_CONTROLLER_COMMANDS: &[&str] = &[
    "activate_longshot_controller",
    "auto_append_longshot_controller",
    "append_longshot_controller",
    "cancel_longshot_controller",
    "finish_longshot_controller",
    "mark_longshot_controller_ready",
    "preview_longshot_controller",
    "undo_longshot_controller",
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

const RECORDING_CONTROL_COMMANDS: &[&str] = &[
    "cancel_recording",
    "mark_recording_control_ready",
    "pause_recording",
    "resume_recording",
    "stop_recording",
];

const RECORDING_LIBRARY_COMMANDS: &[&str] = &[
    "close_recording_library",
    "delete_recording_session",
    "export_recording_artifact",
    "get_recording_library_settings",
    "list_recordings",
    "recording_library_ready",
    "reveal_recording_artifact",
    "start_recording_library_drag",
];

const PIN_WORKSPACE_LIBRARY_COMMANDS: &[&str] = &[
    "assign_pin_workspace_library_group",
    "close_pin_workspace_library",
    "create_pin_workspace_library_group",
    "delete_pin_workspace_library_group",
    "get_pin_workspace_library_settings",
    "get_pin_workspace_thumbnail",
    "list_pin_workspace_library",
    "pin_workspace_library_ready",
    "remove_pin_workspace_library_item",
    "rename_pin_workspace_library_group",
    "show_pin_workspace_item",
    "start_pin_workspace_library_drag",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallerKind {
    Main,
    Launcher,
    Settings,
    Pin,
    CaptureOverlay,
    RecordingOverlay,
    LongshotController,
    ImageViewer,
    RecordingControl,
    RecordingLibrary,
    PinWorkspaceLibrary,
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

pub(crate) fn caller_kind(label: &str) -> CallerKind {
    match label {
        "main" => CallerKind::Main,
        "launcher" => CallerKind::Launcher,
        "settings" => CallerKind::Settings,
        "recordings" => CallerKind::RecordingLibrary,
        "pin-workspaces" => CallerKind::PinWorkspaceLibrary,
        _ if safe_dynamic_label(label, "pin-", 96) => CallerKind::Pin,
        _ if safe_dynamic_label(label, "capture-overlay-", 128) => CallerKind::CaptureOverlay,
        _ if safe_dynamic_label(label, "recording-overlay-", 128) => CallerKind::RecordingOverlay,
        _ if safe_dynamic_label(label, "longshot-controller-", 128) => {
            CallerKind::LongshotController
        }
        _ if safe_dynamic_label(label, "image-viewer-", 96) => CallerKind::ImageViewer,
        _ if safe_dynamic_label(label, "recording-control-", 128) => CallerKind::RecordingControl,
        _ => CallerKind::Unknown,
    }
}

pub(crate) fn allowed(caller: &str, command: &str) -> bool {
    let caller = caller_kind(caller);
    if caller == CallerKind::Main {
        return true;
    }
    if ACTION_COMMANDS.contains(&command) {
        return matches!(
            caller,
            CallerKind::Launcher
                | CallerKind::Pin
                | CallerKind::CaptureOverlay
                | CallerKind::ImageViewer
        );
    }
    let commands = match caller {
        CallerKind::Main => return true,
        CallerKind::Launcher => LAUNCHER_COMMANDS,
        CallerKind::Settings => SETTINGS_COMMANDS,
        CallerKind::Pin => PIN_COMMANDS,
        CallerKind::CaptureOverlay => CAPTURE_OVERLAY_COMMANDS,
        CallerKind::RecordingOverlay => RECORDING_OVERLAY_COMMANDS,
        CallerKind::LongshotController => LONGSHOT_CONTROLLER_COMMANDS,
        CallerKind::ImageViewer => IMAGE_VIEWER_COMMANDS,
        CallerKind::RecordingControl => RECORDING_CONTROL_COMMANDS,
        CallerKind::RecordingLibrary => RECORDING_LIBRARY_COMMANDS,
        CallerKind::PinWorkspaceLibrary => PIN_WORKSPACE_LIBRARY_COMMANDS,
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
            ("launcher", LAUNCHER_COMMANDS),
            ("settings", SETTINGS_COMMANDS),
            ("pin-image-1", PIN_COMMANDS),
            ("capture-overlay-session-1", CAPTURE_OVERLAY_COMMANDS),
            ("recording-overlay-session-1", RECORDING_OVERLAY_COMMANDS),
            ("longshot-controller-1", LONGSHOT_CONTROLLER_COMMANDS),
            ("image-viewer-1", IMAGE_VIEWER_COMMANDS),
            ("recording-control-1", RECORDING_CONTROL_COMMANDS),
            ("recordings", RECORDING_LIBRARY_COMMANDS),
            ("pin-workspaces", PIN_WORKSPACE_LIBRARY_COMMANDS),
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
    fn shared_action_ipc_is_limited_to_declared_functional_windows() {
        for label in [
            "main",
            "launcher",
            "pin-image-1",
            "capture-overlay-session-1",
            "image-viewer-1",
        ] {
            for command in ACTION_COMMANDS {
                assert!(allowed(label, command), "{label} 应允许 {command}");
            }
        }
        for label in [
            "settings",
            "recording-overlay-session-1",
            "longshot-controller-1",
            "recording-control-1",
            "unknown-window",
            "pin-workspaces",
        ] {
            for command in ACTION_COMMANDS {
                assert!(!allowed(label, command), "{label} 不应允许 {command}");
            }
        }
    }

    #[test]
    fn every_restricted_window_rejects_multiple_cross_domain_commands() {
        let cases = [
            (
                "launcher",
                ["get_config", "get_pin_payload", "get_viewer_payload"],
            ),
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
                "recording-overlay-session-1",
                [
                    "commit_capture_action",
                    "scan_capture_selection",
                    "open_longshot_controller",
                ],
            ),
            (
                "longshot-controller-1",
                ["get_config", "get_pin_payload", "get_viewer_payload"],
            ),
            (
                "image-viewer-1",
                ["get_config", "get_pin_payload", "get_capture_overlay"],
            ),
            (
                "recording-control-1",
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
            "recording-overlay-one/two",
            "longshot-controller-one/two",
            "image-viewer-一",
            "recording-control-one/two",
            "unknown-window",
        ] {
            assert!(!allowed(label, "get_config"), "{label:?}");
            assert!(!allowed(label, "get_viewer_payload"), "{label:?}");
        }
    }

    #[test]
    fn launcher_has_only_the_restricted_action_surface() {
        assert_eq!(caller_kind("launcher"), CallerKind::Launcher);
        assert!(!allowed("launcher", "get_config"));
        assert!(allowed("launcher", "discover_actions"));
        assert!(allowed("launcher", "prepare_action"));
        assert!(allowed("launcher", "run_action"));
        assert!(allowed("launcher", "cancel_action"));
    }

    #[test]
    fn recording_overlay_has_only_selection_and_start_commands() {
        assert_eq!(
            caller_kind("recording-overlay-session-1"),
            CallerKind::RecordingOverlay
        );
        for command in RECORDING_OVERLAY_COMMANDS {
            assert!(allowed("recording-overlay-session-1", command));
        }
        for command in [
            "commit_capture_action",
            "scan_capture_selection",
            "translate_capture_selection",
            "open_longshot_controller",
            "discover_actions",
        ] {
            assert!(!allowed("recording-overlay-session-1", command));
        }
    }

    #[test]
    fn recording_library_cannot_control_capture_or_other_windows() {
        assert_eq!(caller_kind("recordings"), CallerKind::RecordingLibrary);
        for command in RECORDING_LIBRARY_COMMANDS {
            assert!(allowed("recordings", command));
        }
        for command in [
            "start_capture_recording",
            "pause_recording",
            "commit_capture_action",
            "get_pin_payload",
            "update_config",
        ] {
            assert!(!allowed("recordings", command), "{command}");
        }
    }

    #[test]
    fn pin_workspace_library_has_only_opaque_workspace_operations() {
        assert_eq!(
            caller_kind("pin-workspaces"),
            CallerKind::PinWorkspaceLibrary
        );
        for command in PIN_WORKSPACE_LIBRARY_COMMANDS {
            assert!(allowed("pin-workspaces", command));
        }
        for command in [
            "get_config",
            "get_pin_payload",
            "save_pin_canvas",
            "delete_recording_session",
            "commit_capture_action",
        ] {
            assert!(!allowed("pin-workspaces", command), "{command}");
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
