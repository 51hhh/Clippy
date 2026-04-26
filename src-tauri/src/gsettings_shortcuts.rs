//! gsettings_shortcuts.rs — Wayland/GNOME 全局快捷键模块
//!
//! 原理：
//! 1. 通过 gsettings 在 GNOME 自定义快捷键路径下注册条目
//! 2. gsd-media-keys 启动时从 dconf 读取并通过 Mutter GrabAccelerator 注册键位
//! 3. 按键触发时 gsd-media-keys 执行绑定的 command（dbus-send）
//! 4. 应用内 zbus D-Bus 服务收到 Toggle 方法调用 → 切换窗口
//!
//! 已验证：GNOME 50 + Wayland，gsd-media-keys 可正确 grab 并执行 command。

use std::process::Command;
use tauri::AppHandle;

/// GNOME 自定义快捷键 dconf 路径（标准 custom0 格式）
const DCONF_BASE: &str =
    "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom0/";
/// gsettings schema
const SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
/// gsettings relocatable schema（读写具体条目用）
const ENTRY_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
/// D-Bus Toggle 命令
const DBUS_TOGGLE_CMD: &str =
    "dbus-send --session --type=method_call --dest=com.clippy.app /com/clippy/app com.clippy.app.Toggle";

/// 检测当前是否运行在 Wayland 会话中
pub fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|v| v == "wayland")
        .unwrap_or(false)
}

/// 将 Tauri 快捷键格式转为 GNOME accelerator 格式
///
/// `Ctrl+Alt+V` → `<Control><Alt>v`
/// `Super+V`    → `<Super>v`
fn to_gnome_accel(tauri_shortcut: &str) -> String {
    let parts: Vec<&str> = tauri_shortcut.split('+').collect();
    let mut result = String::new();
    for (i, part) in parts.iter().enumerate() {
        let is_last = i == parts.len() - 1;
        if is_last {
            result.push_str(&part.to_lowercase());
        } else {
            result.push('<');
            let modifier = match part.trim() {
                "Ctrl" | "Control" | "CmdOrCtrl" | "CommandOrControl" => "Control",
                "Alt" => "Alt",
                "Shift" => "Shift",
                "Super" | "Meta" | "Cmd" | "Command" => "Super",
                other => other,
            };
            result.push_str(modifier);
            result.push('>');
        }
    }
    result
}

/// 注册 gsettings 自定义快捷键（应用启动时调用）
pub fn register(shortcut: &str) -> Result<(), String> {
    let accel = to_gnome_accel(shortcut);
    log::info!("注册 GNOME 自定义快捷键: {} -> {}", shortcut, accel);

    ensure_in_custom_list()?;
    gsettings_set("name", "Clippy Toggle")?;
    gsettings_set("command", DBUS_TOGGLE_CMD)?;
    gsettings_set("binding", &accel)?;
    restart_gsd_media_keys()?;

    log::info!("GNOME 自定义快捷键注册完成");
    Ok(())
}

/// 更新绑定（设置页面修改快捷键时调用）
pub fn update_binding(shortcut: &str) -> Result<(), String> {
    let accel = to_gnome_accel(shortcut);
    log::info!("更新 GNOME 快捷键绑定: {}", accel);
    gsettings_set("binding", &accel)?;
    restart_gsd_media_keys()
}

/// 暂停快捷键（录制新快捷键时调用）
pub fn pause() -> Result<(), String> {
    log::info!("暂停 GNOME 快捷键");
    gsettings_set("binding", "")?;
    restart_gsd_media_keys()
}

/// 恢复快捷键
pub fn resume(shortcut: &str) -> Result<(), String> {
    update_binding(shortcut)
}

/// 卸载快捷键
#[allow(dead_code)]
pub fn unregister() -> Result<(), String> {
    log::info!("卸载 GNOME 自定义快捷键");
    remove_from_custom_list()?;
    dconf_reset()
}

// ─── gsettings / dconf 操作 ──────────────────────────────────────────────────

/// 后台重启 gsd-media-keys 使新 binding 生效（不阻塞调用者）
///
/// gsd-media-keys 仅在启动时通过 Mutter GrabAccelerators 注册键位，
/// 运行时修改 dconf 不会触发 re-grab。因此需要 kill → systemd target start。
fn restart_gsd_media_keys() -> Result<(), String> {
    log::info!("后台重启 gsd-media-keys 以应用新绑定");
    std::thread::spawn(|| {
        // kill 现有进程
        let _ = Command::new("pkill")
            .args(["-9", "gsd-media-keys"])
            .status();

        // 等待进程退出
        std::thread::sleep(std::time::Duration::from_millis(300));

        // 通过 systemd target 重新启动
        match Command::new("systemctl")
            .args(["--user", "start", "org.gnome.SettingsDaemon.MediaKeys.target"])
            .status()
        {
            Ok(s) if s.success() => log::info!("gsd-media-keys 已重启"),
            Ok(s) => log::error!("systemctl start MediaKeys.target 退出码: {s}"),
            Err(e) => log::error!("重启 gsd-media-keys 失败: {e}"),
        }
    });
    Ok(())
}

