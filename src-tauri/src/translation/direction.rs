//! 翻译方向：目标语言与「源语言 = 目标语言」时的自动换向。
//!
//! 行为参考 translator 的 `language_direction`（GPL-3.0-only，未复制其代码）：
//! 用户没有显式指定源语言时，按字符集粗判文本语言；若粗判结果就是目标语言，
//! 就换到备选语言，避免出现「英文翻成英文」这类无意义请求。
//!
//! 粗判**只**用于决定是否换向：发给 provider 的源语言仍然是 `auto`，
//! 因此粗判错了最多是没换向或多换了一次向，不会让译文质量变差。

use crate::models::AppConfig;

/// 交给服务自行检测源语言的取值，同时也是配置里「自动」的取值。
pub const AUTO_LANGUAGE: &str = "auto";
/// 配置与备选语言都为空时的兜底语言对。
const FALLBACK_TARGET: &str = "en";
const FALLBACK_COUNTERPART: &str = "zh";

/// 一次请求最终使用的翻译方向。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslationDirection {
    /// 发给 provider 的源语言；`auto` 表示由服务检测。
    pub source: String,
    pub target: String,
}

/// 解析本次请求的翻译方向。
///
/// `source_override` / `target_override` 来自调用方（选区翻译等）：显式给出的目标语言
/// 表示调用方明确知道要译成什么，此时不再换向。
pub fn resolve_direction(
    config: &AppConfig,
    text: &str,
    source_override: Option<&str>,
    target_override: Option<&str>,
) -> TranslationDirection {
    let explicit_source = explicit_language(source_override)
        .or_else(|| explicit_language(Some(&config.translation_source_language)));
    let source = explicit_source
        .clone()
        .unwrap_or_else(|| AUTO_LANGUAGE.to_string());

    if let Some(target) = explicit_language(target_override) {
        return TranslationDirection { source, target };
    }

    let target = explicit_language(Some(&config.translation_target_language))
        .unwrap_or_else(|| FALLBACK_TARGET.to_string());
    // 用户指定了源语言就直接用它判断，否则退回字符集粗判。
    let effective_source =
        explicit_source.or_else(|| detect_script_language(text).map(str::to_string));
    let target = match effective_source {
        Some(effective) if same_language(&effective, &target) => swapped_target(config, &effective),
        _ => target,
    };

    TranslationDirection { source, target }
}

/// 空串与 `auto` 都表示「没有指定」，统一折叠成 None。
fn explicit_language(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case(AUTO_LANGUAGE))
        .map(str::to_string)
}

/// 换向后的目标语言：备选语言里第一个与当前文本语言不同的。
fn swapped_target(config: &AppConfig, source: &str) -> String {
    preferred_languages(config)
        .into_iter()
        .find(|language| !same_language(language, source))
        .unwrap_or_else(|| default_counterpart(source).to_string())
}

/// 备选语言列表，按优先级排列且至少两项，否则换向没有落点。
/// 未配置时用配置里的目标/源语言，这正是用户关心的语言对。
fn preferred_languages(config: &AppConfig) -> Vec<String> {
    let mut languages: Vec<String> = Vec::new();
    for language in &config.preferred_languages {
        push_unique(&mut languages, explicit_language(Some(language)));
    }
    if languages.is_empty() {
        for language in [
            &config.translation_target_language,
            &config.translation_source_language,
        ] {
            push_unique(&mut languages, explicit_language(Some(language)));
        }
    }
    if languages.is_empty() {
        languages.push(FALLBACK_TARGET.to_string());
    }
    if languages.len() == 1 {
        let counterpart = default_counterpart(&languages[0]).to_string();
        push_unique(&mut languages, Some(counterpart));
    }
    languages
}

fn push_unique(languages: &mut Vec<String>, language: Option<String>) {
    let Some(language) = language else { return };
    if !languages
        .iter()
        .any(|existing| same_language(existing, &language))
    {
        languages.push(language);
    }
}

/// 只有一项备选语言时的对手语言。中英互译是最常见的场景，其余语言配英文。
fn default_counterpart(language: &str) -> &'static str {
    if language_key(language) == "en" {
        FALLBACK_COUNTERPART
    } else {
        FALLBACK_TARGET
    }
}

fn same_language(left: &str, right: &str) -> bool {
    language_key(left) == language_key(right)
}

/// 归一化语言标签用于比较。`zh-CN` 与 `zh-Hans` 视为同一语言，
/// 但简体与繁体必须分开：目标是简体时，繁体文本仍然需要翻译。
fn language_key(language: &str) -> String {
    let normalized = language.trim().replace('_', "-").to_ascii_lowercase();
    for prefix in ["zh-hant", "zh-tw", "zh-hk", "zh-mo"] {
        if normalized.starts_with(prefix) {
            return "zh-hant".to_string();
        }
    }
    if normalized == "zh" || normalized.starts_with("zh-") {
        return "zh-hans".to_string();
    }
    normalized.split('-').next().unwrap_or_default().to_string()
}

