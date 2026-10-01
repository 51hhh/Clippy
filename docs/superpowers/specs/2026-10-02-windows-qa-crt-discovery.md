# WIN-QA-CRT-DISCOVERY-01 — Windows QA 的已发布 v14 CRT 家族发现

## Goal

基于 W48 / `263a2e7` 审查 SDK 目录发现，避免 Windows QA 把兼容的已发布 v14 release CRT
家族限定为 VC143。对应 WIN-NATIVE-01 / W49；独立分支 `codex/windows-qa-crt-discovery`。

## Requirements

1. `windows-latest` 官方镜像已使用 VS 2026，Microsoft 声明已发布 v140/v141/v142/v143/v145
   二进制兼容及 v145 redistributable。原脚本只找 desktop x64 `Microsoft.VC143.CRT`；用实际
   PowerShell 入口和受控 SDK 布局/元数据验证发现缺口，不声称已观察真实 VS 2026 CI 失败。
2. 只枚举版本化 redist 根目录下直接 desktop x64 的已发布 CRT 家族：VC140/141/142/143/145。
   不递归搜索 onecore、debug、x86/arm64 或未知家族。按数字版本排序选择最新兼容目录，
   同一最新版本出现多个候选时拒绝，不能靠枚举顺序或安静回退掩盖歧义/不可信新版。
3. 原文件签名、完整版本至少与 toolset 同样新、AMD64、核心 DLL/允许列表、哈希和递归
   闭包全部保留；目录名不是信任证明。清单登记实际 redist 家族/目录，准备失败不发布配置。
4. 保留原五十项合同和原产品/Rust/vendor/基础配置/正式 release；新增独立目录夹具通过
   PowerShell 5/7 的真实脚本。元数据受控，不替代真实 Microsoft 签名或 VS 2026 安装/编译。
5. 干净新 SHA 完整 Windows 默认/QA 门禁、现有真实 VC143 SDK 准备与隔离 unsigned/unbundled
   QA release 编译/实际 payload 核对分层记录；旧 W48 证据与同 ID CHANGELOG 保留。

## Acceptance Criteria

- [x] 旧入口的 VC145/跨家族最新版本发现缺口已复现，原五十项保持通过。
- [x] 支持已发布家族、数字版本选择、歧义拒绝与不可信最新目录失败关闭，受控合同通过。
- [ ] 真实现有 VC143 SDK 的签名/版本/字节/闭包和实际 QA payload 通过，默认产物保持。
- [ ] 干净源码完整 Windows 门禁、实际子进程退出和源码/日志/文档核对通过。
- [ ] 当前 SHA 远程 CI 与真实 VS 2026 SDK/安装器/无 CRT 系统启动通过。

## Out of Scope

不安装 VS 2026、系统运行库或新工具；不启动/安装应用、不恢复桌面、不改证书/信任；
不推送/PR/合入/发布，不执行 Linux/WSL。当前 Windows 11 单屏、Windows 10/多屏、
录屏/音频/升级与其它宿主/Wayland 继续保留未验。未知 CRT 家族不会按通配符自动放行。

## Primary references

- [GitHub runner-images](https://github.com/actions/runner-images)：`windows-latest` 关联 VS 2026 镜像。
- [Microsoft v14 二进制兼容](https://learn.microsoft.com/en-us/cpp/porting/binary-compat-2015-2017?view=msvc-170)：
  已发布工具集家族与运行库不得早于最新组件 toolset 的限制。
- [Microsoft VS 2026 redistribution 清单](https://learn.microsoft.com/en-us/visualstudio/releases/2026/redistribution)：
  v145、版本化 redist，以及 debug/onecore debug 排除范围；不据此宣称旧宿主实际兼容。

## Verification

原入口在新增 24 项中 12 passed / 12 failed；原五十项全部通过，共 62/12。十二项失败
分别涉及 VC145、新旧并存、跨家族最新/数字排序、歧义与不可信最新目录被跳过；PowerShell
5/7 各六项。修改生产发现流程后，同七十四项全部通过；再增加其余三个已发布目录标签
六项，共 80 passed（新增三十项，原五十项正文未改）。目录标签/版本/publisher 元数据
受控，不能写成真实 VS 2026 SDK 或旧 SDK 编译通过。真实现有 SDK、完整门禁和隔离
unbundled 编译待新 SHA 核对；W48 与保存实际 QA 仍保持各自历史身份。
