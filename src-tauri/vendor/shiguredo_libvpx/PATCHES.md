# Clippy 对 shiguredo_libvpx 的补丁

上游基线：`shiguredo_libvpx 2026.2.0-canary.1`，crates.io 包对应上游提交
`6dc4ed275ebe1018cbc2f862c36c814c5f817563`，许可证为 Apache-2.0。

Clippy 只修改 `build.rs` 的外部输入校验来源：

- 移除从归档所在 GitHub Release 动态下载 `.sha256` 的行为；
- 在仓库中固定已经审阅的归档 SHA-256；
- 未列入哈希表的平台立即失败，禁止自动接受新归档；
- 当前固定 Ubuntu 22.04/24.04/26.04 x86_64、Windows GNU x86_64 与 macOS arm64；
- 上游 Windows 预编译归档使用 MinGW/pthread ABI，不能链接 Clippy 的 MSVC 目标；Windows MSVC
  启用 `source-build` 后从固定源码归档生成 Visual Studio 17/v143 工程，并用 MSBuild 构建；
- libvpx 1.16.0 的 VS 工程生成器会拒绝 external-build 泄漏的 `-O3`；按 Microsoft vcpkg 同版补丁
  忽略这类未知 GCC flag。替换只允许命中固定源码中的唯一一行，源码结构变化时立即失败；
- VS Release 工程关闭 LTCG，输出标准 COFF 对象，使现有的 `llvm-objcopy` 符号隔离步骤能够读取并
  重写静态库；优化级别仍为 `MaxSpeed`，只关闭全程序优化；
- 上游没有 macOS x86_64 预编译归档；该目标启用 `source-build` 时改为下载 libvpx v1.16.0
  源码归档，并用仓库固定 SHA-256 校验后再编译；
- 移除源码构建时按可变 tag 浅克隆 Git 仓库的行为。
- 将 `EncoderConfig::lag_in_frames` 从 `Option<NonZeroUsize>` 放宽为 `Option<usize>`，允许录屏原型
  显式设置 libvpx 的 `g_lag_in_frames = 0`。这使周期分段能在不结束连续编码器的情况下立即取得
  边界前全部压缩 packet；默认值仍为 `None`，不改变其他调用方。

预编译归档仍按固定 crate 版本从上游 GitHub Release 下载，源码归档来自 libvpx 官方 GitHub tag
archive；网络不可用时 feature 构建会失败。默认产品构建不启用 `recording-vp9-prototype`，不会下载
或链接 libvpx。
