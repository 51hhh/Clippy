use std::{collections::BTreeMap, fmt::Write as _, fs, path::PathBuf};

fn main() {
    println!("cargo:rustc-check-cfg=cfg(embedded_recording_playback)");
    println!("cargo:rustc-check-cfg=cfg(native_recording_playback)");
    let native = matches!(
        std::env::var("CARGO_CFG_TARGET_OS").as_deref(),
        Ok("linux" | "windows")
    ) && std::env::var_os("CARGO_FEATURE_RECORDING_PLAYBACK_NATIVE_QA").is_some();
    if native {
        println!("cargo:rustc-cfg=native_recording_playback");
    }
    if native || std::env::var_os("CARGO_FEATURE_RECORDING_PLAYBACK_GSTREAMER_QA").is_some() {
        println!("cargo:rustc-cfg=embedded_recording_playback");
    }
    let ui = slint_build::CompilerConfiguration::new()
        .with_bundled_translations("i18n")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("slint-project.json", ui).expect("Slint UI 编译失败");
    bundle_rust_messages();
}

fn bundle_rust_messages() {
    let source = PathBuf::from("i18n/zh-CN/LC_MESSAGES/clippy-slint.po");
    println!("cargo:rerun-if-changed={}", source.display());
    let catalog = rspolib::pofile(source.as_path()).expect("中文词表可读取");
    let entries: BTreeMap<_, _> = catalog
        .translated_entries()
        .into_iter()
        .filter(|entry| entry.msgctxt.is_none())
        .filter_map(|entry| Some((entry.msgid.as_str(), entry.msgstr.as_deref()?)))
        .collect();
    let mut output = String::from("static SIMPLIFIED_CHINESE: &[(&str, &str)] = &[\n");
    for (english, chinese) in entries {
        writeln!(output, "    ({english:?}, {chinese:?}),").unwrap();
    }
    output.push_str("];\n");
    let path = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("rust-translations.rs");
    fs::write(path, output).expect("生成 Rust 界面词表");
}