/// 通过 gsettings 写入条目字段（schema-aware，比裸 dconf 更可靠）
fn gsettings_set(key: &str, value: &str) -> Result<(), String> {
    let path_arg = format!("{ENTRY_SCHEMA}:{DCONF_BASE}");
    let status = Command::new("gsettings")
        .args(["set", &path_arg, key, value])
        .status()
        .map_err(|e| format!("gsettings set {key} 失败: {e}"))?;
    if !status.success() {
        return Err(format!("gsettings set {key} 返回非零退出码"));
    }
    Ok(())
}

/// 确保 custom0 路径存在于自定义快捷键列表中
fn ensure_in_custom_list() -> Result<(), String> {
    let output = Command::new("gsettings")
        .args(["get", SCHEMA, "custom-keybindings"])
        .output()
        .map_err(|e| format!("gsettings get 失败: {e}"))?;

    let current = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if current.contains(DCONF_BASE) {
        return Ok(());
    }

    let our_entry = format!("'{DCONF_BASE}'");
    let new_list = if current == "@as []" || current.is_empty() {
        format!("[{our_entry}]")
    } else {
        let trimmed = current.trim_end_matches(']');
        format!("{trimmed}, {our_entry}]")
    };

    let status = Command::new("gsettings")
        .args(["set", SCHEMA, "custom-keybindings", &new_list])
        .status()
        .map_err(|e| format!("gsettings set custom-keybindings 失败: {e}"))?;
    if !status.success() {
        return Err("gsettings set custom-keybindings 返回非零退出码".into());
    }
    Ok(())
}

/// 从自定义快捷键列表中移除 custom0 路径
fn remove_from_custom_list() -> Result<(), String> {
    let output = Command::new("gsettings")
        .args(["get", SCHEMA, "custom-keybindings"])
        .output()
        .map_err(|e| format!("gsettings get 失败: {e}"))?;

    let current = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if !current.contains(DCONF_BASE) {
        return Ok(());
    }

    let entries: Vec<&str> = current
        .trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|s| s.trim().trim_matches('\''))
        .filter(|s| !s.is_empty() && *s != DCONF_BASE)
        .collect();

    let new_list = if entries.is_empty() {
        "@as []".to_string()
    } else {
        let inner: Vec<String> = entries.iter().map(|e| format!("'{e}'")).collect();
        format!("[{}]", inner.join(", "))
    };

    let status = Command::new("gsettings")
        .args(["set", SCHEMA, "custom-keybindings", &new_list])
        .status()
        .map_err(|e| format!("gsettings set 失败: {e}"))?;
    if !status.success() {
        return Err("gsettings set custom-keybindings 返回非零退出码".into());
    }
    Ok(())
}

/// dconf reset 清空条目数据
fn dconf_reset() -> Result<(), String> {
    let status = Command::new("dconf")
        .args(["reset", "-f", DCONF_BASE])
        .status()
        .map_err(|e| format!("dconf reset 失败: {e}"))?;
    if !status.success() {
        return Err("dconf reset 返回非零退出码".into());
    }
    Ok(())
}

// ─── D-Bus 服务 ──────────────────────────────────────────────────────────────

/// 启动 D-Bus 服务：注册 com.clippy.app，暴露 Toggle 方法
pub async fn start_dbus_service(handle: AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use zbus::connection::Builder;
    use zbus::interface;

    struct ClippyInterface {
        handle: AppHandle,
    }

    #[interface(name = "com.clippy.app")]
    impl ClippyInterface {
        fn toggle(&self) {
            log::info!("D-Bus Toggle 被调用");
            super::toggle_main_window(&self.handle);
        }
    }

    let iface = ClippyInterface {
        handle: handle.clone(),
    };

    let _conn = Builder::session()?
        .name("com.clippy.app")?
        .serve_at("/com/clippy/app", iface)?
        .build()
        .await?;

    log::info!("D-Bus 服务已启动: com.clippy.app");

    // 保持连接活跃（_conn 的生命周期 = 此 future 的生命周期）
    std::future::pending::<()>().await;
    Ok(())
}

// ─── 测试 ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_gnome_accel() {
        assert_eq!(to_gnome_accel("Ctrl+Alt+V"), "<Control><Alt>v");
        assert_eq!(to_gnome_accel("Super+V"), "<Super>v");
        assert_eq!(to_gnome_accel("Ctrl+Shift+A"), "<Control><Shift>a");
        assert_eq!(to_gnome_accel("F12"), "f12");
        assert_eq!(to_gnome_accel("Control+Alt+V"), "<Control><Alt>v");
        assert_eq!(to_gnome_accel("CmdOrCtrl+Alt+V"), "<Control><Alt>v");
        assert_eq!(to_gnome_accel("CommandOrControl+Shift+A"), "<Control><Shift>a");
        assert_eq!(to_gnome_accel("Cmd+V"), "<Super>v");
        assert_eq!(to_gnome_accel("Meta+V"), "<Super>v");
    }

    #[test]
    fn test_is_wayland() {
        let _ = is_wayland();
    }
}