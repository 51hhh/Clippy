# Clippy 对 shiguredo_libvpx 的补丁

上游基线：`shiguredo_libvpx 2026.2.0-canary.1`，crates.io 包对应上游提交
`6dc4ed275ebe1018cbc2f862c36c814c5f817563`，许可证为 Apache-2.0。

Clippy 只修改 `build.rs` 的外部输入校验来源：

- 移除从归档所在 GitHub Release 动态下载 `.sha256` 的行为；
- 在仓库中固定已经审阅的归档 SHA-256；
- 未列入哈希表的平台立即失败，禁止自动接受新归档；
- 当前固定 Ubuntu 22.04/24.04/26.04 x86_64、Windows x86_64 与 macOS arm64；
- 上游没有 macOS x86_64 预编译归档；该目标启用 `source-build` 时改为下载 libvpx v1.16.0
  源码归档，并用仓库固定 SHA-256 校验后再编译；
- 移除源码构建时按可变 tag 浅克隆 Git 仓库的行为。

预编译归档仍按固定 crate 版本从上游 GitHub Release 下载，源码归档来自 libvpx 官方 GitHub tag
archive；网络不可用时 feature 构建会失败。默认产品构建不启用 `recording-vp9-prototype`，不会下载
或链接 libvpx。
