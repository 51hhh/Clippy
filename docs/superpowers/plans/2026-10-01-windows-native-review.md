# WIN-NATIVE-01 — 最新分支 Windows 审查与整改

日期：2026-10-01；状态：in_progress。

## Goal

以最新功能分支为基线，在 Windows 复核现有核心功能及非默认录屏 QA 能力，修复可复现问题，
补齐本机验证入口，并建立可追踪的原生编译、安装包和桌面验收任务。

## Baseline

- 已刷新 origin；最新分支为 `origin/codex/recording-audio-device-selection`。
- 基线 SHA：`8b99b884f660f37c9d81ba0dc8947d13c3d3a08a`，应用版本 `0.1.20`。
- 工作分支：`codex/windows-native-review`；只承载 Windows 验证入口和合同测试修复。
- 用户确认 Ubuntu Wayland 已调试正常；本轮未重跑该环境，不扩展为 Linux 全矩阵验收。
- 基线 GitHub CI：<https://github.com/51hhh/Clippy/actions/runs/35792281966>，
  三项默认原生检查及 Ubuntu、Windows、macOS ARM/Intel 四项录屏原型检查均为 success。
  这是基线证据，不能替代后续修改 SHA 的检查或 Windows 桌面 QA。

## Requirements

1. 区分 dev、发布 tag 和最新功能分支，记录关键提交带来的实际能力及 feature 门控。
2. 在 Windows 本机运行完整前端测试、类型检查、静态合同和生产构建；文件 URL 使用系统路径转换，
   源码合同兼容 LF/CRLF；负例修改必须实际生效。vendor 和 Cargo 锁文件按仓库规则保留 LF，
   原始 SHA-256 校验不得归一化输入。保留现有安全合同的全部负例，不能删除失败测试或弱化校验。
3. 提供原生 PowerShell 门禁，覆盖 Python 纯合同、默认 Rust、vendor WGC、前端和可选 Windows
   双轨 QA 检查。缺工具、外部命令非零、显式部分检查和跳过项不得被报告为完整通过。
4. Windows 原生 CI 增加前端检查，补上只有 Ubuntu 执行前端导致的宿主路径盲区。
5. 审查混合 DPI/负坐标、普通与管理员目标粘贴、DACL/原子配置、WGC/WASAPI、设备目录、
   录屏控制窗排除、暂停/恢复和崩溃分段恢复；每个结论区分复现缺陷、静态疑点和未执行验收。
6. 保持录屏 `recording-windows-av-qa` 非默认；正式 release 的能力不因 review 提前开放。
7. 产品行为修复使用独立分支、自己的回归测试与 CHANGELOG；本分支仅验证工具变更，无用户界面行为变更。
8. OCR 质量工具的诊断目录在 Windows 使用当前用户专用、禁止宽松继承的 DACL；创建失败不得
   落回普通目录。符号链接拒绝合同不要求普通 Windows 用户具备创建真实符号链接的权限。

## Acceptance Criteria

- [x] 最新分支、完整 SHA、关键节点和基线同 SHA CI 已核对。
- [x] Windows 前端红基线已取得：73 个文件，1265 项通过、9 项失败，失败均为两组合同测试路径错误。
- [x] 两组合同测试与完整前端测试在 Windows 通过，类型、JS lint、IPC/HTML、供应链和生产入口通过。
- [x] PowerShell 门禁的缺依赖、非零退出码和部分检查不能虚报完整成功；入口文档与脚本一致。
- [x] Windows OCR 诊断目录及新建子文件的 DACL 原生检查通过；质量测试不依赖 POSIX mode 或符号链接特权。
- [ ] Windows 前端 CI 在 fe37aec 已通过；OCR 阶段新失败已修复待复验，完整原生/原型 CI 仍需同 SHA 成功。
- [x] 默认及 `recording-windows-av-qa` 在 Windows 本机完成 Rust check/clippy/test。
- [ ] Windows 10/11 混合 DPI、多屏、权限、安装更新及有声录屏桌面 QA 绑定相同包/SHA。
- [ ] 已确认的产品缺陷修复后复测；尚未执行、外部工具缺失和静态疑点保留未完成状态。

