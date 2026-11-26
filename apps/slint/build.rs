fn main() {
    println!("cargo:rustc-check-cfg=cfg(embedded_recording_playback)");
    println!("cargo:rustc-check-cfg=cfg(native_recording_playback)");
    let native = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
        && std::env::var_os("CARGO_FEATURE_RECORDING_PLAYBACK_NATIVE_QA").is_some();
    if native {
        println!("cargo:rustc-cfg=native_recording_playback");
    }
    if native || std::env::var_os("CARGO_FEATURE_RECORDING_PLAYBACK_GSTREAMER_QA").is_some() {
        println!("cargo:rustc-cfg=embedded_recording_playback");
    }
    slint_build::compile("slint-project.json").expect("Slint UI 编译失败");
}
