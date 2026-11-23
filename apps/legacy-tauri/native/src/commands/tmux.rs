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
                teardown_tmux_hook()?;
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

#[cfg(target_os = "linux")]
const TMUX_BINDINGS: &[(&str, &str)] = &[
    ("copy-mode-vi", "y"),
    ("copy-mode-vi", "Enter"),
    ("copy-mode-vi", "MouseDragEnd1Pane"),
    ("copy-mode", "M-w"),
    ("copy-mode", "Enter"),
    ("copy-mode", "MouseDragEnd1Pane"),
];

/// 配置 tmux copy-mode 绑定。第一次启用时保存用户原绑定，重启只会重放 Clippy 绑定。
#[cfg(target_os = "linux")]
pub(crate) fn setup_tmux_hook() -> Result<(), String> {
    ensure_tmux_state_dir()?;
    validate_buffer_target()?;
    let backup = tmux_backup_path();
    if backup.exists() {
        validate_private_backup(&backup)?;
    } else {
        write_binding_backup(&backup)?;
    }

    if let Err(error) = remove_legacy_clippy_hook().and_then(|_| apply_clippy_bindings()) {
        let rollback = restore_bindings();
        return Err(match rollback {
            Ok(()) => format!("{error}; 已恢复原 tmux 绑定"),
            Err(rollback_error) => format!("{error}; 原 tmux 绑定恢复失败: {rollback_error}"),
        });
    }

    log::info!("tmux copy-pipe 绑定已配置；原绑定已私有备份");
    Ok(())
}

#[cfg(target_os = "linux")]
fn apply_clippy_bindings() -> Result<(), String> {
    let pipe_cmd = tmux_pipe_command(&tmux_buf_path());
    for (table, key) in TMUX_BINDINGS {
        run_tmux_checked(
            &[
                "bind-key",
                "-T",
                table,
                key,
                "send-keys",
                "-X",
                "copy-pipe-and-cancel",
                &pipe_cmd,
            ],
            &format!("配置 {table}/{key}"),
        )?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn write_binding_backup(path: &std::path::Path) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut contents = String::from("# Clippy tmux binding backup v1\n");
    for (table, key) in TMUX_BINDINGS {
        contents.push_str(&snapshot_binding(table, key)?);
    }

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| format!("创建 tmux 绑定备份失败: {error}"))?;
    if let Err(error) = file
        .write_all(contents.as_bytes())
        .and_then(|_| file.sync_all())
    {
        let _ = std::fs::remove_file(path);
        return Err(format!("写入 tmux 绑定备份失败: {error}"));
    }
    validate_private_backup(path)
}

#[cfg(target_os = "linux")]
fn snapshot_binding(table: &str, key: &str) -> Result<String, String> {
    let output = std::process::Command::new("tmux")
        .args(["list-keys", "-T", table, key])
        .output()
        .map_err(|error| format!("读取 tmux 绑定 {table}/{key} 失败: {error}"))?;
    if output.status.success() {
        let value = String::from_utf8(output.stdout)
            .map_err(|_| format!("tmux 绑定 {table}/{key} 不是 UTF-8"))?;
        if !value.trim().is_empty() {
            return Ok(restore_directive(table, key, &value));
        }
    } else {
        // 精确键不存在时 tmux 返回失败；再查询整张表，区分“未绑定”和 tmux/server 故障。
        run_tmux_checked(
            &["list-keys", "-T", table],
            &format!("验证 tmux 表 {table}"),
        )?;
    }
    Ok(format!("unbind-key -q -T {table} {key}\n"))
}

/// 旧版 Clippy 没有备份。只在输出明确指向 Clippy 私有缓冲文件时使用旧版的退出语义迁移，
/// 其它用户绑定保持 tmux 原样输出并由 source-file 精确恢复。
#[cfg(target_os = "linux")]
fn restore_directive(table: &str, key: &str, current: &str) -> String {
    if current.contains("copy-pipe-and-cancel")
        && current.contains("tmux-buf")
        && current.contains("clippy")
    {
        if key == "MouseDragEnd1Pane" {
            return format!("unbind-key -q -T {table} {key}\n");
        }
        return format!("bind-key -T {table} {key} send-keys -X copy-selection-and-cancel\n");
    }
    let mut value = current.to_string();
    if !value.ends_with('\n') {
        value.push('\n');
    }
    value
}

