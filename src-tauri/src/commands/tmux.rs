use super::AppState;
use crate::config::save_config;
use tauri::State;

/// 切换 tmux 缓冲区捕获。
#[tauri::command]
pub fn toggle_tmux_capture(enabled: bool, state: State<AppState>) -> Result<(), String> {
    if !cfg!(target_os = "linux") && enabled {
        return Err("tmux clipboard capture is not available on this platform".to_string());
    }
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    if config.tmux_capture == enabled {
        return Ok(());
    }
    let mut next = config.clone();
    next.tmux_capture = enabled;
    crate::config::commit_config_change(
        &mut config,
        next,
        |value| {
            save_config(&state.config_path, value).map_err(|error| format!("配置保存失败: {error}"))
        },
        |value| {
            #[cfg(target_os = "linux")]
            if value.tmux_capture {
                setup_tmux_hook()?;
            } else {
                teardown_tmux_hook();
            }
            #[cfg(not(target_os = "linux"))]
            let _ = value;
            Ok(())
        },
    )
}

/// 检测 tmux 是否可用。
#[tauri::command]
pub fn tmux_available() -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    std::process::Command::new("tmux")
        .arg("-V")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// 配置 tmux copy-mode 绑定和 after-copy-mode 兜底 hook。
#[cfg(target_os = "linux")]
pub(crate) fn setup_tmux_hook() -> Result<(), String> {
    let buf_path = tmux_buf_path();
    if let Some(parent) = buf_path.parent() {
        let _ = std::fs::create_dir_all(parent);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }

    let path_str = buf_path.to_string_lossy();
    if path_str.contains(['"', '\'', ';', '&', '|']) {
        return Err("tmux 缓冲路径包含不安全字符".to_string());
    }

    let pipe_cmd = format!("cat > {}", path_str);
    let output = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode-vi",
            "y",
            "send-keys",
            "-X",
            "copy-pipe-and-cancel",
            &pipe_cmd,
        ])
        .output()
        .map_err(|e| format!("执行 tmux 失败: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("tmux copy-pipe 绑定失败 (y): {}", stderr.trim()));
    }

    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode-vi",
            "Enter",
            "send-keys",
            "-X",
            "copy-pipe-and-cancel",
            &pipe_cmd,
        ])
        .output();
    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode-vi",
            "MouseDragEnd1Pane",
            "send-keys",
            "-X",
            "copy-pipe-and-cancel",
            &pipe_cmd,
        ])
        .output();
    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode",
            "M-w",
            "send-keys",
            "-X",
            "copy-pipe-and-cancel",
            &pipe_cmd,
        ])
        .output();
    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode",
            "Enter",
            "send-keys",
            "-X",
            "copy-pipe-and-cancel",
            &pipe_cmd,
        ])
        .output();
    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode",
            "MouseDragEnd1Pane",
            "send-keys",
            "-X",
            "copy-pipe-and-cancel",
            &pipe_cmd,
        ])
        .output();

    let hook_cmd = format!("run-shell -b \"sleep 0.1; tmux save-buffer {}\"", path_str);
    let _ = std::process::Command::new("tmux")
        .args(["set-hook", "-g", "after-copy-mode", &hook_cmd])
        .output();

    log::info!("tmux copy-pipe 绑定和 after-copy-mode hook 已配置");
    Ok(())
}

#[cfg(target_os = "linux")]
fn teardown_tmux_hook() {
    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode-vi",
            "y",
            "send-keys",
            "-X",
            "copy-selection-and-cancel",
        ])
        .output();
    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode-vi",
            "Enter",
            "send-keys",
            "-X",
            "copy-selection-and-cancel",
        ])
        .output();
    let _ = std::process::Command::new("tmux")
        .args(["unbind-key", "-T", "copy-mode-vi", "MouseDragEnd1Pane"])
        .output();
    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode",
            "M-w",
            "send-keys",
            "-X",
            "copy-selection-and-cancel",
        ])
        .output();
    let _ = std::process::Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "copy-mode",
            "Enter",
            "send-keys",
            "-X",
            "copy-selection-and-cancel",
        ])
        .output();
    let _ = std::process::Command::new("tmux")
        .args(["unbind-key", "-T", "copy-mode", "MouseDragEnd1Pane"])
        .output();
    let _ = std::process::Command::new("tmux")
        .args(["set-hook", "-gu", "after-copy-mode"])
        .output();
    let _ = std::fs::remove_file(tmux_buf_path());
    log::info!("tmux 绑定和 hook 已移除");
}

/// tmux 缓冲区文件路径。
#[cfg(target_os = "linux")]
pub fn tmux_buf_path() -> std::path::PathBuf {
    std::path::PathBuf::from(
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string()),
    )
    .join("clippy")
    .join("tmux-buf")
}
