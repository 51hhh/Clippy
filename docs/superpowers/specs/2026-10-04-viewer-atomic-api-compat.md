# VIEWER-ATOMIC-API-01 — 查看器原子更新 API 兼容

## Goal

修复 Rust stable 升至 1.99 后查看器三处原子状态更新导致严格 Clippy 失败的问题。
关联 WIN-NATIVE-01；生产基线 228cc935，前置文档 ba39224。原同 SHA CI run
37166791297 中 macOS A/V job 111331245300 的 Rust 1.99 日志明确报告三处
fetch_update 弃用错误并 exit 101；本机现有工具链为 Rust 1.98.1。原始失败日志保留
在 src-tauri/target/windows-ci-228cc93/FAILED-JOB-111331245300-RAW.json。

## Requirements

1. mark_ready、invalidate、restore_after_close_failure 使用标准库 try_update。
   原闭包、AcqRel/Acquire 顺序、返回值/错误映射、事务边界和后续调用保持。
2. 不添加 allow(deprecated)、降低 -D warnings、固定旧 CI Rust 或增加测试超时；
   不修改 Cargo 依赖、默认功能、原测试正文、平台分支或状态机。
3. 验证现有 Rust 1.98.1 下的查看器状态/关闭/销毁并发合同，并对修复源码执行
   完整原生 Windows 门禁；原用例和断言保持，新 API 不计新增测试用例。
4. 原 228cc935 的失败 run/log/包与本机证据保留；新源码须绑定新 SHA 的七项 CI。
   本机检查、同 SHA CI 与真实设备/安装验收分别记录，原 8R/9AC/47 任务保持。

## Acceptance Criteria

- [ ] 生产源码只替换三处 API 名称，所有闭包、顺序及返回值处理原字节保持。
- [ ] 现有 Rust 1.98.1 查看器合同通过，原 viewer/tests.rs 字节及超时未变。
- [ ] 修复源码的完整 Windows 默认/QA 门禁通过，跳过/ignored 如实记录。
- [ ] 新 SHA 上原七项 CI 在 Rust stable 下成功，原失败证据不被覆盖。

## Out of Scope

不改查看器行为、原子算法、依赖或 CI 门禁策略；不安装新工具链或改认证/系统配置。
桌面、设备、安装更新、Win10/多屏与历史未归因失败仍未验。固定 228cc935 的原验证
分支不移动；新修复的 CI 使用新分支/新 SHA，不把旧包描述为包含新修复。

## Primary references

Rust 官方 [Atomic 文档](https://doc.rust-lang.org/beta/core/sync/atomic/struct.Atomic.html)
记录 fetch_update 是 try_update 的别名、1.99 弃用，try_update 自 1.95 可用。
三处仅更换入口名称；现有 1.98.1 本机编译与当前 stable CI 分别验证版本兼容。