/// 仅移除精确匹配旧版 Clippy 命令的 hook 索引；新实现不创建或占用任何全局 hook。
#[cfg(target_os = "linux")]
fn remove_legacy_clippy_hook() -> Result<(), String> {
    let output = std::process::Command::new("tmux")
        .args(["show-hooks", "-g", "after-copy-mode"])
        .output()
        .map_err(|error| format!("检查旧 tmux hook 失败: {error}"))?;
    if !output.status.success() {
        // 没有该 hook 是正常状态；用 show-hooks 总表确认 server 本身可用。
        run_tmux_checked(&["show-hooks", "-g"], "验证 tmux hook 表")?;
        return Ok(());
    }
    let hook =
        String::from_utf8(output.stdout).map_err(|_| "旧 tmux hook 输出不是 UTF-8".to_string())?;
    for key in legacy_clippy_hook_keys(&hook)? {
        run_tmux_checked(&["set-hook", "-gu", &key], "移除旧版 Clippy tmux hook")?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn legacy_clippy_hook_keys(output: &str) -> Result<Vec<String>, String> {
    output
        .lines()
        .filter(|line| {
            line.contains("run-shell")
                && line.contains("sleep 0.1")
                && line.contains("tmux save-buffer")
                && line.contains("tmux-buf")
                && line.contains("clippy")
        })
        .map(|line| {
            let key = line
                .split_ascii_whitespace()
                .next()
                .ok_or_else(|| "旧 tmux hook 缺少名称".to_string())?;
            let indexed = key
                .strip_prefix("after-copy-mode[")
                .and_then(|suffix| suffix.strip_suffix(']'))
                .is_some_and(|index| {
                    !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
                });
            if key != "after-copy-mode" && !indexed {
                return Err("旧 tmux hook 名称无效，拒绝修改全局 hook".to_string());
            }
            Ok(key.to_string())
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn teardown_tmux_hook() -> Result<(), String> {
    ensure_tmux_state_dir()?;
    restore_bindings()?;
    remove_buffer_file()?;
    log::info!("tmux 原绑定已恢复，Clippy 缓冲文件已移除");
    Ok(())
}

#[cfg(target_os = "linux")]
fn restore_bindings() -> Result<(), String> {
    let backup = tmux_backup_path();
    if !backup.exists() {
        return Ok(());
    }
    validate_private_backup(&backup)?;
    if !tmux_server_running() {
        std::fs::remove_file(&backup)
            .map_err(|error| format!("删除已失效的 tmux 绑定备份失败: {error}"))?;
        return Ok(());
    }
    let backup_text = backup
        .to_str()
        .ok_or_else(|| "tmux 绑定备份路径不是 UTF-8".to_string())?;
    run_tmux_checked(&["source-file", backup_text], "恢复 tmux 原绑定")?;
    std::fs::remove_file(&backup).map_err(|error| format!("删除 tmux 绑定备份失败: {error}"))
}

#[cfg(target_os = "linux")]
fn remove_buffer_file() -> Result<(), String> {
    match std::fs::remove_file(tmux_buf_path()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("删除 tmux 缓冲文件失败: {error}")),
    }
}

#[cfg(target_os = "linux")]
fn run_tmux_checked(args: &[&str], context: &str) -> Result<String, String> {
    let output = std::process::Command::new("tmux")
        .args(args)
        .output()
        .map_err(|error| format!("{context}失败: {error}"))?;
    checked_tmux_output(output, context)
}

#[cfg(target_os = "linux")]
fn checked_tmux_output(output: std::process::Output, context: &str) -> Result<String, String> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{context}失败: {}", stderr.trim()));
    }
    String::from_utf8(output.stdout).map_err(|_| format!("{context}输出不是 UTF-8"))
}

#[cfg(target_os = "linux")]
fn tmux_server_running() -> bool {
    std::process::Command::new("tmux")
        .args(["list-sessions"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "linux")]
pub(crate) fn tmux_bindings_present() -> bool {
    let expected_path = tmux_buf_path();
    let Some(expected_path) = expected_path.to_str() else {
        return false;
    };
    TMUX_BINDINGS.iter().all(|(table, key)| {
        std::process::Command::new("tmux")
            .args(["list-keys", "-T", table, key])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .is_some_and(|output| {
                let binding = String::from_utf8_lossy(&output.stdout);
                binding.contains("copy-pipe-and-cancel") && binding.contains(expected_path)
            })
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn ensure_tmux_state_dir() -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let directory = tmux_state_dir();
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("创建 tmux 状态目录失败: {error}"))?;
    let metadata = std::fs::symlink_metadata(&directory)
        .map_err(|error| format!("读取 tmux 状态目录失败: {error}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("tmux 状态路径必须是普通目录".to_string());
    }
    if metadata.uid() != nix::unistd::Uid::effective().as_raw() {
        return Err("tmux 状态目录不属于当前用户".to_string());
    }
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("收紧 tmux 状态目录权限失败: {error}"))
}

#[cfg(target_os = "linux")]
fn validate_buffer_target() -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let path = tmux_buf_path();
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("读取 tmux 缓冲文件失败: {error}")),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("tmux 缓冲路径必须是普通文件".to_string());
    }
    if metadata.uid() != nix::unistd::Uid::effective().as_raw() {
        return Err("tmux 缓冲文件不属于当前用户".to_string());
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("收紧 tmux 缓冲文件权限失败: {error}"))
}

#[cfg(target_os = "linux")]
fn validate_private_backup(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("读取 tmux 绑定备份失败: {error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("tmux 绑定备份必须是普通文件".to_string());
    }
    if metadata.uid() != nix::unistd::Uid::effective().as_raw() {
        return Err("tmux 绑定备份不属于当前用户".to_string());
    }
    if metadata.mode() & 0o077 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("收紧 tmux 绑定备份权限失败: {error}"))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn tmux_pipe_command(path: &std::path::Path) -> String {
    format!("umask 077; cat > {}", shell_quote(&path.to_string_lossy()))
}

#[cfg(target_os = "linux")]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(target_os = "linux")]
fn tmux_state_dir() -> std::path::PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("clippy-{}", nix::unistd::Uid::effective().as_raw()))
        })
        .join("clippy")
}

