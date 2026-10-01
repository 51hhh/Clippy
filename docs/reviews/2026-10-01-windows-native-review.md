# 最新分支 Windows 原生审查

日期：2026-10-01；需求：`WIN-NATIVE-01`。
计划：[`2026-10-01-windows-native-review.md`](../superpowers/plans/2026-10-01-windows-native-review.md)。

## 基线与结论

本轮按用户指定审查最新分支。刷新 origin 后，最新分支是
`origin/codex/recording-audio-device-selection`，完整 SHA
`8b99b884f660f37c9d81ba0dc8947d13c3d3a08a`。原检出的 dev 是 `2383cc0`，两者不能混用验证结果。
当前修复分支是 `codex/windows-native-review`，应用版本仍为 `0.1.20`。

最新分支已经具备 Windows 原生粘贴/权限边界、二维长截图自动滚动、Pin 工作区历史、类型化动作
启动器，以及非默认 WGC/WASAPI 录屏 QA。Windows 待做工作的重点是本机回归、混合 DPI 与真实桌面
证据，不能因为 dev 没有这些实现再开发一套，也不能把 QA feature 算作正式发布功能。

## 关键 Git 节点

| 节点 | 实际变化 | 对本轮的意义 |
|---|---|---|
| `0b374b5` / v0.1.18 | 跨平台交付及 Windows 临时签名/信任说明所在发布节点 | 发布包、签名和桌面行为有独立验收边界 |
| `9f9a1f7` / v0.1.20 | 截图覆盖层资源协议、混合缩放原始像素与黑屏热修复 | 保持权威像素；Windows DPI 不能仅沿用 GNOME 几何假设 |
| `0fc171b` | 全应用复审与增强 OCR 集成 | 异步隔离、内存/线程与输出状态机基线 |
| `1409664`、`5ff6e45`、`fcc5096` | 平台单一事实源、窗口 IPC allowlist、serde/前端合同门禁 | 本轮修复不能绕过 typed capability 和 IPC 边界 |
| `46dada6`、`feae32c` | WASAPI 音频源及 Windows 双轨 QA 接线 | 只在 QA feature 图中验证；默认构建成功不会覆盖这些代码 |
| `0c00719`、`ea41d28` | Windows/macOS 原生四向自动滚动及平台编译修复 | Windows 使用物理鼠标点和完整性检查；应实测目标复核与用户接管 |
| `c798c34` | 系统声与麦克风固定增益混音 | 双源并发、时钟和拔出恢复增加真机测试维度 |
| `bfae832`、`8b99b88` | 设备枚举/一次性 token 选择及 Windows 字符串转换修复 | 非默认设备、同名设备、目录失效和指定设备丢失必须有独立证据 |

以上是本机 Git 实际可达节点；录屏、动作和长截图后续分支已包含在最新基线的祖先链中。

## 已验证的远程基线

通过已登录 gh 的只读 check-runs API 核对，以下七项对基线完整 SHA 均为 completed/success：

- Check (ubuntu-22.04)；
- Native Check (windows-latest)、Native Check (macos-latest)；
- Recording Codec Prototype (ubuntu-22.04)、(windows-latest)、(macos-15)、(macos-15-intel)。

Run：<https://github.com/51hhh/Clippy/actions/runs/35792281966>。
匿名验证脚本请求曾遇 GitHub 403 限流；随后读取 authenticated check-runs，未把失败请求计为通过。
该证据覆盖编译、lint 和对应测试，不覆盖 Windows 10/11 桌面、音频听感、DPI 或安装更新。
本轮修改尚未推送，修改后 SHA 的 CI 仍待执行。

## Findings

### W01 / P1 — Windows 文件 URL 路径导致合同测试失效（已复现并修复）

`src/tests/html-sinks.test.js` 与 `src/tests/ipc-contract.test.js` 使用
`new URL('../..', import.meta.url).pathname`。Windows 文件 URL 是 `/C:/...`，作为路径再次 resolve
后得到 `C:\C:\win\Clippy\...`，加载源码时 ENOENT。空格和 URL 编码的目录也有同类隐患。

