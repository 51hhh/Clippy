//! shortcut_conflict.rs — 快捷键占用检测
//!
//! 设置页录制快捷键时需要回答"这个组合已经被别人占了吗"。两种会话能给出的答案不同：
//!
//! - GNOME/Wayland：键位由 gsd-media-keys / mutter / gnome-shell 通过 gsettings 声明，
//!   可以逐个 schema 枚举出来做精确比较（Clippy 自己的 custom0/1/2 要排除，
//!   否则修改自己的快捷键会被报成冲突）。
//! - X11：抓键是 XGrabKey，X 服务器不提供"谁抓了哪个键"的枚举接口。
//!   因此只能确认 Clippy 自己有没有注册，其余一律"查不出来"，
//!   由 `enumerable = false` 明确告诉前端这不是"没有冲突"。
//!
//! Clippy 三个动作之间的自冲突不在这里判断：设置页里用户可能已经改了输入框但还没保存，
//! 前端拿到的是更新的值，那一层的比较由 `shortcut-recording.js` 负责。

use crate::gsettings_shortcuts::{clippy_custom_paths, entry_schema, to_gnome_accel};
use std::process::Command;

/// 需要扫描的 GNOME 快捷键 schema。缺失的 schema（非 GNOME 桌面）直接跳过。
const SCAN_SCHEMAS: &[&str] = &[
    "org.gnome.desktop.wm.keybindings",
    "org.gnome.settings-daemon.plugins.media-keys",
    "org.gnome.shell.keybindings",
    "org.gnome.mutter.keybindings",
    "org.gnome.mutter.wayland.keybindings",
];

/// 快捷键占用检测结果
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ShortcutConflict {
    /// 明确检测到冲突
    pub conflicted: bool,
    /// 冲突来源：`desktop` = 桌面已有绑定，`clippy` = 本应用当前已注册，None = 无冲突
    pub source: Option<String>,
    /// 占用者标识（gsettings key 或自定义快捷键的名字），只用于提示与日志
    pub owner: Option<String>,
    /// 本会话能否枚举桌面绑定。false 表示"查不出来"，不等于"没有冲突"
    pub enumerable: bool,
}

impl ShortcutConflict {
    fn none(enumerable: bool) -> Self {
        Self {
            conflicted: false,
            source: None,
            owner: None,
            enumerable,
        }
    }

    fn found(source: &str, owner: String) -> Self {
        Self {
            conflicted: true,
            source: Some(source.to_string()),
            owner: Some(owner),
            enumerable: true,
        }
    }
}

/// 一条已声明的绑定：`(可读的占用者, 该项声明的所有 accelerator)`
pub type Binding = (String, Vec<String>);

/// 归一化 GNOME accelerator：修饰键顺序、别名与大小写都不参与比较。
///
/// `<Primary><Alt>T` 与 `<Alt><Control>t` 归一化后相同；空串与纯修饰键返回 None
/// （GNOME 用 `['']` 表示"未绑定"，不能把它当成一个能被占用的键位）。
pub fn normalize_accel(accel: &str) -> Option<String> {
    let mut rest = accel.trim();
    let mut modifiers: Vec<String> = Vec::new();
    while let Some(start) = rest.find('<') {
        let Some(offset) = rest[start..].find('>') else {
            break;
        };
        let name = &rest[start + 1..start + offset];
        modifiers.push(match name.to_ascii_lowercase().as_str() {
            "primary" | "control" | "ctrl" => "control".to_string(),
            "alt" | "mod1" => "alt".to_string(),
            "shift" => "shift".to_string(),
            "super" | "meta" | "cmd" | "command" | "mod4" => "super".to_string(),
            other => other.to_string(),
        });
        rest = &rest[start + offset + 1..];
    }
    let key = rest.trim().to_ascii_lowercase();
    if key.is_empty() {
        return None;
    }
    modifiers.sort();
    modifiers.dedup();
    let prefix: String = modifiers.iter().map(|m| format!("<{m}>")).collect();
    Some(format!("{prefix}{key}"))
}

/// 解析 `gsettings list-recursively <schema>` 的输出。
///
/// 每行形如 `schema key value`，value 可能是 `['<Alt>F4']`、`['']`、`@as []`
/// 或多个值。取所有单引号内的字符串作为 accelerator 候选。
pub fn parse_bindings(raw: &str) -> Vec<Binding> {
    raw.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, ' ');
            let schema = parts.next()?;
            let key = parts.next()?;
            let value = parts.next().unwrap_or("");
            let accels = quoted_values(value);
            if accels.is_empty() {
                return None;
            }
            Some((format!("{schema} {key}"), accels))
        })
        .collect()
}

/// 取出 `['a', 'b']` 里的 `a`、`b`
fn quoted_values(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find('\'') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('\'') else { break };
        let item = &after[..end];
        if !item.trim().is_empty() {
            out.push(item.to_string());
        }
        rest = &after[end + 1..];
    }
    out
}

/// 在解析结果里找占用同一个 accelerator 的项
pub fn find_owner(bindings: &[Binding], accel: &str) -> Option<String> {
    let target = normalize_accel(accel)?;
    bindings
        .iter()
        .find(|(_, accels)| {
            accels
                .iter()
                .any(|candidate| normalize_accel(candidate).as_deref() == Some(target.as_str()))
        })
        .map(|(owner, _)| owner.clone())
}

