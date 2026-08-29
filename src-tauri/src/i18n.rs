//! 后端原生文案的本地化。
//!
//! 托盘菜单和 Rust 侧创建的窗口标题不经过前端 i18n（它们在 webview 之外），
//! 所以这里按 `AppConfig.language` 取一份静态文案。语言集合与 `src/i18n/*.json`
//! 保持一致：显式的 `en` / `zh-CN`，`auto` 时按环境变量判断，其余一律回退英文。

/// 支持的界面语言。新增语言时同时补 `native_text` 的分支，编译器会提醒漏项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    En,
    ZhCn,
}

/// 后端需要的全部用户可见文案。用 `&'static str` 而不是查表，
/// 漏字段在编译期就会报错，不会在运行时退化成 key 字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeText {
    pub open_clipboard: &'static str,
    pub settings_menu: &'static str,
    pub quit_menu: &'static str,
    pub settings_title: &'static str,
    pub screenshot_title: &'static str,
}

pub fn native_text(locale: Locale) -> NativeText {
    match locale {
        Locale::En => NativeText {
            open_clipboard: "Open Clipboard",
            settings_menu: "Settings",
            quit_menu: "Quit",
            settings_title: "Clippy Settings",
            screenshot_title: "Clippy Screenshot",
        },
        Locale::ZhCn => NativeText {
            open_clipboard: "打开剪贴板",
            settings_menu: "设置",
            quit_menu: "退出",
            settings_title: "Clippy 设置",
            screenshot_title: "Clippy 截图",
        },
    }
}

/// 按配置值取文案，等价于 `native_text(resolve_locale(configured))`。
pub fn text_for_language(configured: &str) -> NativeText {
    native_text(resolve_locale(configured))
}

/// 解析配置里的语言值。与前端 `i18n.js::resolveLocale` 行为一致：
/// 显式值优先且不支持时回退英文，只有 `auto`（或空）才看环境。
pub fn resolve_locale(configured: &str) -> Locale {
    resolve_locale_with(configured, environment_tag().as_deref())
}

/// 环境变量作为参数传入，便于测试且不需要在测试里改进程环境。
pub fn resolve_locale_with(configured: &str, environment: Option<&str>) -> Locale {
    match configured {
        "en" => Locale::En,
        "zh-CN" => Locale::ZhCn,
        "" | "auto" => environment.map(locale_from_tag).unwrap_or(Locale::En),
        _ => Locale::En,
    }
}

/// POSIX locale 标签（如 `zh_CN.UTF-8`）到界面语言。`C` / `POSIX` 落到英文。
fn locale_from_tag(tag: &str) -> Locale {
    if tag.to_ascii_lowercase().starts_with("zh") {
        Locale::ZhCn
    } else {
        Locale::En
    }
}

/// 依次读 `LC_ALL` / `LC_MESSAGES` / `LANG`，取第一个非空值。
fn environment_tag() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|value| value.to_string_lossy().to_string())
        .find(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_language_wins_over_environment() {
        assert_eq!(resolve_locale_with("en", Some("zh_CN.UTF-8")), Locale::En);
        assert_eq!(
            resolve_locale_with("zh-CN", Some("en_US.UTF-8")),
            Locale::ZhCn
        );
    }

    #[test]
    fn unsupported_language_falls_back_to_english_without_consulting_environment() {
        // 前端对无法识别的显式值也是直接回退英文，不再看浏览器语言。
        assert_eq!(resolve_locale_with("de", Some("zh_CN.UTF-8")), Locale::En);
        assert_eq!(resolve_locale_with("zh", Some("zh_CN.UTF-8")), Locale::En);
    }

    #[test]
    fn auto_follows_the_environment_tag() {
        assert_eq!(
            resolve_locale_with("auto", Some("zh_CN.UTF-8")),
            Locale::ZhCn
        );
        assert_eq!(resolve_locale_with("auto", Some("zh_TW")), Locale::ZhCn);
        assert_eq!(
            resolve_locale_with("auto", Some("ZH_cn.utf8")),
            Locale::ZhCn
        );
        assert_eq!(resolve_locale_with("auto", Some("en_US.UTF-8")), Locale::En);
        assert_eq!(resolve_locale_with("auto", Some("C")), Locale::En);
        assert_eq!(resolve_locale_with("auto", None), Locale::En);
        assert_eq!(resolve_locale_with("", None), Locale::En);
    }

    #[test]
    fn both_locales_provide_every_string() {
        for locale in [Locale::En, Locale::ZhCn] {
            let text = native_text(locale);
            for value in [
                text.open_clipboard,
                text.settings_menu,
                text.quit_menu,
                text.settings_title,
                text.screenshot_title,
            ] {
                assert!(!value.trim().is_empty(), "{locale:?} 存在空文案");
            }
        }
    }

    #[test]
    fn chinese_menu_text_is_translated_not_english() {
        let english = native_text(Locale::En);
        let chinese = native_text(Locale::ZhCn);

        assert_eq!(chinese.open_clipboard, "打开剪贴板");
        assert_eq!(chinese.settings_menu, "设置");
        assert_eq!(chinese.quit_menu, "退出");
        // 品牌名保留，只翻译后半段。
        assert_eq!(chinese.settings_title, "Clippy 设置");
        assert_ne!(chinese.screenshot_title, english.screenshot_title);
    }

    #[test]
    fn text_for_language_matches_resolved_locale() {
        assert_eq!(text_for_language("zh-CN"), native_text(Locale::ZhCn));
        assert_eq!(text_for_language("en"), native_text(Locale::En));
    }
}
