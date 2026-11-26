use std::{
    fmt::{Display, Write as _},
    sync::atomic::{AtomicBool, Ordering},
};

include!(concat!(env!("OUT_DIR"), "/rust-translations.rs"));

const PREFERENCES: [&str; 3] = ["auto", "en", "zh-CN"];
static CHINESE: AtomicBool = AtomicBool::new(false);

pub(crate) fn preference_index(saved: &str) -> i32 {
    PREFERENCES
        .iter()
        .position(|&choice| choice == saved)
        .unwrap_or(1) as i32
}

pub(crate) fn preference(index: i32) -> Option<&'static str> {
    PREFERENCES.get(index as usize).copied()
}

fn selected_locale(saved: &str, system: Option<&str>) -> &'static str {
    match saved {
        "zh-CN" => "zh-CN",
        "auto"
            if system.is_some_and(|value| {
                value
                    .split(['-', '_'])
                    .next()
                    .is_some_and(|base| base.eq_ignore_ascii_case("zh"))
            }) =>
        {
            "zh-CN"
        }
        _ => "en",
    }
}

pub(crate) fn apply(saved: &str) {
    let system = (saved == "auto").then(sys_locale::get_locale).flatten();
    let selected = selected_locale(saved, system.as_deref());
    slint::select_bundled_translation(selected).expect("应用包含选中的 Slint 词表");
    CHINESE.store(selected == "zh-CN", Ordering::Relaxed);
}

pub(crate) fn tr(english: &str) -> &str {
    if CHINESE.load(Ordering::Relaxed) {
        if let Ok(index) =
            SIMPLIFIED_CHINESE.binary_search_by_key(&english, |&(english, _)| english)
        {
            return SIMPLIFIED_CHINESE[index].1;
        }
    }
    english
}

pub(crate) fn message(english: &str, values: &[&dyn Display]) -> String {
    let mut rest = tr(english);
    let mut result = String::with_capacity(rest.len());
    while let Some((before, field)) = rest.split_once('{') {
        result.push_str(before);
        let (index, after) = field.split_once('}').expect("词条占位符成对");
        let index: usize = index.parse().expect("词条使用数字占位符");
        write!(result, "{}", values[index]).unwrap();
        rest = after;
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_preferences_override_the_system_language() {
        assert_eq!(selected_locale("en", Some("zh-CN")), "en");
        assert_eq!(selected_locale("zh-CN", Some("en-US")), "zh-CN");
        assert_eq!(selected_locale("auto", Some("zh-Hans-SG")), "zh-CN");
        assert_eq!(selected_locale("auto", Some("en-US")), "en");
        assert_eq!(selected_locale("auto", None), "en");
        assert_eq!(selected_locale("unsupported", Some("zh-CN")), "en");
    }

    #[test]
    fn interpolates_without_interpreting_user_values_as_placeholders() {
        assert_eq!(message("{0} · {1}", &[&"folder{1}", &4]), "folder{1} · 4");
    }
}
