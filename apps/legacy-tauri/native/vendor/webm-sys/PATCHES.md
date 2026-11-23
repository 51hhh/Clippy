# Clippy patches for `webm-sys 2.2.1`

Upstream: <https://github.com/DiamondLovesYou/rust-webm>

The bundled libwebm already implements `Track::set_codec_delay`, `Track::set_seek_pre_roll`,
`SegmentInfo::set_timecode_scale`, and `Segment::AddFrameWithDiscardPadding`. Clippy exposes those
four operations through the crate's existing C ABI and rejects null/empty frame and codec-private
inputs at that boundary. No bundled libwebm source file is modified.

## WIN-WEBM-MSVC-01：按编译器族选择参数

来源：crates.io `webm-sys 2.2.1`，上游 Git
`dc9c7caf9a3f29239d47b8b6dcb41a049caf2572`，路径 `src/sys`。
原始 archive SHA-256：`4573631d064f24e233a9cd6e5764eef0f364f1248444c0049d047a7784e613a2`。
原始 `build.rs` SHA-256：`eda0ed58d90a1a492291df6755c91d8f40faa489409f185e6e0db5e27251af6b`。
补丁 `build.rs` SHA-256：`c7e90a7f5cd1e24d0cbe355442c21cfaf15d27d13ea0bf64f1d86e37f6b1b833`。

`cc::Tool::is_like_msvc()` 同时识别 MSVC 和 clang-cl，两者跳过原先被忽略的
`-fno-rtti`、`-std=gnu++11`、`-fno-exceptions`，沿用既有有效模式。GCC/Clang 保留原参数。
本机 clang-cl 23.1.2 的前后预处理探针均为 C++14、RTTI 开启、异常关闭；六个原生编译命令通过。
MSVC 六个编译命令前后对照验证 D9002 消失；编译和诊断探针不等于桌面录屏验收。

原 archive 的 29 个 libwebm C/C++ 源码文件与 vendor 字节完全一致。既有许可证正文一致，vendor
仅比原 archive 少一个末尾空行；本补丁保持现有许可证、libwebm 和 Opus FFI 字节。
供应链验证对补丁构建脚本执行原始 SHA-256；Git 属性固定 LF，不能归一化输入掩盖源码修改。
