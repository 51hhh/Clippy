# VIEWER-ATOMIC-API-01 — 查看器原子更新 API 实现与验证复核

日期：2026-10-19；计划节点：W89 / WIN-NATIVE-01；验证源码：
`71a0a0a85dfb55c12f3d6c08b32f1f4842ea33db`；分支：
`codex/viewer-atomic-api-compat`。需求与边界见
[VIEWER-ATOMIC-API-01](../superpowers/specs/2026-10-04-viewer-atomic-api-compat.md)。

## 实现核对

`src-tauri/src/viewer/manager.rs` 的 `mark_ready`、`invalidate`、
`restore_after_close_failure` 仅把三处 `fetch_update` 入口改为 `try_update`。三处闭包、
`Ordering::AcqRel` / `Ordering::Acquire`、返回值和错误映射均保持；
`src-tauri/src/viewer/tests.rs` 未改。没有增加 `allow(deprecated)`、降低严格 lint、固定旧
Rust 工具链或修改依赖、功能图、测试超时及平台分支。

原失败基线仍是 `228cc935bc0767ed9906f03ff5680cc89e72febe` 的 GitHub Actions run
37166791297；macOS A/V job 111331245300 在 Rust 1.99 严格 Clippy 中报告三处弃用并
exit 101。原日志与包未被本轮成功证据覆盖。

## Linux 本地完整门禁

2026-10-19 在 Linux x86_64、rustc 1.98.0、Node.js 24.19.0 上执行：

```bash
./scripts/ci-local.sh
```

终端 exit 0，总结为 25 passed / 0 failed / 2 skipped。两个跳过项是未设置
`CLIPPY_CROSS_CHECK=1` 的非宿主交叉 lint，以及未设置 `CLIPPY_APPIMAGE_SMOKE=1` 的
AppImage X11 可视 smoke；均按脚本定义保留为跳过，不计通过。

其中默认 Rust 测试为 1319 passed / 0 failed / 14 ignored；前端 Vitest 为 79 个文件
通过、2 个文件跳过，1348 passed / 36 skipped。Rust fmt/check/clippy、X11 录屏闭环、
私有 X11 剪贴板回归、DOM/Xvfb、Canvas/布局 smoke、TypeScript 与 Vite 生产构建均通过。
本层证据只覆盖当前 Linux 宿主和实际执行步骤，不替代非宿主条件代码或真实桌面行为。

## 同 SHA GitHub Actions

[CI Check run 37168304825](https://github.com/51hhh/Clippy/actions/runs/37168304825)
由 `workflow_dispatch` 触发，绑定上述完整 SHA，状态为 completed/success。七个 job 如下：

| Job | 结论 |
| --- | --- |
| Check (ubuntu-22.04) | success |
| Native Check (windows-latest) | success |
| Native Check (macos-latest) | success |
| Recording Codec Prototype (ubuntu-22.04) | success |
| Recording Codec Prototype (windows-latest) | success |
| Recording Codec Prototype (macos-15) | success |
| Recording Codec Prototype (macos-15-intel) | success |

Windows 原生 job 的前端、OCR、Rust check/clippy/test、vendored xcap WGC、WGC
关闭/初始化回滚、注册表缓冲区、CF_HTML、图像预算、DIBV5 与文件视图步骤均成功；
Windows 录屏原型 job 的供应链、lint、VP9/Opus、可恢复 A/V、VP9 session/recovery 与
WASAPI 音频合同步骤均成功。各平台不适用的条件步骤仍按 runner 图显示 skipped，不改写为通过。

## 验收结论与剩余边界

- 三处源码最小替换、原查看器合同和新 SHA 七项 CI 已满足。
- Windows GitHub runner 证据不等于仓库的 Windows 本机完整默认/QA 门禁；该验收继续未勾选。
- Windows 10/11、混合 DPI/多屏、设备、声音、安装升级卸载、真实桌面与交付未执行。
- 旧失败 run/log、W88 的 52 项完成状态快照和原 8R/9AC/47 任务范围均保留。
