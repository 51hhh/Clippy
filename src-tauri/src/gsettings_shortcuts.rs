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
use std::sync::OnceLock;
use tauri::AppHandle;

/// GNOME 自定义快捷键的 dconf 路径前缀（条目按 customN 编号）
const CUSTOM_PREFIX: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";
/// gsettings schema
const SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
/// gsettings relocatable schema（读写具体条目用）
const ENTRY_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
/// D-Bus Toggle 命令（使用 .Shortcuts 子名称，避免与 GTK GApplication 的 com.clippy.app 冲突）
const DBUS_TOGGLE_CMD: &str =
    "dbus-send --session --type=method_call --dest=com.clippy.app.Shortcuts /com/clippy/app com.clippy.app.Toggle";
/// D-Bus PinCurrent 命令
const DBUS_PIN_CMD: &str =
    "dbus-send --session --type=method_call --dest=com.clippy.app.Shortcuts /com/clippy/app com.clippy.app.PinCurrent";
/// D-Bus Capture 命令
const DBUS_CAPTURE_CMD: &str =
    "dbus-send --session --type=method_call --dest=com.clippy.app.Shortcuts /com/clippy/app com.clippy.app.Capture";

/// 非 GNOME 桌面上这条路径不可用时给出的原因（会传到设置页）
pub const NOT_GNOME_REASON: &str =
    "当前 Wayland 桌面不由 gsd-media-keys 管理自定义快捷键，无法自动注册";

/// 检测当前是否运行在 Wayland 会话中
pub fn is_wayland() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|v| v == "wayland")
            .unwrap_or(false)
}

/// 桌面是否是 GNOME 系。
///
/// Wayland 下的自动注册依赖 gsd-media-keys 读取 dconf 再向 Mutter grab 键位，
/// 这是 GNOME 特有的链路：KDE/wlroots 上即使 gsettings 写入成功也没有任何组件会读它，
/// 快捷键会静默失效。因此这里必须显式判断，而不是"能写入就算注册成功"。
pub fn is_gnome_desktop_with(desktop: Option<&str>, session: Option<&str>) -> bool {
    [desktop, session].iter().flatten().any(|value| {
        value
            .split(':')
            .any(|part| part.trim().eq_ignore_ascii_case("gnome"))
    })
}

pub fn is_gnome_desktop() -> bool {
    is_gnome_desktop_with(
        std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
        std::env::var("XDG_SESSION_DESKTOP").ok().as_deref(),
    )
}

/// Clippy 三个动作各自使用的自定义快捷键条目路径
///
/// 不能写死 custom0/1/2：GNOME 里这些编号是先到先得的，用户自己建的快捷键很可能已经
/// 占了它们，直接覆盖 name/command/binding 会把用户的快捷键静默销毁。
#[derive(Debug, Clone, PartialEq, Eq)]
struct CustomSlots {
    toggle: String,
    pin: String,
    capture: String,
}

impl CustomSlots {
    fn paths(&self) -> [&str; 3] {
        [
            self.toggle.as_str(),
            self.pin.as_str(),
            self.capture.as_str(),
        ]
    }
}

/// 条目 command 里用于认领 Clippy 自己条目的 D-Bus 方法名（顺序 = toggle/pin/capture）
const CLIPPY_METHODS: [&str; 3] = [
    "com.clippy.app.Toggle",
    "com.clippy.app.PinCurrent",
    "com.clippy.app.Capture",
];

fn custom_path(index: usize) -> String {
    format!("{CUSTOM_PREFIX}custom{index}/")
}

/// 路径比较忽略末尾斜杠差异（gsettings 写入时总带斜杠，手工配置的可能不带）
fn same_path(left: &str, right: &str) -> bool {
    left.trim_end_matches('/') == right.trim_end_matches('/')
}