/// 检测快捷键是否已被占用。`registered_by_self` 由调用方注入
/// （X11 下是 global-shortcut 插件的 `is_registered`），便于测试不依赖 Tauri。
pub fn detect_with(
    shortcut: &str,
    wayland: bool,
    registered_by_self: impl FnOnce() -> bool,
    scan: impl FnOnce() -> Option<Vec<Binding>>,
) -> ShortcutConflict {
    let accel = to_gnome_accel(shortcut);
    if wayland {
        // GNOME 之外的 Wayland 桌面没有这些 schema，枚举不出任何东西。
        return match scan() {
            Some(bindings) => match find_owner(&bindings, &accel) {
                Some(owner) => ShortcutConflict::found("desktop", owner),
                None => ShortcutConflict::none(true),
            },
            None => ShortcutConflict::none(false),
        };
    }
    // X11：只能看到自己。录制期间快捷键是暂停的，所以这里为真通常意味着
    // 用户把某个动作设成了它自己当前已注册的键位。
    if registered_by_self() {
        return ShortcutConflict::found("clippy", shortcut.to_string());
    }
    ShortcutConflict::none(false)
}

/// 枚举 GNOME 已声明的快捷键。所有 schema 都不可用时返回 None（"查不出来"）。
pub fn scan_gnome_bindings() -> Option<Vec<Binding>> {
    let mut bindings = Vec::new();
    let mut any_schema = false;
    for schema in SCAN_SCHEMAS {
        let Ok(output) = Command::new("gsettings")
            .args(["list-recursively", schema])
            .output()
        else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        any_schema = true;
        bindings.extend(parse_bindings(&String::from_utf8_lossy(&output.stdout)));
    }
    if !any_schema {
        return None;
    }
    bindings.extend(scan_custom_keybindings());
    Some(bindings)
}

/// 自定义快捷键的 binding 不在 `list-recursively` 里（relocatable schema），
/// 需要按路径逐个读；Clippy 自己的三个路径必须排除。
fn scan_custom_keybindings() -> Vec<Binding> {
    let Ok(output) = Command::new("gsettings")
        .args([
            "get",
            "org.gnome.settings-daemon.plugins.media-keys",
            "custom-keybindings",
        ])
        .output()
    else {
        return Vec::new();
    };
    let own = clippy_custom_paths();
    quoted_values(&String::from_utf8_lossy(&output.stdout))
        .into_iter()
        .filter(|path| !own.contains(&path.as_str()))
        .filter_map(|path| {
            let binding = gsettings_entry(&path, "binding")?;
            let accels = quoted_values(&binding);
            let accels = if accels.is_empty() {
                vec![binding.trim().to_string()]
            } else {
                accels
            };
            let name = gsettings_entry(&path, "name")
                .map(|value| value.trim().trim_matches('\'').to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| path.clone());
            Some((name, accels))
        })
        .collect()
}

fn gsettings_entry(path: &str, key: &str) -> Option<String> {
    let target = format!("{}:{}", entry_schema(), path);
    let output = Command::new("gsettings")
        .args(["get", &target, key])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_ignores_modifier_order_and_aliases() {
        assert_eq!(
            normalize_accel("<Primary><Alt>T"),
            normalize_accel("<Alt><Control>t")
        );
        assert_eq!(normalize_accel("<Super>V"), normalize_accel("<Meta>v"));
        assert_eq!(normalize_accel("<Alt>F4").unwrap(), "<alt>f4");
        // GNOME 用空串表示未绑定，不能当成一个可被占用的键位
        assert!(normalize_accel("").is_none());
        assert!(normalize_accel("<Control>").is_none());
    }

    #[test]
    fn parse_skips_unbound_entries() {
        let raw = "org.gnome.desktop.wm.keybindings close ['<Alt>F4']\n\
                   org.gnome.desktop.wm.keybindings always-on-top @as []\n\
                   org.gnome.settings-daemon.plugins.media-keys calculator ['']\n\
                   org.gnome.shell.keybindings toggle-overview ['<Super>s', '<Super>S']\n";
        let bindings = parse_bindings(raw);
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].0, "org.gnome.desktop.wm.keybindings close");
        assert_eq!(bindings[1].1, vec!["<Super>s", "<Super>S"]);
    }

    #[test]
    fn finds_desktop_owner_regardless_of_spelling() {
        let bindings = parse_bindings(
            "org.gnome.desktop.wm.keybindings close ['<Alt>F4']\n\
             org.gnome.shell.keybindings screenshot ['<Shift><Primary>S']\n",
        );
        assert_eq!(
            find_owner(&bindings, "<Control><Shift>s").as_deref(),
            Some("org.gnome.shell.keybindings screenshot")
        );
        assert!(find_owner(&bindings, "<Alt>v").is_none());
    }

    #[test]
    fn wayland_reports_desktop_conflict() {
        let bindings =
            parse_bindings("org.gnome.shell.keybindings screenshot ['<Shift><Control>S']\n");
        let conflict = detect_with("Ctrl+Shift+S", true, || false, || Some(bindings.clone()));
        assert!(conflict.conflicted);
        assert_eq!(conflict.source.as_deref(), Some("desktop"));
        assert!(conflict.enumerable);

        let free = detect_with("Alt+V", true, || false, || Some(bindings));
        assert!(!free.conflicted);
        assert!(free.enumerable);
    }

    #[test]
    fn non_gnome_wayland_is_not_enumerable() {
        let conflict = detect_with("Alt+V", true, || false, || None);
        assert!(!conflict.conflicted);
        assert!(!conflict.enumerable);
    }

    #[test]
    fn x11_only_sees_its_own_registration() {
        let mine = detect_with("Alt+V", false, || true, || unreachable!());
        assert!(mine.conflicted);
        assert_eq!(mine.source.as_deref(), Some("clippy"));

        let unknown = detect_with("Alt+V", false, || false, || unreachable!());
        assert!(!unknown.conflicted);
        // X11 拿不到别人的 grab，必须让前端知道这是"查不出来"
        assert!(!unknown.enumerable);
    }
}