最新基线在本机：73 个文件，1265 项通过、9 项失败；两组正/负合同用例均无法加载源码。
Linux 前端 CI 成功不能发现该宿主问题。修复改用 Node 的 `fileURLToPath`，保留全部合同断言。

定向红绿验证：两组 9 项从失败转为成功；新增 Windows 门禁退出码测试 3 项也通过。

### W02 / P1 — 本机 Windows 门禁缺口和 CI 前端宿主盲区（实现已补，远程待验证）

`ci-local.sh` 无条件要求 Xvfb，调用 Bash/GNOME/WebKit smoke，无法直接作为原生 PowerShell 入口。
`build.yml` 原先仅在 Ubuntu 执行前端，Windows runner 只执行 Rust，因此 W01 带着 CI success 留在分支。

本轮增加 `scripts/ci-windows.ps1`，默认检查 Python/Rust/前端，`-RecordingQa` 检查独立 QA 图；
`-FrontendOnly`、`-Quick` 明确标注部分范围。所有外部命令非零计为失败，不被后续成功覆盖。
Windows Native Check 增加 Node/IPC/HTML/lint/typecheck/Vitest/build/生产入口步骤。
同时增加 Windows/Ubuntu OCR 质量合同，分别覆盖 DACL/POSIX mode，避免宿主权限证据互相替代。

Linux 的完整 `ci-local.sh` 和像素 smoke 仍是独立证据；本轮未执行，不能以 Windows 部分门禁代替。

### W04 / P1 — 混合 DPI 跨屏候选坐标疑点（静态证据，未标记复现）

调用链：`vendor/xcap/src/windows/impl_monitor.rs` → `screenshot/backends.rs` 的
`normalize_monitor_geometry` → `capture/window_probe.rs` 的 `window_coordinate_ratio/to_logical`
→ `append_window_intersections` → `capture/overlay_windows.rs` 的 LogicalPosition。

Windows xcap 返回物理显示器原点和尺寸，当前归一化分别按每块显示器缩放折算原点；窗口矩形则按
窗口当前显示器缩放一次后与所有帧求交。混合 DPI 下没有一个可供所有屏幕共用的全局逻辑比例。
例如主屏 100%、右屏 150%，跨屏窗口按 150% 整体缩小，与主屏候选求交会使用不一致的尺度。
负坐标、每屏取整、Pin 留白与初始窗口所在屏又会影响后续定位。
本机枚举只发现一块显示器；WinForms 会话报告 bounds `0,0,2048,1152`（不作为物理像素或 DPI 证据），
因此无法在当前硬件上完成跨屏、负坐标和混合 DPI 的复现矩阵。

不能只修一个除法：长截图指针、Pin origin、录屏物理 crop 和 guide 都依赖这份合同。
本轮在缺混合 DPI 真机证据时不做跨域转换重构。W04 应先保存实际显示布局、
候选矩形和覆盖层位置，构建 100/125/150%、左右/上下/负坐标和跨屏窗口回归，然后单独修复。

### W05 — Windows 权限与私有文件已有实现，需实际复测

- `paste/native.rs` 保存 HWND + PID，检查目标是否仍存在且 PID 未变，调用共享
  `platform/windows.rs` 完整性检查后再恢复前台和注入。
- `private_files/windows.rs` 构造当前用户 DACL、阻止父级继承，配置覆盖使用
  `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)`；对应 Windows 测试已有代码。
- 普通权限→管理员目标、窗口销毁/复用、焦点被系统拒绝、旧文件 ACL 修复与连续配置保存仍需
  Windows 10/11 真机证明，不能把源码存在或 CI unit pass 写成桌面通过。

### W06 — Windows 录屏能力已接线，QA 未等于默认交付

- 默认 Cargo features 不含 `recording-windows-av-qa`；Native QA Windows 包显式启用该 feature。
- WGC 帧桥保留最新帧并设首帧期限，控制窗原生排除、显示器身份与 crop 校验已有实现。
- WASAPI 对象在 worker 内创建/销毁；显式 endpoint 用 `GetDevice` 连接，设备失败不会回退默认。
- 目录使用 caller 绑定、一次消费的 opaque token；用户标签与设备身份分别处理。
- 双源混音、统一时钟、暂停/恢复和可恢复 WebM 分段已有测试/生产链路。

