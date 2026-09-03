# Clippy 对 libspa 0.9.2 的补丁

此目录来自 crates.io 的 `libspa 0.9.2`，原始 `.crate` SHA-256 为
`b6b8cfa2a7656627b4c92c6b9ef929433acd673d5ab3708cda1b18478ac00df4`。
保留上游 MIT 许可证和 Cargo 发布元数据；仓库根应用通过 `[patch.crates-io]` 固定使用本目录。

只维护以下兼容差异：

1. `src/param/video/raw.rs`：使用 C POD 合法的全零初态构造 `VideoInfoRaw`，并按
   `v0_3_65` feature 隔离 `flags`/`modifier` 字段。上游字段字面量无法在 Ubuntu 22
   的 PipeWire 0.3.48 头文件上编译。
2. `src/utils/dict.rs`：补全返回迭代器的显式生命周期。只消除当前 Rust 的
   `mismatched_lifetime_syntaxes` 警告，不改变行为。
3. `build.rs`：删除已经弃用且不产生效果的 `cc::Build::shared_flag(true)`，避免
   `-D warnings` 失败；`cc` crate 会按调用 `compile()` 的既有行为生成测试辅助库。

升级 libspa 时必须重新验证 Ubuntu 22/24 容器编译、默认依赖图和 GNOME Wayland
真机截图性能；不得仅删除本补丁后改回 PNG 兜底。