## Out of Scope

- 自动合入 dev、改写历史、发布版本或启用录屏默认 feature。
- 以 Ubuntu Wayland、GitHub runner 或合成源测试替代 Windows 桌面证据。
- 重做 UI，或在 Windows review 中扩展 OCR 权重交付、智能擦除、任意脚本等其它产品需求。
- 未经确认地把静态 DPI 疑点写成已复现缺陷或做跨域坐标重构。

## Tasks

| ID | 优先级 | 工作与验收 | 状态 |
|---|---|---|---|
| W01 | P1 | 修复文件 URL 路径；保留 9 项合同负例/正例并完成 Windows 前端门禁 | 本机已通过 |
| W02 | P1 | PowerShell 门禁与 Windows CI 前端检查；缺工具/失败/部分运行严格区分 | 本机入口已验证，修改后 CI 待执行 |
| W03 | P1 | Rust MSVC、C++ SDK、WebView2；录屏另需 MSYS2 make/diffutils/perl/nasm、MSBuild、CMake、LLVM tools/libclang；默认与 QA 图分别验证 | 工具已安装，默认与录屏 QA 本机门禁通过 |
| W04 | P1 | 100%/125%/150% 多屏与负坐标：冻结帧、跨屏窗口候选、覆盖层、Pin、guide、长截图自动滚动、WGC 选区 | 静态疑点；本机单屏，待多屏真机复现 |
| W05 | P1 | 同权限自动粘贴一次、高完整性目标 copy-only、目标销毁/复用、用户接管；DACL 与配置连续覆盖 | 待 Windows 复测 |
| W06 | P1 | QA 包设备默认/非默认/同名/拔出、双源混音、暂停恢复、控制窗排除、强杀恢复、30 分钟 A/V 漂移 | 代码存在，待真机验收 |
| W07 | P2 | NSIS/MSI 安装升级卸载、WebView2、自启动、托盘/快捷键、系统凭据与更新 | 本机未签名 debug 包构建成功；正式 QA 包、安装与桌面验收待执行 |
| W08 | P1 | 每个产品修复单独分支，更新对应需求/CHANGELOG；同 SHA 三平台 + 四原型 CI，回归 Ubuntu Wayland | 后续改动后执行 |
| W09 | P1 | OCR 质量工具 Windows 私有诊断目录与符号链接拒绝合同；失败关闭，检查子文件继承 | 实际 DACL/等价 SDDL 及 10 类失败关闭负例通过；本机 33 项质量合同通过，跨平台 CI 待复验 |
| W10 | P2 | 审查 webm-sys 的 C++ 编译参数在 MSVC 上产生 D9002；按宿主选择 flag，保留固定来源与许可证 | 独立 WIN-WEBM-MSVC-01 / PR #14 已提交；本机完整 QA 门禁通过，CI 待执行 |
| W11 | P1 | 新 Windows runner 使用 CRLF 检出时的 IPC 负例与结构回归；保留两种换行的正/负合同 | 独立 CRLF checkout 修复后 1284 项通过，修改后 CI 待执行 |
| W12 | P1 | vendored xcap 保持固定 LF 字节并运行原始 SHA-256 校验；不能归一化哈希输入或跳过检查 | 独立 CRLF checkout 前端门禁 11 项通过、0 失败；字节篡改仍被拒绝，修改后 CI 待执行 |
| W13 | P1 | Windows 原生进程/locale 测试预算覆盖实测初始化；保留子进程硬超时、全部断言与普通单元测试默认预算 | CI 暴露两项 5 秒超时；限定测试组补齐预算后，Node 24.21.0 + CRLF 全前端门禁通过，远程待复验 |

W04–W07 使用 `docs/native-qa.md` 和 `scripts/manual-qa.mjs` 的 Windows profile。
安装包证据与本地源码构建分开，模板初始 `not_run` 不能计作通过。