/// tmux 缓冲区文件路径。
#[cfg(target_os = "linux")]
pub fn tmux_buf_path() -> std::path::PathBuf {
    tmux_state_dir().join("tmux-buf")
}

#[cfg(target_os = "linux")]
fn tmux_backup_path() -> std::path::PathBuf {
    tmux_state_dir().join("tmux-bindings.conf")
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn shell_path_is_single_quoted_without_a_blacklist() {
        assert_eq!(shell_quote("/tmp/a b'c"), "'/tmp/a b'\"'\"'c'");
        assert_eq!(
            tmux_pipe_command(std::path::Path::new("/tmp/a b'c")),
            "umask 077; cat > '/tmp/a b'\"'\"'c'"
        );
    }

    #[test]
    fn restore_plan_preserves_custom_bindings_and_migrates_only_legacy_clippy_ones() {
        let custom = "bind-key -T copy-mode-vi y send-keys -X copy-selection\n";
        assert_eq!(restore_directive("copy-mode-vi", "y", custom), custom);

        let legacy = "bind-key -T copy-mode-vi y send-keys -X copy-pipe-and-cancel 'cat > /run/user/1/clippy/tmux-buf'\n";
        assert_eq!(
            restore_directive("copy-mode-vi", "y", legacy),
            "bind-key -T copy-mode-vi y send-keys -X copy-selection-and-cancel\n"
        );
        assert_eq!(
            restore_directive("copy-mode", "MouseDragEnd1Pane", legacy),
            "unbind-key -q -T copy-mode MouseDragEnd1Pane\n"
        );
    }

    #[test]
    fn backup_validation_rejects_symlinks_and_repairs_permissions() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let directory = tempfile::tempdir().unwrap();
        let backup = directory.path().join("backup.conf");
        std::fs::write(&backup, "# backup\n").unwrap();
        std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o644)).unwrap();
        validate_private_backup(&backup).unwrap();
        assert_eq!(
            std::fs::metadata(&backup).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let link = directory.path().join("link.conf");
        symlink(&backup, &link).unwrap();
        assert!(validate_private_backup(&link).is_err());
    }

    #[test]
    fn failed_tmux_commands_propagate_their_stderr() {
        use std::os::unix::process::ExitStatusExt;

        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(1 << 8),
            stdout: Vec::new(),
            stderr: b"server unavailable\n".to_vec(),
        };
        assert_eq!(
            checked_tmux_output(output, "配置绑定").unwrap_err(),
            "配置绑定失败: server unavailable"
        );
    }

    #[test]
    fn legacy_hook_cleanup_targets_only_matching_hook_indexes() {
        let hooks = concat!(
            "after-copy-mode[0] run-shell 'notify-send copied'\n",
            "after-copy-mode[1] run-shell -b \"sleep 0.1; tmux save-buffer /run/user/1/clippy/tmux-buf\"\n",
        );
        assert_eq!(
            legacy_clippy_hook_keys(hooks).unwrap(),
            vec!["after-copy-mode[1]"]
        );
        assert!(legacy_clippy_hook_keys(
            "before-copy-mode[1] run-shell 'sleep 0.1; tmux save-buffer /tmp/clippy/tmux-buf'"
        )
        .is_err());
    }
}