/// 规划三个动作分别使用哪个 customN 条目。
///
/// 先按 command 认领 Clippy 已有的条目（升级或重启后能原地复用，不会每次都新建），
/// 认不出来的再分配一个当前列表里没出现过的编号。纯函数，`command_of` 由调用方注入便于测试。
fn plan_slots(entries: &[String], command_of: impl Fn(&str) -> Option<String>) -> CustomSlots {
    let mut owned: [Option<String>; 3] = [None, None, None];
    for entry in entries {
        let Some(command) = command_of(entry) else {
            continue;
        };
        for (index, method) in CLIPPY_METHODS.iter().enumerate() {
            if owned[index].is_none() && command.contains(method) {
                owned[index] = Some(entry.clone());
                break;
            }
        }
    }

    let mut used: Vec<String> = entries.to_vec();
    let mut next = 0usize;
    let mut allocate = || loop {
        let candidate = custom_path(next);
        next += 1;
        if !used.iter().any(|entry| same_path(entry, &candidate)) {
            used.push(candidate.clone());
            return candidate;
        }
    };

    let [toggle, pin, capture] = owned;
    CustomSlots {
        toggle: toggle.unwrap_or_else(&mut allocate),
        pin: pin.unwrap_or_else(&mut allocate),
        capture: capture.unwrap_or_else(&mut allocate),
    }
}

/// 进程内只解析一次：解析结果决定了后续所有读写的路径，中途变化会写到两个地方去。
fn slots() -> &'static CustomSlots {
    static SLOTS: OnceLock<CustomSlots> = OnceLock::new();
    SLOTS.get_or_init(|| {
        let entries = read_custom_list().unwrap_or_default();
        let planned = plan_slots(&entries, |path| entry_value(path, "command"));
        log::info!(
            "Clippy 自定义快捷键条目: toggle={} pin={} capture={}",
            planned.toggle,
            planned.pin,
            planned.capture
        );
        planned
    })
}

/// 读取单个条目的字段（失败或空值返回 None）
fn entry_value(dconf_path: &str, key: &str) -> Option<String> {
    let target = format!("{ENTRY_SCHEMA}:{dconf_path}");
    let output = Command::new("gsettings")
        .args(["get", &target, key])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout)
        .trim()
        .trim_matches('\'')
        .to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// 读取 `custom-keybindings` 列表
fn read_custom_list() -> Result<Vec<String>, String> {
    let output = Command::new("gsettings")
        .args(["get", SCHEMA, "custom-keybindings"])
        .output()
        .map_err(|e| format!("gsettings get 失败: {e}"))?;
    if !output.status.success() {
        return Err("gsettings get custom-keybindings 返回非零退出码".into());
    }
    let current = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(parse_custom_list(&current))
}

/// 解析 `['/path/custom0/', '/path/custom1/']` 或 `@as []`
fn parse_custom_list(raw: &str) -> Vec<String> {
    if raw.is_empty() || raw.starts_with("@as") {
        return Vec::new();
    }
    raw.trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|item| item.trim().trim_matches('\'').to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// Clippy 占用的三个自定义快捷键路径。占用检测要排除它们，
/// 否则用户修改自己的快捷键会被报成"已被占用"。
pub fn clippy_custom_paths() -> [&'static str; 3] {
    let resolved = slots();
    [
        resolved.toggle.as_str(),
        resolved.pin.as_str(),
        resolved.capture.as_str(),
    ]
}

/// 自定义快捷键条目的 relocatable schema 名
pub fn entry_schema() -> &'static str {
    ENTRY_SCHEMA
}

/// 将 Tauri 快捷键格式转为 GNOME accelerator 格式
///
/// `Ctrl+Alt+V` → `<Control><Alt>v`
/// `Super+V`    → `<Super>v`
pub fn to_gnome_accel(tauri_shortcut: &str) -> String {
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
    if !is_gnome_desktop() {
        return Err(NOT_GNOME_REASON.to_string());
    }
    let accel = to_gnome_accel(shortcut);
    log::info!("注册 GNOME 自定义快捷键: {} -> {}", shortcut, accel);

    ensure_in_custom_list()?;
    let path = slots().toggle.as_str();
    gsettings_set(path, "name", "Clippy Toggle")?;
    gsettings_set(path, "command", DBUS_TOGGLE_CMD)?;
    gsettings_set(path, "binding", &accel)?;
    restart_gsd_media_keys()?;

    log::info!("GNOME 自定义快捷键注册完成");
    Ok(())
}

/// 注册 Pin 快捷键（应用启动时调用）
pub fn register_pin(shortcut: &str) -> Result<(), String> {
    if !is_gnome_desktop() {
        return Err(NOT_GNOME_REASON.to_string());
    }
    let accel = to_gnome_accel(shortcut);
    log::info!("注册 GNOME Pin 快捷键: {} -> {}", shortcut, accel);

    ensure_in_custom_list()?;
    let path = slots().pin.as_str();
    gsettings_set(path, "name", "Clippy Pin")?;
    gsettings_set(path, "command", DBUS_PIN_CMD)?;
    gsettings_set(path, "binding", &accel)?;
    restart_gsd_media_keys()?;

    log::info!("GNOME Pin 快捷键注册完成");
    Ok(())
}