/// 按字符集粗判文本语言，判不出返回 None（例如纯数字或纯符号）。
///
/// 拉丁字母无法区分英/西/法/德，一律返回 `en`：这意味着目标语言是 `de` 时，
/// 德语文本不会触发换向，provider 会把德语「译成」德语并原样返回。
/// 需要精确判断时用户可以直接指定源语言。
fn detect_script_language(text: &str) -> Option<&'static str> {
    let mut latin = 0usize;
    let mut han = 0usize;
    let mut kana = 0usize;
    let mut hangul = 0usize;
    let mut cyrillic = 0usize;
    let mut arabic = 0usize;

    for ch in text.chars() {
        match ch as u32 {
            0x41..=0x5a | 0x61..=0x7a | 0xc0..=0x24f => latin += 1,
            0x3041..=0x3096 | 0x30a1..=0x30fa => kana += 1,
            0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff => han += 1,
            0x1100..=0x11ff | 0x3130..=0x318f | 0xac00..=0xd7af => hangul += 1,
            0x400..=0x4ff => cyrillic += 1,
            0x600..=0x6ff | 0x750..=0x77f => arabic += 1,
            _ => {}
        }
    }

    // 假名只出现在日文里：日文常混用汉字，按数量取最大会把它误判成中文。
    if kana > 0 {
        return Some("ja");
    }
    [
        ("ko", hangul),
        ("zh", han),
        ("ru", cyrillic),
        ("ar", arabic),
        ("en", latin),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .max_by_key(|(_, count)| *count)
    .map(|(language, _)| language)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(source: &str, target: &str, preferred: &[&str]) -> AppConfig {
        AppConfig {
            translation_source_language: source.to_string(),
            translation_target_language: target.to_string(),
            preferred_languages: preferred.iter().map(|value| value.to_string()).collect(),
            ..AppConfig::default()
        }
    }

    #[test]
    fn text_already_in_the_target_language_switches_to_the_next_preferred() {
        let config = config("auto", "en", &["en", "zh", "ja"]);
        let direction = resolve_direction(&config, "Hello there", None, None);
        assert_eq!(direction.target, "zh");
        // 粗判只影响目标语言，源语言仍交给服务检测。
        assert_eq!(direction.source, AUTO_LANGUAGE);
    }

    #[test]
    fn text_in_another_language_keeps_the_configured_target() {
        let config = config("auto", "en", &["en", "zh"]);
        assert_eq!(
            resolve_direction(&config, "你好，世界", None, None).target,
            "en"
        );
        // 判不出语言时同样保持配置的目标语言。
        assert_eq!(
            resolve_direction(&config, "42 :-)", None, None).target,
            "en"
        );
    }

    #[test]
    fn an_explicit_source_language_decides_the_direction_instead_of_the_text() {
        // 用户说这是英文，即使文本是中文也按英文处理：显式设置优先于粗判。
        let config = config("en", "en", &["en", "zh"]);
        let direction = resolve_direction(&config, "你好", None, None);
        assert_eq!(direction.source, "en");
        assert_eq!(direction.target, "zh");
    }

    #[test]
    fn an_explicit_target_override_is_never_switched() {
        let config = config("auto", "en", &["en", "zh"]);
        let direction = resolve_direction(&config, "Hello", None, Some("en"));
        assert_eq!(direction.target, "en");
        // 空串与 auto 不算显式指定，仍然走换向。
        assert_eq!(
            resolve_direction(&config, "Hello", None, Some("  ")).target,
            "zh"
        );
    }

    #[test]
    fn without_preferred_languages_the_configured_pair_is_used() {
        // 源=ja 目标=ja 时换向落到配置里的另一门语言。
        let direction = resolve_direction(&config("ja", "ja", &[]), "テスト", None, None);
        assert_eq!(direction.target, "en");

        // 源语言是 auto 时只有目标语言可用，补一个对手语言才有换向的落点。
        let direction = resolve_direction(&config("auto", "en", &[]), "Hello", None, None);
        assert_eq!(direction.target, "zh");
        let direction = resolve_direction(&config("auto", "zh", &[]), "你好", None, None);
        assert_eq!(direction.target, "en");
    }

    #[test]
    fn a_single_preferred_language_gains_a_counterpart() {
        let direction = resolve_direction(&config("auto", "fr", &["fr"]), "Bonjour", None, None);
        // 拉丁字母粗判为 en，与 fr 不同，因此不换向。
        assert_eq!(direction.target, "fr");
        let direction = resolve_direction(&config("auto", "en", &["en"]), "Hello", None, None);
        assert_eq!(direction.target, "zh");
    }

    #[test]
    fn chinese_variants_compare_as_the_same_language_except_simplified_versus_traditional() {
        assert!(same_language("zh", "zh-CN"));
        assert!(same_language("zh-Hans", "zh_cn"));
        assert!(same_language("zh-TW", "zh-Hant"));
        assert!(!same_language("zh-Hans", "zh-Hant"));
        assert!(same_language("EN", "en-GB"));
        assert!(!same_language("en", "de"));
    }

    #[test]
    fn preferred_languages_ignore_blanks_auto_and_duplicates() {
        let config = config("auto", "en", &["  ", "auto", "en", "zh-CN", "zh"]);
        assert_eq!(preferred_languages(&config), ["en", "zh-CN"]);
    }

    #[test]
    fn script_detection_prefers_kana_over_han_for_japanese() {
        assert_eq!(detect_script_language("日本語のテキスト"), Some("ja"));
        assert_eq!(detect_script_language("这是一段中文"), Some("zh"));
        assert_eq!(detect_script_language("한국어 문장"), Some("ko"));
        assert_eq!(detect_script_language("Привет"), Some("ru"));
        assert_eq!(detect_script_language("مرحبا"), Some("ar"));
        assert_eq!(detect_script_language("Grüße"), Some("en"));
        assert_eq!(detect_script_language("123 —— ()"), None);
    }
}
