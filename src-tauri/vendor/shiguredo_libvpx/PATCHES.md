# Clippy 对 shiguredo_libvpx 的补丁

上游基线：`shiguredo_libvpx 2026.2.0-canary.1`，crates.io 包对应上游提交
`6dc4ed275ebe1018cbc2f862c36c814c5f817563`，许可证为 Apache-2.0。

Clippy 只修改 `build.rs` 的预编译归档校验来源：

- 移除从归档所在 GitHub Release 动态下载 `.sha256` 的行为；
- 在仓库中固定已经审阅的归档 SHA-256；
- 未列入哈希表的平台立即失败，禁止自动接受新归档；
- 当前固定 Ubuntu 22.04/24.04/26.04 x86_64、Windows x86_64 与 macOS arm64；
- 上游没有 macOS x86_64 预编译归档，Clippy 因此不能把该原型设为四目标默认编码器。

归档仍按固定 crate 版本从上游 GitHub Release 下载；网络不可用时 feature 构建会失败。默认产品
构建不启用 `recording-vp9-prototype`，不会下载或链接 libvpx。