/// 注册 Capture 快捷键（应用启动时调用）
pub fn register_capture(shortcut: &str) -> Result<(), String> {
    if !is_gnome_desktop() {
        return Err(NOT_GNOME_REASON.to_string());
    }
    let accel = to_gnome_accel(shortcut);
    log::info!("注册 GNOME Capture 快捷键: {} -> {}", shortcut, accel);

    ensure_in_custom_list()?;
    let path = slots().capture.as_str();
    gsettings_set(path, "name", "Clippy Screenshot")?;
    gsettings_set(path, "command", DBUS_CAPTURE_CMD)?;
    gsettings_set(path, "binding", &accel)?;
    restart_gsd_media_keys()?;

    log::info!("GNOME Capture 快捷键注册完成");
    Ok(())
}

/// 更新 Pin 快捷键绑定
pub fn update_pin_binding(shortcut: &str) -> Result<(), String> {
    if !is_gnome_desktop() {
        return Err(NOT_GNOME_REASON.to_string());
    }
    let accel = to_gnome_accel(shortcut);
    log::info!("更新 GNOME Pin 快捷键绑定: {}", accel);
    gsettings_set(&slots().pin, "binding", &accel)?;
    restart_gsd_media_keys()
}

/// 更新 Capture 快捷键绑定
pub fn update_capture_binding(shortcut: &str) -> Result<(), String> {
    if !is_gnome_desktop() {
        return Err(NOT_GNOME_REASON.to_string());
    }
    let accel = to_gnome_accel(shortcut);
    log::info!("更新 GNOME Capture 快捷键绑定: {}", accel);
    gsettings_set(&slots().capture, "binding", &accel)?;
    restart_gsd_media_keys()
}

/// 更新绑定（设置页面修改快捷键时调用）
pub fn update_binding(shortcut: &str) -> Result<(), String> {
    if !is_gnome_desktop() {
        return Err(NOT_GNOME_REASON.to_string());
    }
    let accel = to_gnome_accel(shortcut);
    log::info!("更新 GNOME 快捷键绑定: {}", accel);
    gsettings_set(&slots().toggle, "binding", &accel)?;
    restart_gsd_media_keys()
}

/// 暂停快捷键（录制新快捷键时调用）
pub fn pause() -> Result<(), String> {
    if !is_gnome_desktop() {
        // 这条路径下本来就没有注册成功的键位，没有东西需要暂停。
        log::debug!("非 GNOME 桌面，跳过暂停快捷键");
        return Ok(());
    }
    log::info!("暂停 GNOME 快捷键");
    for path in slots().paths() {
        gsettings_set(path, "binding", "")?;
    }
    restart_gsd_media_keys()
}