Windows 10/11 非默认设备、录制中拔出、默认设备变化、控制窗排除实际像素、强杀恢复和 30 分钟漂移
仍未观测。按 `PX-REC-AUDIO-DEVICE-01`、`PX-REC-WINDOWS-AV-QA-01` 保留未完成项；不默认启用。

### W09 / P1 — Windows OCR 诊断目录未获得私有权限（已复现并修复）

本机 Python 3.12.7 的原 `mkdir(mode=0o700)` 不应用 Windows 私有 ACL，质量测试也错误地
按 POSIX 权限断言。Python 3.13 起对该 mode 有特殊处理，但授予当前用户与管理员，仍不等于本工具
要求的当前用户专用 DACL，见 [Python mkdir 文档](https://docs.python.org/3/library/os.html#os.mkdir)。
普通 Windows 用户创建真实符号链接另会遇到 `WinError 1314`，导致拒绝合同
未能进入待测代码。本轮保留拒绝断言：有权限时使用真实链接，只有该权限错误时模拟链接标记。

诊断目录使用 Win32 `CreateDirectoryW` 的 security attributes 在创建时应用
`D:P(A;OICI;FA;;;当前用户SID)`，查询实际 DACL 后才返回；不接受宽松继承或权限失败的回退。
创建后核对失败只尝试移除本次新建的空目录；已有目录和内容不改。NUL 路径在原生调用前拒绝，
避免宽字符串截断创建另一目录。原生测试核对中文路径、目录/子文件 ACL 和失败关闭。
POSIX 分支继续使用 `0700`，本轮 Windows 证据不能代替 Linux 回归。

实现依据：[CreateDirectoryW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createdirectoryw)、
[安全描述符字符串](https://learn.microsoft.com/en-us/windows/win32/secauthz/security-descriptor-string-format)。

### W10 / P2 — 第三方 C++ flag 在 MSVC 上被忽略（已观测，待独立维护）

`webm-sys 2.2.1` 构建时传入 `-fno-rtti`、`-std=gnu++11`、`-fno-exceptions`，MSVC 输出 D9002 并忽略。
本轮 QA check 和 Rust `-D warnings` clippy 仍成功；这不代表 C++ 构建无告警，也不是桌面录制通过。
该问题应在单独依赖维护分支复核 MSVC 对应参数、固定来源/许可证和跨平台 codec 回归，不在本工具
分支临时改 cargo registry 或压掉日志。

## 本机验证记录

### 后续新 checkout 审查

草稿 PR：<https://github.com/51hhh/Clippy/pull/13>，基于最新设备选择分支，仅提交本轮 Windows 验证修复。
首次修改后 CI：<https://github.com/51hhh/Clippy/actions/runs/36809445003>，SHA `85907d0e96dcf9dc598f5d9abb653cae42f45835`。
Windows 前端 1275 passed、5 failed，尚不能作为同 SHA 三平台通过证据。

W11：Windows runner 的 CRLF checkout 使两个 IPC 负例的 LF 字符串替换没有生效，另有三个结构
断言依赖 LF。用 `core.autocrlf=true` 的独立 checkout 重现同样五项失败。修复保留校验器本身，
IPC 夹具在每种宿主都执行 LF/CRLF 两组正负合同；删除注册/权限项还断言修改实际生效。
修复后 CRLF checkout：74 文件 / 1284 项通过；该 checkout 的完整前端门禁另暴露 W12，
结果为 10 项通过、1 失败、3 组显式跳过，不报告整体成功。

W12：vendored xcap 原始 SHA-256 绑定 LF 字节，默认 Windows checkout 的 CRLF 转换导致失配。
应限定 vendor 检出规则保存原始 LF，同时保留逐字节哈希校验；不通过归一化哈希输入放宽供应链。

Windows 桌面自动验收通道：computer-use 的 node_repl 在初始化时发生 Windows sandbox
`helper_unknown_error: setup refresh had errors`，重置后重试仍报 trusted Node process exited。
仅完成技能初始化/恢复检查，未执行 UI 输入或取得桌面验收证据。继续保留 W04–W07 的人工项。

环境：Windows 11 Pro for Workstations x64 / 10.0.22000；Node 22.22.0；Python 3.12.7。
用户已授权安装 Rust/MSVC/Windows SDK/MSYS2。Rust 1.98.1（MSVC host）、VS 2022 C++ Build Tools、
Windows SDK 10.0.26100.0、MSBuild 17.14.60.43110、MSYS2 make 4.4.1/NASM 3.02/diffutils/perl 与
Rust LLVM tools、LLVM 23.1.2/libclang 已安装；VS 附 CMake 3.31.6-msvc6 已补入会话 PATH。
WebView2 Runtime 154.0.4258.37 已存在。
修复未涉及应用 Rust 或前端界面行为。
本机结果针对基线 `8b99b88` 加本分支修改；修复拆分为独立提交，修改后同 SHA CI 待运行，
本机结果不能替代远程 CI 证据。

- 红基线：最新分支完整 Vitest 1265 passed、9 failed。
- 初次定向修复：12/12 passed；最终真实 PowerShell 入口 6/6 passed，覆盖退出码、
  部分检查、参数冲突、缺 cargo、缺 CMake 和缺 libclang 提前失败。
- Windows 前端门禁：11 项检查通过、0 失败、3 组显式跳过；74 个文件、1277 项 Vitest 通过。
  类型、JS lint、IPC/HTML、供应链、lockfile 安装、生产构建与真实入口均通过。
- Python 质量合同：31 项通过；视觉段落 3 项通过；智能擦除证据校验通过。
- 缺 cargo 时入口在 prerequisites 阶段失败，不进入检查，也未报告完整成功。
- 工作流 YAML 使用 UTF-8 解析成功；修改后远程执行仍待验证。
- Windows 默认完整范围：`ci-windows.ps1` 20 项通过、0 失败、2 组显式跳过（录屏 QA 图及 Linux smoke）。
  Rust check、严格 clippy、vendored WGC 严格 clippy 通过；Rust 1040 passed、0 failed、5 ignored。
  当次前端 74 个文件、1278 项通过，包含追加的缺依赖退出码合同。忽略项未计作通过。
- `cargo test` 输出一次 MSVC 中文 linker stdout 警告，返回成功；严格 clippy 无告警失败。
  jsdom 报告 Canvas 原生方法未实现，前端合同仍通过；真实像素 smoke 与桌面行为未由此证明。
- 首次 `ci-windows.ps1 -RecordingQa`：20 项通过、3 项失败、1 组显式跳过，退出 1。
  Opus 构建缺 PATH 中的 CMake；三项 QA 检查均未计作通过。VS 已附 CMake 3.31.6-msvc6，补充会话 PATH。
  另确认 VP9 bindgen 需要 libclang.dll，Rust LLVM tools 不包含该库；补齐 LLVM 与前置检查后重验。
- 补齐依赖后的 `ci-windows.ps1 -RecordingQa`：23 项通过、0 失败、1 组 Linux smoke 显式跳过，退出 0。
  QA check、严格 Rust clippy、全量 QA Rust tests 通过，VP9/Opus 原生库及绑定已生成。
  QA Rust：1093 passed、0 failed、5 ignored；前端最终：74 文件、1280 passed。
  默认与 QA Rust 图大量重叠，不相加为独立测试数；所有忽略/跳过项不计作通过。
- Linux 本地完整门禁、修改后远程 CI、安装包和人工 QA 尚未执行。

本分支是开发工具/测试修复；OCR 质量工具行为与 Windows 验证入口已写入 CHANGELOG，引用 `WIN-NATIVE-01`。
后续 W04–W07 若产生产品修复，使用对应独立分支、需求 ID 和 CHANGELOG，不能混入本工具分支。
