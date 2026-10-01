# Windows PowerShell 原生命令版本检查

需求 ID：`WIN-PS-GATE-01`。父任务：`WIN-NATIVE-01 / W02`。
基线：`e5bfd9b0e33f9f027d2725dcd4c470938b920781`。

## Goal

Windows 自带 PowerShell 5.1 与 PowerShell 7 均可进入原生门禁；支持的 Node.js 不被参数引号改写误拒绝。

## Requirements

1. 使用真实外部 Node.js 获取版本，避免向 Windows 原生命令传递含内嵌双引号的 JavaScript。
2. 保留 Node.js >= 22.12.0 门槛；版本不足或命令失败仍在前置检查失败，不虚报通过。
3. Windows 合同覆盖真实 Node/PowerShell 参数传输与不支持版本；既有失败计数和部分检查断言保留。
4. 验证限定当前 Windows 11；源码原生测试、CI、桌面和安装验收分别记录。

## Acceptance Criteria

- [x] Windows PowerShell 5.1 + Node 24.21.0 复现原入口引号解析失败，保存非零退出码。
- [x] 修复后真实 PowerShell 5.1 与 PowerShell 7 + Node 可通过前置版本检查。
- [x] Node 22.11/旧主版本仍被拒绝，最低支持版本与后续主版本被接受。
- [ ] 最新修复 SHA 的完整 `ci-windows.ps1 -RecordingQa` 通过；跳过项不计通过。
- [ ] CHANGELOG 与 PR 引用同一需求 ID；远程 CI、Windows 桌面与安装未执行时保留未完成。

## Out of Scope

- 修改系统执行策略、默认 feature、录屏发布门控、Windows 证书信任或安装 Clippy。
- Linux/WSL 验证；用户明确要求当前阶段只测试 Windows。
- 合入 dev、发布；Windows 10、多屏和 39 项 Windows 11 桌面 QA 保持未验证。

## Verification

原始失败：Windows 11 x64 / Windows PowerShell 5.1 / Node 24.21.0 / Rust MSVC 1.98.1。
原门禁将 `split(".")` 传到 Node 后成为 `split(.)`，触发 SyntaxError，再错误报告 Node 版本不足。
原 SHA e5bfd9b 的门禁 exit 1，未运行任何编译或前端检查。
证据：调用项目 ignored `src-tauri/target/windows-latest-native-qa-e5bfd9b/`。
修复后 `windows-gate.test.js` 13 项合同通过：真实 Windows PowerShell 5.1 与 PowerShell 7
均执行外部 Node；22.11.0/20.19.0 拒绝，22.12.0/23.0.0 接受，版本输出有效但 exit 17 仍拒绝。
既有六项失败计数、缺依赖与部分检查合同保持通过。修复前真实 5.1 合同失败、7 合同通过。
完整 Windows 原生/录屏门禁和修改后同 SHA CI 尚未执行；桌面与安装保持未验证。
Linux 门禁未启动，按用户要求停止该路线。
