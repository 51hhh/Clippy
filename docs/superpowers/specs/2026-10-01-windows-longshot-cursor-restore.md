# WIN-LONGSHOT-CURSOR-01 — 自动长截图失败清理保留用户指针

## Goal

修复原生自动长截图在抓帧或目标检查提前失败时，清理路径可能抢回用户已经移动的鼠标位置。
这是 `WIN-NATIVE-01` / W04 的独立代码修复，基于 `1cd09c8`；桌面操作继续停止。

## Requirements

1. 光标恢复 guard 记录本步自动移动的目标点；销毁时即使业务已经提前返回，也要查询当前指针。
2. 仅当指针仍在自动目标点的既有 3 像素容差内时恢复步骤开始位置；用户移出容差后保留新位置。
3. 指针查询失败时不能盲目注入恢复移动；已经明确检测到用户接管的 disarm 路径继续不移动。
4. 正常完成及无人接管的失败仍恢复原位置；保留既有业务错误，不用清理错误覆盖抓帧错误。
5. 生产 guard 与回归共用 RAII 销毁路径，测试使用注入的指针接口，禁止调用本机指针输入 API。

## Acceptance Criteria

- [x] 提前抓帧失败且用户已经移动的回归先失败后通过，销毁 guard 时不产生恢复移动。
- [x] 查询失败、容差边界、无人接管失败和显式 disarm 的合同通过。
- [ ] Windows 默认与录屏 QA 完整原生门禁通过，日志绑定修复源码 SHA。
- [ ] CHANGELOG、总计划与审查记录同步，真实桌面及其它平台边界保留未验证。

## Out of Scope

修改滚动方向、选区坐标、混合 DPI 模型、输入权限、窗口聚焦或 Wayland Portal 路径；
不操作桌面、安装新包、合入 dev 或发布。共享原生 guard 影响 Windows/X11/macOS；
本机 Windows 合同不能替代 X11/macOS 原生编译及三个平台的真实鼠标接管验收。

## Review Evidence

`with_scroll` 的 `capture()?`、指针和窗口检查错误均可能早于显式 `user_interrupted` 检查返回。
旧 `CursorRestore::drop` 只检查 `armed`，随后直接移动到原位置；其自身并未重新查询当前指针。
原需求 `PX-LS-NATIVE-AUTO-01` 已要求用户移动鼠标时保留新位置，本修复补齐提前失败的清理边界。

## Verification

Windows MSVC 红基线保留原有 armed 时无条件恢复行为，仅提取指针副作用接口。
四项新回归为 1 passed / 3 failed，exit 101：用户已移动、查询失败和容差外位置均仍产生恢复移动。
修复后 `capture::longshot::auto_scroll::tests` 八项定向合同通过，exit 0，格式检查通过。
测试使用同一生产 RAII guard 的销毁路径；所有新增测试仅调用 `TestPointer`，不调用系统鼠标 API。
这是代码失败路径的确定性复现，不声称实际 Windows 鼠标接管场景已通过。

红绿源码/差异、日志和哈希位于主检出的 `src-tauri/target/windows-longshot-cursor-red/`。
完整 Windows 默认/录屏 QA 门禁及新 SHA 原生 CI 待运行；桌面操作未执行。
