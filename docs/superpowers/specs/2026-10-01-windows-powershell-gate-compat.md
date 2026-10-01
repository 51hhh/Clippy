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
- [x] 修复 SHA 的完整 `ci-windows.ps1 -RecordingQa` 通过；跳过项不计通过。
- [x] 修复的文档后继 b2fd247 在同 SHA 的三项远程原生 CI 成功；Windows 录屏原型 CI 另行通过。

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
完整 Windows 门禁已在 `ef78a1fbbaa5803356956c8d740dd4ec8572a1ab` 实际运行：
Windows PowerShell 5.1.22000.2538 / Node 24.21.0 / Rust MSVC 1.98.1，exit 0，
23 passed / 0 failed / 1 skipped（Linux smoke）。运行前后 Git 均干净。
默认 Rust 1040 passed / 5 ignored；录屏 QA Rust 1093 passed / 5 ignored，两组不累加。
前端 74 文件 / 1291 passed；Python 33 项质量 + 3 项视觉段落；严格 clippy、WGC lint、
供应链、JS/TS、IPC/HTML、生产构建与真实 built main entry 合同均通过。

结果、环境与返回码在调用项目 ignored `src-tauri/target/windows-latest-native-qa-ef78a1f/`。
Windows PowerShell 父 transcript 未捕获子 stdout，不当作完整测试日志；默认 Rust 的工具
逐测试名输出被 token 预算截断，最终 1040/0/5 汇总保留，QA/前端最终 chunk 完整。
README 式验证报告与分阶段工具输出记录上述边界。后续只更新文档，不将文档 HEAD 写成已运行的 SHA。

远程 CI 证据绑定 `b2fd247c81c8833c5e7e15ddb39642d47fa8780c`，相对本机验证的 ef78a1f
仅两份 Markdown 文档变化。[run 36831731858](https://github.com/51hhh/Clippy/actions/runs/36831731858)
的三项原生检查均 completed/success；Windows 原生 job 110269660965 的完整下载日志记录
前端 1291 passed、Rust 1040 passed / 5 ignored、Python 33 + 3 passed。Windows 录屏
job 110269661020 的 clippy、VP9、Opus/WebM、会话恢复和 WASAPI 合同均 success。
四项录屏原型中三项 success，macOS Intel 当时仍在运行；完整七项尚不计为通过。
完整 Windows job 日志在调用项目 ignored `src-tauri/target/windowsps-ci-b2fd247-win-*.log`。
本次只更新文档，后继文档 SHA 不冒称已执行门禁或远程 CI。

补充 Windows Edge 154.0.4258.48 无界面渲染夹具检查未完成：两次有界启动均 exit 0，
没有 stdout、stderr 或截图，未取得夹具断言结果；布局夹具未开始。这属于验证工具限制，
不计为测试通过，也不据此认定产品缺陷。自己的 Vite 进程已收回，没有观察到匹配专用
profile 的 Edge 残留进程。两次 RESULT.json 保存于调用项目 ignored target。

旧 45769c9 的 CI/QA 包单独记录。39 项 Windows 11 桌面 QA、Windows 10、多屏、
真实有声录屏、安装和 updater 保持未验证；没有安装 Clippy 或改变证书信任。
Linux 门禁未启动，按用户要求停止该路线。CHANGELOG 与后续草稿 PR 使用 `WIN-PS-GATE-01`。