## Verification

本机：Windows 11 Pro for Workstations x64，build 22000；Node 22.22.0，Python 3.12.7。
用户已授权安装工具链。Rust 1.98.1 MSVC、VS 2022 C++ Build Tools/Windows SDK、MSYS2 已安装；
WebView2 Runtime 154.0.4258.37 已核对，Rust LLVM tools 与 LLVM 23.1.2 libclang 已安装，
VS 附 CMake 3.31.6-msvc6 已补入当前会话 PATH。原生默认与录屏 QA 检查通过。

`ci-windows.ps1 -FrontendOnly`：11 项检查通过，0 失败，3 组显式跳过；这是完整前端范围，
属于整体部分门禁。Vitest 74 个文件、1277 项通过；包含 3 项真实 PowerShell 退出码合同。
Python 质量合同 31 项通过，视觉段落 3 项通过，智能擦除证据校验通过。
默认完整 Windows 范围：20 项检查通过、0 失败、2 组显式跳过；Rust 1040 项通过、5 项忽略，
check、严格 clippy、vendor WGC 严格 clippy 通过。前端追加缺依赖回归后为 74 文件、1278 项通过。
录屏 `-RecordingQa` 最终完整 Windows 范围：23 项检查通过、0 失败、1 组 Linux smoke 显式跳过。
QA Rust 1093 项通过、5 项忽略；前端最终为 74 文件、1280 项通过，含 6 项门禁退出码/前置依赖合同。
默认与 QA 测试大量重叠，不累加为独立覆盖数。Python 质量 31 项、视觉段落 3 项通过。
独立 `core.autocrlf=true` checkout 修复 W11/W12 后：完整前端范围 11 项检查通过、0 失败、
3 组显式跳过，74 文件 / 1284 项通过。主 Rust 源码保持 CRLF，xcap 和 Cargo 锁文件按属性检出 LF；
刻意在固定文件追加一个 LF 被原始 SHA-256 校验拒绝，复原后校验成功。
全新 CRLF clone 在 `47f1ee7dcd5f234d3bc5756cebe6202de2f5fc47` 首次检出即保持 vendor/锁文件 LF，
原始哈希校验成功。本机同 SHA 使用 `recording-windows-av-qa`、debug、`--no-sign` 构建 MSI/NSIS，
两包成功且为 NotSigned；绑定 SHA 的 LOCAL-BUILD.json 和 SHA256SUMS.txt 保存在 ignored target 目录。
这些仅属诊断构建证据，不计入测试通过数，也不替代官方 Native QA 包、签名或安装验收。
用户确认目前仅有当前 Windows 11 单屏，暂无多屏或 Windows 10 验收环境。
CI SHA `c9e504c` 原五项 CRLF 合同已通过，但 PowerShell/首次 locale 两项触发默认 5 秒超时。
W13 补丁仅给 Windows 原生测试组有界预算，PowerShell 子进程仍在 20 秒硬超时后失败；无重试/删断言。
与 CI 同版本的 Node 24.21.0 在 workspace 内隔离下载并验证官方 SHA-256，CRLF 前端门禁为
11 项通过、0 失败、3 组显式跳过；74 文件 / 1284 项通过。系统 Node 版本未替换。
Linux 本地完整门禁、修改后 CI、官方 QA 安装包、Windows 10 和桌面交互尚未执行。

详细审查证据见 `docs/reviews/2026-10-01-windows-native-review.md`。所有新结果按层级追加，
没有实际执行的项不勾选。

W09 后续：fe37aec 的 Windows 前端 CI 通过，OCR 实际 DACL 与手写 SDDL 比较失配，Rust 未执行。
改为二进制 ACE/保护位严格核对，本机 33 项质量 + 3 项视觉段落通过；等价 SID/AI 正例和
10 类真实创建失败关闭负例均保留。修改后同 SHA CI 与完整 Windows 门禁仍待复验。
