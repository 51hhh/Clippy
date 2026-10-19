fn main() {
    println!("cargo:rerun-if-changed=ffi.cpp");
    let files = &[
        "libwebm/mkvmuxer/mkvmuxer.cc",
        "libwebm/mkvmuxer/mkvwriter.cc",
        "libwebm/mkvmuxer/mkvmuxerutil.cc",
        "libwebm/mkvparser/mkvparser.cc",
        "libwebm/mkvparser/mkvreader.cc",
        "ffi.cpp",
    ];
    let mut c = cc::Build::new();
    c.cpp(true);
    c.warnings(false);
    // GNU 参数仅用于 GCC/Clang；MSVC/clang-cl 沿用原先忽略无效参数后的默认模式。
    if !c.get_compiler().is_like_msvc() {
        c.flag("-fno-rtti");
        c.flag("-std=gnu++11");
        c.flag("-fno-exceptions");
    }
    c.include("libwebm");
    for &f in files {
        c.file(f);
    }
    c.compile("libwebmadapter.a");
}
