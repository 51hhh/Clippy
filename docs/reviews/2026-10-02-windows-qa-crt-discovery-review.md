# WIN-NATIVE-01 / W49 — Windows QA 的已发布 CRT 家族发现

需求 `WIN-QA-CRT-DISCOVERY-01`；基线 `263a2e7`，独立分支 `codex/windows-qa-crt-discovery`。
被测源码 `ae2fdb6d20f54d8577eceeca82bcd502db00676c`；规格见
[`2026-10-02-windows-qa-crt-discovery.md`](../superpowers/specs/2026-10-02-windows-qa-crt-discovery.md)。
本轮仅代码审查、已有 Windows 原生 CLI 和文件检查；桌面保持停止。

## 问题与修复

原 QA 准备脚本只接受 `Microsoft.VC143.CRT`。新版已发布 v14 家族即使文件兼容，
也会发现失败；新旧家族并存时还会忽略新版、同版本歧义或不可信新版并回退旧版。
这些缺口通过真实 PowerShell 入口和受控 SDK 布局复现，未观察真实 VS 2026 CI 失败。

仅枚举版本化 redist 根下直接 desktop x64 的 VC140/141/142/143/145。按数字版本
选择最新兼容候选，同一最新版本多个家族时拒绝；新版文件不可信时失败关闭，不回退。
原核心 DLL、允许列表、完整版本、Microsoft 有效签名、AMD64、哈希及递归 PE 闭包
检查全部保留。manifest 增加实际 `redistFamily`，共享部署合同仍为 `WIN-QA-CRT-01`。

GitHub 官方镜像列表已将 `windows-latest` 指向 VS 2026 镜像；Microsoft 文档列出
已发布 v140/v141/v142/v143/v145 的二进制兼容，要求运行库至少与组件使用的最新
toolset 同样新，并给出 VS 2026 v145 的 redistribution 清单。
([runner-images](https://github.com/actions/runner-images)、
[Microsoft 二进制兼容限制](https://learn.microsoft.com/en-us/cpp/porting/binary-compat-2015-2017?view=msvc-170)、
[VS 2026 redistribution](https://learn.microsoft.com/en-us/visualstudio/releases/2026/redistribution))

目录标签不是签名或兼容性证明；未知家族、onecore、debug、x86/arm64 不自动放行。
旧家族夹具使用受控新版本元数据，不声称当前产品能用真实旧 SDK 编译。

## 分层验证

- 原新增 24 项：12 passed / 12 failed，PowerShell 5/7 各六个发现缺口；原五十项
  全部通过，共 62/12。修改生产脚本后同七十四项通过；原二十四项正文、命令与断言
  保持。随后增加六个其它已发布目录标签用例，合计新增三十/原五十项共 80 passed。
  夹具有可解析 PE 元数据，publisher/版本受控，没有真实 DLL 代码，也不调用 loader。
- 干净源码完整 `./scripts/ci-windows.ps1 -RecordingQa`，child/terminal exit 0：
  **33 passed / 0 failed / 1 Linux smoke skipped**。Rust 默认 1193、QA 1256，各 5 ignored，
  两图不累加；前端 **81 文件/1403 passed**，新增三十项在总数内。
- 原八十份前端测试 Git blob 未改；Rust/前端产品、vendor、锁文件、基础配置、全部
  workflow、门禁脚本和独立运行库验证器未改。Git 对比数量不是测试通过数。
  保存旧 PowerShell 输入含 Windows CRLF；仅规范化 CRLF 后与原 Git LF 正文严格相同，
  保存字节和 Git blob 各自哈希独立登记。生产脚本 UTF-8 BOM 保留。
- 真实现有 VS 2022/MSVC 14.44.35207 仍选择 `Microsoft.VC143.CRT`；十份 DLL 的
  Microsoft Authenticode 状态均为 Valid，实际版本均为 **14.44.35211.0**。
- 显式 `x86_64-pc-windows-msvc`、unsigned/unbundled、locked/offline QA release 编译
  **native/wrapper/terminal exit 0**，约 3m45s；未运行 EXE、DLL 或安装器。
  真实主程序 rustc 命令包含 QA 五项 feature 及 opt-level=s / panic=abort / fat LTO /
  单 codegen unit / strip=symbols；编译参数不证明运行时清理或设备行为。
- 实际 EXE 31,687,680 bytes，SHA-256
  `405e7856d4cbc28e729c563a5470726963175fa86467379014be01b179b903f3`。
  同目录十份 CRT、清单摘要、原/暂存/部署字节与 direct/delay 递归闭包通过；原十七份
  基础许可证哈希一致。默认 release EXE 的 SHA-256 保持
  `bde08375a1950f4925e2771f544476947e958a832b2800934e007e85b3837410`，默认目录无 QA CRT。

构建、资源、DLL 和 Git 对比数量均不增加测试通过数。完整门禁及构建前后源码干净，
构建恢复原分支；后续只同步五份 Markdown。根 checkout 未改，历史 W48/W47 证据
保留原 SHA；共享 target 的实际 QA 文件已是本轮源码，旧副本与 UUID stage 各自保留。

## 本机证据

根目录 `C:\win\Clippy\src-tauri\target\` 下：

- `windows-qa-crt-discovery-contract/`：旧输入、最初红/绿及扩展绿报告、保存用例、
  `DISCOVERY-CONTRACT-AUDIT.json` 与之前 W48 状态/报告。
- `windows-qa-crt-discovery-native-qa-ae2fdb6/RESULT.json`：完整门禁、实际 exit 0 与日志摘要。
- `windows-qa-crt-discovery-release-build-ae2fdb6/`：实际 EXE、构建日志、运行库来源/配置、
  `PAYLOAD-VERIFICATION.json`、`MAIN-RUSTC-COMMAND.json`、`SOURCE-AND-PAYLOAD-AUDIT.json`。

## 未验证边界

真实 VS 2026 SDK、当前 SHA 远程 CI、NSIS/MSI 构建、无 CRT 系统启动均未验证。
Linux/其它宿主完整门禁、同 SHA 三平台/四编码器 CI、Windows 10、多屏/混合 DPI、
真实录屏/音频/升级与 Wayland 回归继续保留。没有推送、合入、发布或修改系统工具/信任。

保存旧 `45769c9` Windows 11 桌面 QA 仍是 2 pass / 1 fail / 36 not_run；记录未刷新。
该记录及本轮文件/自动化证据不能替代本轮产品的桌面验收，整体 `WIN-NATIVE-01` 未完成。