/// 恢复快捷键：逐个写回 binding，只重启一次 gsd。
///
/// 返回每个动作的 `(动作名, 键位, 结果)`，调用方据此按动作记账——恢复失败同样意味着
/// 用户按键没反应，不能只写日志。非 GNOME 桌面返回空列表：这条链路上本来就没有注册成功的
/// 键位（启动时已上报过），没有东西需要恢复，也不该再报一遍错。
pub fn resume_with_results(
    global_shortcut: &str,
    pin_shortcut: &str,
    capture_shortcut: &str,
) -> Vec<(&'static str, String, Result<(), String>)> {
    if !is_gnome_desktop() {
        log::debug!("非 GNOME 桌面，跳过恢复快捷键");
        return Vec::new();
    }
    let resolved = slots();
    let targets = [
        ("global", resolved.toggle.as_str(), global_shortcut),
        ("pin", resolved.pin.as_str(), pin_shortcut),
        ("capture", resolved.capture.as_str(), capture_shortcut),
    ];
    let results: Vec<(&'static str, String, Result<(), String>)> = targets
        .into_iter()
        .map(|(action, path, shortcut)| {
            let result = gsettings_set(path, "binding", &to_gnome_accel(shortcut));
            (action, shortcut.to_string(), result)
        })
        .collect();
    // 只要写进去了一条就得让 gsd 重新 grab，否则成功的那几个也不生效
    if results.iter().any(|(_, _, result)| result.is_ok()) {
        if let Err(error) = restart_gsd_media_keys() {
            log::warn!("恢复快捷键后重启 gsd-media-keys 失败: {error}");
        }
    }
    results
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
            .args([
                "--user",
                "start",
                "org.gnome.SettingsDaemon.MediaKeys.target",
            ])
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
fn gsettings_set(dconf_path: &str, key: &str, value: &str) -> Result<(), String> {
    let path_arg = format!("{ENTRY_SCHEMA}:{dconf_path}");
    let status = Command::new("gsettings")
        .args(["set", &path_arg, key, value])
        .status()
        .map_err(|e| format!("gsettings set {key} 失败: {e}"))?;
    if !status.success() {
        return Err(format!("gsettings set {key} 返回非零退出码"));
    }
    Ok(())
}

/// 序列化 `custom-keybindings` 列表（空列表必须写 `@as []`，否则 gsettings 拒绝）
fn format_custom_list(entries: &[String]) -> String {
    if entries.is_empty() {
        return "@as []".to_string();
    }
    let inner: Vec<String> = entries.iter().map(|entry| format!("'{entry}'")).collect();
    format!("[{}]", inner.join(", "))
}

fn write_custom_list(entries: &[String]) -> Result<(), String> {
    let status = Command::new("gsettings")
        .args([
            "set",
            SCHEMA,
            "custom-keybindings",
            &format_custom_list(entries),
        ])
        .status()
        .map_err(|e| format!("gsettings set custom-keybindings 失败: {e}"))?;
    if !status.success() {
        return Err("gsettings set custom-keybindings 返回非零退出码".into());
    }
    Ok(())
}

/// 确保 Clippy 使用的自定义快捷键路径存在于列表中
fn ensure_in_custom_list() -> Result<(), String> {
    let mut entries = read_custom_list()?;
    let mut needs_update = false;
    for path in slots().paths() {
        if !entries.iter().any(|entry| same_path(entry, path)) {
            entries.push(path.to_string());
            needs_update = true;
        }
    }
    if !needs_update {
        return Ok(());
    }
    write_custom_list(&entries)
}

/// 从自定义快捷键列表中移除 Clippy 路径
fn remove_from_custom_list() -> Result<(), String> {
    let entries = read_custom_list()?;
    let own = slots().paths();
    let kept: Vec<String> = entries
        .iter()
        .filter(|entry| !own.iter().any(|path| same_path(entry, path)))
        .cloned()
        .collect();
    if kept.len() == entries.len() {
        return Ok(());
    }
    write_custom_list(&kept)
}

/// dconf reset 清空条目数据
fn dconf_reset() -> Result<(), String> {
    for path in slots().paths() {
        let status = Command::new("dconf")
            .args(["reset", "-f", path])
            .status()
            .map_err(|e| format!("dconf reset 失败: {e}"))?;
        if !status.success() {
            return Err("dconf reset 返回非零退出码".into());
        }
    }
    Ok(())
}

// ─── D-Bus 服务 ──────────────────────────────────────────────────────────────

/// 启动 D-Bus 服务：注册 com.clippy.app.Shortcuts，暴露 Toggle 方法
///
/// 使用 .Shortcuts 子名称，因为 enableGTKAppId=true 时 GTK GApplication 会占用 com.clippy.app。
///
/// `ready_tx` 在 name 抢占完成后发送结果（成功/失败）。失败 = 已有实例占用此名，
/// 调用方应据此立即退出进程，避免变成幽灵实例（v0.1.6 开机自启幽灵进程的根因）。
pub async fn start_dbus_service(
    handle: AppHandle,
    ready_tx: std::sync::mpsc::Sender<Result<(), String>>,
) -> Result<(), Box<dyn std::error::Error>> {
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

        fn pin_current(&self) {
            log::info!("D-Bus PinCurrent 被调用");
            // 通知前端执行 pin（不显示剪贴板面板）
            use tauri::Emitter;
            let _ = self.handle.emit("pin-current", ());
        }

        fn capture(&self) {
            log::info!("D-Bus Capture 被调用");
            let handle = self.handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = super::commands::show_capture_editor_for_app(handle).await {
                    log::warn!("截图快捷键触发失败: {}", e);
                }
            });
        }
    }

    let iface = ClippyInterface {
        handle: handle.clone(),
    };

    // name 抢占必须先完成，结果立即同步回 setup 主线程
    let conn_result = Builder::session()
        .map_err(|e| e.to_string())
        .and_then(|b| {
            b.name("com.clippy.app.Shortcuts")
                .map_err(|e| e.to_string())
        })
        .and_then(|b| {
            b.serve_at("/com/clippy/app", iface)
                .map_err(|e| e.to_string())
        });

    let builder = match conn_result {
        Ok(b) => b,
        Err(e) => {
            let _ = ready_tx.send(Err(e.clone()));
            return Err(e.into());
        }
    };

    let _conn = match builder.build().await {
        Ok(c) => c,
        Err(e) => {
            let msg = e.to_string();
            let _ = ready_tx.send(Err(msg.clone()));
            return Err(msg.into());
        }
    };

    log::info!("D-Bus 服务已启动: com.clippy.app.Shortcuts");
    let _ = ready_tx.send(Ok(()));

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
        assert_eq!(
            to_gnome_accel("CommandOrControl+Shift+A"),
            "<Control><Shift>a"
        );
        assert_eq!(to_gnome_accel("Cmd+V"), "<Super>v");
        assert_eq!(to_gnome_accel("Meta+V"), "<Super>v");
    }

    #[test]
    fn test_is_wayland() {
        let _ = is_wayland();
    }

    #[test]
    fn gnome_detection_reads_both_desktop_variables() {
        // Ubuntu 的 XDG_CURRENT_DESKTOP 是冒号分隔的复合值
        assert!(is_gnome_desktop_with(Some("ubuntu:GNOME"), None));
        assert!(is_gnome_desktop_with(Some("GNOME"), None));
        assert!(is_gnome_desktop_with(None, Some("gnome")));
        // KDE/wlroots 上没有 gsd-media-keys，必须判为不支持
        assert!(!is_gnome_desktop_with(Some("KDE"), Some("plasmawayland")));
        assert!(!is_gnome_desktop_with(Some("sway"), None));
        assert!(!is_gnome_desktop_with(None, None));
    }

    fn paths(indices: &[usize]) -> Vec<String> {
        indices.iter().map(|index| custom_path(*index)).collect()
    }

    #[test]
    fn plans_lowest_free_slots_on_empty_desktop() {
        let planned = plan_slots(&[], |_| None);
        assert_eq!(planned.toggle, custom_path(0));
        assert_eq!(planned.pin, custom_path(1));
        assert_eq!(planned.capture, custom_path(2));
    }

    #[test]
    fn never_overwrites_foreign_entries() {
        // 用户自己的三个快捷键已经占了 custom0/1/2，Clippy 必须往后排
        let entries = paths(&[0, 1, 2]);
        let planned = plan_slots(&entries, |_| Some("firefox".to_string()));
        assert_eq!(planned.toggle, custom_path(3));
        assert_eq!(planned.pin, custom_path(4));
        assert_eq!(planned.capture, custom_path(5));
    }

    #[test]
    fn reclaims_its_own_entries_by_command() {
        // 上次运行留下的条目（顺序被打乱）必须原地复用，而不是每次启动新建三个
        let entries = paths(&[0, 1, 2, 3]);
        let planned = plan_slots(&entries, |path| {
            if path == custom_path(3) {
                Some(DBUS_TOGGLE_CMD.to_string())
            } else if path == custom_path(1) {
                Some(DBUS_CAPTURE_CMD.to_string())
            } else {
                None
            }
        });
        assert_eq!(planned.toggle, custom_path(3));
        assert_eq!(planned.capture, custom_path(1));
        // pin 认不出来，分配一个没被占用的编号
        assert_eq!(planned.pin, custom_path(4));
    }

    #[test]
    fn slot_paths_are_distinct_even_with_gaps() {
        let entries = paths(&[1, 3]);
        let planned = plan_slots(&entries, |_| None);
        let resolved: Vec<String> = planned
            .paths()
            .iter()
            .map(|path| path.to_string())
            .collect();
        assert_eq!(resolved, paths(&[0, 2, 4]));
    }

    #[test]
    fn custom_list_round_trips() {
        assert!(parse_custom_list("@as []").is_empty());
        assert!(parse_custom_list("").is_empty());
        let entries = parse_custom_list("['/a/custom0/', '/a/custom5/']");
        assert_eq!(entries, vec!["/a/custom0/", "/a/custom5/"]);
        assert_eq!(
            format_custom_list(&entries),
            "['/a/custom0/', '/a/custom5/']"
        );
        assert_eq!(format_custom_list(&[]), "@as []");
        // 手工写入的路径可能缺末尾斜杠，不能因此重复添加
        assert!(same_path("/a/custom0", "/a/custom0/"));
    }
}
