# 最新分支 Windows 原生审查

日期：2026-10-01；需求：`WIN-NATIVE-01`。
计划：[`2026-10-01-windows-native-review.md`](../superpowers/plans/2026-10-01-windows-native-review.md)。

## 当前续审结果

用户要求停止桌面操控，当前继续代码 review 与 Windows 本机自动验证。已安装的 QA 源码仍为
`45769c9`：真实记录是文本/图片 2 pass、Pin 工具栏裁切 1 fail、36 not_run；原始 39 项模板保留不变。
新 Pin、私有文件和长截图光标修复未安装，桌面复测、录屏/音频、管理员目标、多屏和 Windows 10 均保持未验证。

最新独立长截图光标修复源码 `d8dff808e320fd840376e2acec396887e6bbc3ce` 的 Windows 本机完整门禁
exit 0，23 passed / 0 failed / 1 skipped（Linux smoke）。默认 Rust 1046 / 5 ignored、QA Rust
1099 / 5 ignored、前端 75 文件 / 1292 passed；重叠 Rust 图不累加，验证前后检出干净、日志哈希已核对。
证据位于主检出的 `src-tauri/target/windows-longshot-cursor-native-qa-d8dff80/RESULT.json`。
失败路径使用同一生产 guard 的注入指针接口复现，不代表实际系统鼠标接管已经验收。
新 SHA CI、共享 X11/macOS 原生图和修复后桌面仍未验证。

独立私有写入修复源码 `f788b1f57d852b7df87e334b579c3224c7fd2543` 的 Windows 本机完整门禁
exit 0，23 passed / 0 failed / 1 skipped（Linux smoke）。默认 Rust 1042 / 5 ignored、QA Rust
1095 / 5 ignored、前端 75 文件 / 1292 passed；两组 Rust 重叠不累加，日志哈希及验证后干净检出已核对。
证据位于主检出的 `src-tauri/target/windows-private-write-native-qa-f788b1f/RESULT.json`。
新 SHA 原生 CI 未执行，后继证据文档不冒称已验证 SHA；其他阶段记录保留各自来源。

## 基线与结论

本轮按用户指定审查最新分支。刷新 origin 后，最新分支是
`origin/codex/recording-audio-device-selection`，完整 SHA
`8b99b884f660f37c9d81ba0dc8947d13c3d3a08a`。原检出的 dev 是 `2383cc0`，两者不能混用验证结果。
最初工具修复分支是 `codex/windows-native-review`，应用版本仍为 `0.1.20`；后续产品修复独立分支见上文和总计划。

最新分支已经具备 Windows 原生粘贴/权限边界、二维长截图自动滚动、Pin 工作区历史、类型化动作
启动器，以及非默认 WGC/WASAPI 录屏 QA。Windows 待做工作的重点是本机回归、混合 DPI 与真实桌面
证据，不能因为 dev 没有这些实现再开发一套，也不能把 QA feature 算作正式发布功能。

本轮工具修复在 SHA `42e52c064aba36bb7e93a5e68a0cfb54f65c5b7b` 的七项原生/录屏原型
CI 全部通过，官方同 SHA Windows QA MSI/NSIS 的来源、哈希与签名身份已核对。
独立 WebM 参数修复的 SHA `437949b` 曾在 macOS 原生的既有进程测试失败，
由独立 `OCR-PROC-CANCEL-01` / [PR #15](https://github.com/51hhh/Clippy/pull/15) 修复。
经用户授权调整依赖基线后，WebM [PR #14](https://github.com/51hhh/Clippy/pull/14) 的
`45769c958e9d4311a7d61ee53ed6707cc9a11ac4` 七项 CI 全部成功；相对 #15 仍仅六个 WebM 文件。
该 SHA 的官方 QA 包 [run 36823747034](https://github.com/51hhh/Clippy/actions/runs/36823747034)
已 completed/success，四平台 bundle 与 Ubuntu 24 X11 smoke 全部成功，六份产物 manifest
绑定完整 SHA；新 Windows MSI/NSIS 已下载并核对来源、哈希和签名身份。
该阶段用户暂不能手动验收，39 项 Windows 11 模板保持 not_run；后续实际记录见“当前续审结果”。
Windows 10、多屏、真实音频与完整安装升级仍未完成。以下失败 run 保留为历史证据。

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
| `ef78a1f` | Windows PowerShell 5.1 版本比较的原生命令引号兼容修复 | 本机 gate 与文档后继 b2fd247 的同 SHA CI 分开记录 |
| `7aa6cf6` | 小图 Pin 原生窗口高度与前端工具栏兜底同步 | 实际旧包缺陷和 CSS/原生窗口回归对应；修复后桌面未验 |
| `f788b1f` | 私有文件先准备权限，再截断与写入 | 权限失败时不先落内容，独立失败注入和实际 Windows DACL 合同验证 |
| `d8dff80` | 自动长截图 guard 清理时重新查询指针位置 | 提前失败时保留用户新位置或查询未知状态；同一生产 guard 的确定性回归 |

以上是本机 Git 实际可达节点；录屏、动作和长截图后续分支已包含在最新基线的祖先链中。

## 已验证的远程基线

通过已登录 gh 的只读 check-runs API 核对，以下七项对基线完整 SHA 均为 completed/success：

- Check (ubuntu-22.04)；
- Native Check (windows-latest)、Native Check (macos-latest)；
- Recording Codec Prototype (ubuntu-22.04)、(windows-latest)、(macos-15)、(macos-15-intel)。

Run：<https://github.com/51hhh/Clippy/actions/runs/35792281966>。
匿名验证脚本请求曾遇 GitHub 403 限流；随后读取 authenticated check-runs，未把失败请求计为通过。
该证据覆盖编译、lint 和对应测试，不覆盖 Windows 10/11 桌面、音频听感、DPI 或安装更新。
本轮修改已推送到草稿 PR；首次修改后 CI 和后续修复证据见下方记录，不能沿用基线结果。

## Findings

### W19 / P2 — 自动长截图提前失败时清理可能抢回用户光标（代码回归复现并修复）

`with_scroll` 的 `capture()?` 和原生窗口/指针检查可能提前返回，尚未进入显式 `user_interrupted`。
旧 `CursorRestore::drop` 只检查 armed，直接恢复开始位置；因此用户在失败发生前移动鼠标，
清理仍可发出恢复移动。保持旧生产行为、仅提取指针接口的 Windows 红基线为 1 passed / 3 failed，
exit 101；用户已移动、查询失败及容差外位置均错误恢复。测试不调用系统鼠标输入 API。

独立 `WIN-LONGSHOT-CURSOR-01` / `codex/windows-longshot-cursor-restore` 让同一 guard 销毁路径
复核当前位置，仅在既有 3 像素容差内恢复；查询失败记日志并保留位置，原业务错误继续返回。
正常完成、无人接管的失败及显式 disarm 路径保留；八项定向合同和 d8dff80 完整 Windows 门禁通过。
原需求 `PX-LS-NATIVE-AUTO-01` 的用户接管约束保留，Wayland Portal 路径未改。
这证明代码清理合同，不证明实际桌面接管、查询/移动间原生输入竞争、X11/macOS 编译或新 SHA CI。

### W18 / P2 — 新私有文件权限准备晚于内容写入（故障注入复现并修复）

`private_files.rs::write_private` 原先对新文件执行打开、截断、write 和 sync，再调用 `restrict_file`。
因此首次权限准备失败时，函数虽然返回错误，待写入内容却已经落盘。Windows MSVC 的共享生产
写入流程故障注入回归 exit 101（1 passed / 1 failed），实际文件非空，确认该失败时序。
配置、录屏 manifest/thumbnail 等调用通常已保护父目录；不声称已观察到跨账户内容泄漏。

独立 `WIN-PRIVATE-WRITE-01` / `codex/windows-private-write-order` 将打开后的权限准备放在
`set_len(0)`、write 之前，保留打开前的旧 ACL 修复、Unix 0600 与写后校正。新文件准备失败仅留空文件，
打开前/后准备失败都保留原文字节，较短内容成功覆盖没有旧尾部。
六项私有文件定向合同和 f788b1f 完整 Windows 默认/QA 门禁通过；规格、CHANGELOG 引用同一需求 ID。
创建瞬间空文件 ACL、路径竞争、真实 ACL 拒绝及跨账户桌面未验，新 SHA CI 未运行。

### W17 / P2 — 小图 Pin 工具栏被底部裁切（真机复现、代码修复通过）

45769c9 Windows 11 单屏 125% DPI：256×128 RGBA 图片的 Pin 视口为 308×280 逻辑像素，
保存、复制、关闭按钮不可访问。打开/关闭画布未消除裁切，完整 Pin 用例记为 fail。
后端下限仍为 280，前端无测量兜底仍为 277；最长工具栏含新增置顶和画布按钮，需要约 351 px。
定位函数钳制落点不能缩小 CSS 内容。

独立 `WIN-PIN-TOOLBAR-01` / `codex/windows-pin-toolbar-height` 将窗口下限调为 368、兜底调为 351，
保留内容尺寸及原始偏移。创建、缩放、工作区恢复的共用窗口尺寸路径已核对；真实 CSS 与 Rust 窗口
下限的回归先失败后通过，7aa6cf6 完整 Windows 本机门禁 exit 0。修复后桌面和新 SHA CI 尚未验证。

### W01 / P1 — Windows 文件 URL 路径导致合同测试失效（已复现并修复）

`src/tests/html-sinks.test.js` 与 `src/tests/ipc-contract.test.js` 使用
`new URL('../..', import.meta.url).pathname`。Windows 文件 URL 是 `/C:/...`，作为路径再次 resolve
后得到 `C:\C:\win\Clippy\...`，加载源码时 ENOENT。空格和 URL 编码的目录也有同类隐患。

最新基线在本机：73 个文件，1265 项通过、9 项失败；两组正/负合同用例均无法加载源码。
Linux 前端 CI 成功不能发现该宿主问题。修复改用 Node 的 `fileURLToPath`，保留全部合同断言。

定向红绿验证：两组 9 项从失败转为成功；新增 Windows 门禁退出码测试 3 项也通过。

### W02 / P1 — 本机 Windows 门禁缺口和 CI 前端宿主盲区（本机与 42e52c0 CI 通过）

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
创建后按保护位、DACL 存在/默认状态和 ACE 原始字节核对，SID、掩码、类型与继承标志均必须一致；
不依赖 SDDL 别名或 AI 状态的字符串形式。核对失败只尝试移除本次新建的空目录；已有目录和内容不改。NUL 路径在原生调用前拒绝，
避免宽字符串截断创建另一目录。原生测试核对中文路径、目录/子文件 ACL 和失败关闭。
POSIX 分支继续使用 `0700`，本轮 Windows 证据不能代替 Linux 回归。

实现依据：[CreateDirectoryW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createdirectoryw)、
[安全描述符字符串](https://learn.microsoft.com/en-us/windows/win32/secauthz/security-descriptor-string-format)。

### W10 / P2 — 第三方 C++ flag 在 MSVC 上被忽略（本机与四项 codec CI 通过，完整门禁待补）

`webm-sys 2.2.1` 构建时传入 `-fno-rtti`、`-std=gnu++11`、`-fno-exceptions`，MSVC 输出 D9002 并忽略。
本轮 QA check 和 Rust `-D warnings` clippy 仍成功；这不代表 C++ 构建无告警，也不是桌面录制通过。
独立分支 `codex/windows-webm-msvc-flags` 的 [PR #14](https://github.com/51hhh/Clippy/pull/14)
按真实编译器族修复，需求 `WIN-WEBM-MSVC-01`。实际 MSVC/clang-cl 六个 C++ 源文件重新编译
通过，既有有效模式保持；来源与构建脚本原始哈希、29 个上游 C/C++ 文件和许可证已核对。
完整本机录屏 QA 门禁 23 passed / 0 failed / 1 skipped；
[初次 e4ccc46 CI](https://github.com/51hhh/Clippy/actions/runs/36815516209) 的四项 codec 原型及
Ubuntu/macOS 原生均成功，Windows 原生仍因旧父分支 OCR 失败而 failure，不能计为七项通过。
同步父修复后，SHA `437949bd74a7d4b852581500834f8cb646cf1895` 的
[新 CI](https://github.com/51hhh/Clippy/actions/runs/36819267902) 最终为六项 success / 一项 failure：
四项 codec 及 Ubuntu/Windows 原生成功，macOS 原生 OCR 取消/回收合同失败，见 W15；
不能计为七项通过。真实桌面/安装保留未完成。

后续用户明确授权将 #14 基线调整到独立 #15（7982866）并同步依赖，六个 WebM 文件范围已由
Git 与 GitHub PR metadata 核对。新 SHA `45769c958e9d4311a7d61ee53ed6707cc9a11ac4` 的
[run 36821989611](https://github.com/51hhh/Clippy/actions/runs/36821989611) completed/success：
三项原生与四项 codec 原型全部成功，authenticated run/job/check-run 的身份和结论一致。
四平台 VP9/WebM、Opus、恢复合同均通过，Windows WASAPI 合同 11 项通过；新 Windows codec
日志不再出现相关 D9002。旧失败记录保留，不以基线调整改写历史。
本机完整录屏 QA 门禁仍绑定 e4ccc46，不能改写为本次 SHA 本机门禁。
同 SHA [Native QA run 36823747034](https://github.com/51hhh/Clippy/actions/runs/36823747034)
completed/success：Windows/Linux/macOS Intel/Apple Silicon bundle 与 Ubuntu 24 X11 smoke 均成功。
六份未过期产物 manifest 的完整 SHA、run ID、名称和 API digest 字段已核对；没有把未在本机
计算的 artifact ZIP 哈希写成已验证。官方 Windows artifact 11144906775 的三项登记文件哈希均匹配：
MSI `d94536813cfaa6922cc6356c536ec1bc796a40213ec9fbde5a7085b26307a0b4`（21159936 字节），
NSIS `ee4c13c1d17279b341bee6cdb2b751184c5ca13a69405b8b5f905afde64bca51`（16809880 字节）。
QA-BUILD 的完整 SHA、0.1.20、windows-x64、recording-windows-av-qa 与自签名 QA 用途一致。
CI 的 Authenticode Valid/预期 signer 检查成功；本机两包 UnknownError（不受信任根），
thumbprint `27486ABF27DA89E9563B0CCE371C5F1DF749E628` 与 CI 相同，未改本机信任或安装。
MSI 数据库 openMode=0 查询确认 ALLUSERS=1、64 位 Program Files、既有 UpgradeCode 和 WebView2
bootstrapper 条件，前后字节哈希相同；这不证明升级、卸载、缺失 runtime 或安装成功。
新 Windows 11 官方/本地模板字节相同，39 项仍 not_run；交接材料保存于 ignored
`src-tauri/target/windows-native-qa/45769c958e9d4311a7d61ee53ed6707cc9a11ac4/HANDOFF.md`。

### W15 / P1 — 原生子进程取消夹具的启动预算竞态（Unix 原生合同已通过）

437949b 的 macOS job 110231086906 在 `process_tests.rs:211` 失败，1054 passed / 1 failed /
5 ignored。失败断言为“假进程必须实际启动”；该模块与父分支相同，C++ 补丁没有修改它。
夹具先启动 Python，再受 250 ms 执行预算约束，却最多等 10 s 观察 PID；启动迟于预算时可能
先被回收，之后不再产生标记。CI 未输出实际解释器启动时长，不能断言本次具体延迟。

Windows 探针直接编译原始生产监督器，稳定复现 750 ms 慢启动在 250 ms 原预算下超时且无标记；
阶段控制下实际就绪，输出上限触发 kill/wait，PID 查询确认子进程已消失。这证明竞态前提和
真实监督器清理路径，不能替代 Unix 的 Tauri runtime/消费者合同。

独立 [PR #15](https://github.com/51hhh/Clippy/pull/15) / `OCR-PROC-CANCEL-01` 修复测试同步：
无延迟/750 ms 慢启动、原子 PID 标记、取消前仍运行、取消后许可仍占用、后继实际排队、
夹具释放后必须因输出上限清理并回收、回收后才进入后继工作。Linux 保留 /proc 存活/僵尸检查，
macOS 新增对已知夹具 PID 的 signal 0 检查；阶段等待和进程兜底仍有硬截止时间。
原有 250 ms 超时合同与生产 OCR 代码不改。格式通过，SHA
`79828665a2d77308b0973b19d993ae745590d521` 的
[run 36821712880](https://github.com/51hhh/Clippy/actions/runs/36821712880) 三项原生检查均成功；
Ubuntu/macOS 日志实际执行并通过取消/回收、250 ms 超时与输出上限用例。
macOS Rust 1055 passed / 5 ignored，Ubuntu 默认 Rust 1139 passed / 14 ignored，
Windows 默认 Rust 1040 passed / 5 ignored；Windows 编译图不含 Unix 专用进程测试。
独立 #15 的 run 36821712880 已 completed/success，三项原生与四项 codec 原型全部成功，
authenticated run/job/check-run 的身份、SHA 和结论一致。
继承该修复的 #14 / 45769c9 七项已全部通过，两个 SHA 的证据单独记录。

## 本机验证记录

### 后续新 checkout 审查

草稿 PR：<https://github.com/51hhh/Clippy/pull/13>，基于最新设备选择分支，仅提交本轮 Windows 验证修复。
首次修改后 CI：<https://github.com/51hhh/Clippy/actions/runs/36809445003>，SHA `85907d0e96dcf9dc598f5d9abb653cae42f45835`。
Windows 前端 1275 passed、5 failed，尚不能作为同 SHA 三平台通过证据。

第二次 CI：<https://github.com/51hhh/Clippy/actions/runs/36811830617>，SHA
`c9e504c6be9298366c6320f10288b5e52b343aee`。Windows 原五项 CRLF 合同已通过；新增现象为
1282 passed / 2 failed，两项均为默认 5000 ms 超时：首次 PowerShell 合同 9884 ms，首次
`Date.toLocaleString()` 的时间戳合同 11720 ms。该 CI 的 Windows Python/Rust 步骤被跳过，不能计作通过。

第三次 CI：[run 36814547166](https://github.com/51hhh/Clippy/actions/runs/36814547166)，
SHA `fe37aec2e6776814248ce935d917aa46181dd410`。Windows 前端、1284 项测试与原始 xcap 哈希
校验通过；随后 Python 31 项质量测试有 1 failure / 2 errors，Rust 被跳过，整体不能计为通过。
语料 failure 为 `source.html` 原始哈希被 CRLF 检出改变；两个 errors 为实际 DACL 与手写 SDDL
字符串不同。runner 的具体 SDDL 形式未输出，不能断言是特定 SID 别名或 AI 标记。

W09 后续修复：查询实际二进制 ACE 并与 CreateDirectoryW 所用的期望 descriptor 比较，严格
保留 P 保护位、DACL 存在/默认状态、全部 ACE 类型/继承标志/掩码/SID。Win32 原生探针复现
`S-1-5-18` 回写为 `SY`，说明字符串身份并不稳定；按 Microsoft 的
[SID 格式](https://learn.microsoft.com/en-us/windows/win32/secauthz/sid-strings)与
[DACL 查询](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-getsecuritydescriptordacl)
处理 NULL 与缺失 DACL。新增正例覆盖 SID 等价表示与 AI，10 个负例通过实际创建流程证明
未保护、错误用户、少权限、缺继承、继承 ACE、额外 ACE、deny、空/NULL/缺失 DACL 均失败关闭。
本机 33 项质量合同、3 项视觉段落通过；当时远程复验仍待完成，42e52c0 的结果见后文。

W14：浏览器 UI 与 MathML 语料 `source.html`、`capture.py` 的原始字节哈希登记在捕获记录。
Windows 默认 CRLF 检出改变来源，新增四个具体路径的 `eol=lf` 属性；PNG/字体/哈希、来源记录与
验证器均不修改，不按换行归一化输入。在 SHA `28186af7470de96eace2e627bbf134b06034a8ab` 的全新 `core.autocrlf=true` clone，
四个来源文本首次检出即为 LF，记录的原始 SHA-256 通过；普通 Rust 源码仍为 CRLF，Git 状态干净。
两套语料各在 HTML 追加一个 LF 均被 exit 2 拒绝，复原后通过。该 clone 的 33 项质量测试通过。

W13：PowerShell 子进程原本设置 20 秒硬超时，外层测试仅 5 秒，预算不一致。该组改为 30 秒，
保留子进程 20 秒硬超时、错误对象和全部退出码/失败计数断言；普通测试默认预算不变。
Windows 原生 locale 合同也采用 30 秒有界预算，Linux/macOS 保持 5 秒；无重试或虚构成功。
Node 24.21.0 在 workspace 内隔离使用，下载包与官方 SHA-256 核对一致；本机首次 locale
调用约 15.9 ms，未复现 runner 的 11.7 秒，不能把 CI 初始化现象写成 WebView2 产品延迟。
补丁下 Node 22 定向 212 项通过，Node 24.21.0 + CRLF 全前端范围 11 项通过、0 失败、
3 组显式跳过，74 文件 / 1284 项通过；当时远程补丁结果仍待复验，42e52c0 已通过。

W11：Windows runner 的 CRLF checkout 使两个 IPC 负例的 LF 字符串替换没有生效，另有三个结构
断言依赖 LF。用 `core.autocrlf=true` 的独立 checkout 重现同样五项失败。修复保留校验器本身，
IPC 夹具在每种宿主都执行 LF/CRLF 两组正负合同；删除注册/权限项还断言修改实际生效。
修复后 CRLF checkout：74 文件 / 1284 项通过；该 checkout 的完整前端门禁另暴露 W12，
结果为 10 项通过、1 失败、3 组显式跳过，不报告整体成功。

W12：vendored xcap 原始 SHA-256 绑定 LF 字节，默认 Windows checkout 的 CRLF 转换导致失配。
新增 `.gitattributes` 限定 vendor 文本和 Cargo 锁文件保留 LF；Windows CI 也执行该供应链校验。
独立 CRLF checkout 的主 Rust 源码仍为 CRLF，按新属性重新检出的 vendor 与锁文件为 LF，
完整前端范围最终为 11 项检查通过、0 失败、3 组显式跳过，74 文件 / 1284 项通过。
在固定 WGC 文件追加一个 LF 时，校验仍以 exit 1 拒绝；复原后成功。验证器和登记哈希未修改，
没有归一化哈希输入或跳过检查。当时修改后同 SHA CI 尚待执行，42e52c0 已通过。

全新 CRLF clone 在 `47f1ee7dcd5f234d3bc5756cebe6202de2f5fc47` 首次检出即通过原始哈希校验，
工作树无修改，无需手工归一化文件。主 Rust 源码为 CRLF，vendor 和锁文件为 LF。

同 SHA 本机诊断打包命令：`npm exec --prefix src -- tauri build --debug --ci --no-sign
--features recording-windows-av-qa --config src-tauri/tauri.ci.conf.json`，退出 0。
MSI/NSIS 均构建成功且 Authenticode 为 NotSigned。包与元数据保存在
`src-tauri/target/windows-local-diagnostic-47f1ee7dcd5f234d3bc5756cebe6202de2f5fc47/`。

| 诊断包 | 字节数 | SHA-256 |
|---|---:|---|
| `Clippy_0.1.20_x64-setup.exe` | 19709991 | `31d5dee6986948dbd42f01aed6c4c8792ce9673a31fb03394f3bc97146e0c667` |
| `Clippy_0.1.20_x64_en-US.msi` | 25591808 | `0a6b08372441d2e9f77277989e913b96631d15fe92d41dd02a9ad1c62c25d1ad` |

LOCAL-BUILD.json 明确记录 debug、非默认录屏 feature、未签名、本地诊断用途，以及桌面/安装 QA
为 `not_run`。构建数量不加入测试通过数，不替代同 SHA 官方 Native QA 包或正式 updater 产物。
用户确认仅有当前 Windows 11 单屏，混合 DPI/多屏与 Windows 10 场景缺少环境，继续保留未完成。

Windows 桌面自动验收通道：computer-use 的 node_repl 在初始化时发生 Windows sandbox
`helper_unknown_error: setup refresh had errors`，重置后重试仍报 trusted Node process exited。
仅完成技能初始化/恢复检查，未执行 UI 输入或取得桌面验收证据。继续保留 W04–W07 的人工项。

环境：Windows 11 Pro for Workstations x64 / 10.0.22000；Node 22.22.0；Python 3.12.7。
用户已授权安装 Rust/MSVC/Windows SDK/MSYS2。Rust 1.98.1（MSVC host）、VS 2022 C++ Build Tools、
Windows SDK 10.0.26100.0、MSBuild 17.14.60.43110、MSYS2 make 4.4.1/NASM 3.02/diffutils/perl 与
Rust LLVM tools、LLVM 23.1.2/libclang 已安装；VS 附 CMake 3.31.6-msvc6 已补入会话 PATH。
WebView2 Runtime 154.0.4258.37 已存在。
修复未涉及应用 Rust 或前端界面行为。
以下早期本机结果针对基线 `8b99b88` 加本分支修改；当时修改后同 SHA CI 待运行，
本机结果不能替代远程 CI 证据。

- 红基线：最新分支完整 Vitest 1265 passed、9 failed。
- 初次定向修复：12/12 passed；最终真实 PowerShell 入口 6/6 passed，覆盖退出码、
  部分检查、参数冲突、缺 cargo、缺 CMake 和缺 libclang 提前失败。
- Windows 前端门禁：11 项检查通过、0 失败、3 组显式跳过；74 个文件、1277 项 Vitest 通过。
  类型、JS lint、IPC/HTML、供应链、lockfile 安装、生产构建与真实入口均通过。
- Python 质量合同：31 项通过；视觉段落 3 项通过；智能擦除证据校验通过。
- 缺 cargo 时入口在 prerequisites 阶段失败，不进入检查，也未报告完整成功。
- 工作流 YAML 使用 UTF-8 解析成功；当时修改后远程执行仍待验证，42e52c0 已通过。
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
- 当时 Linux 本地完整门禁、修改后远程 CI、官方 QA 包、安装升级和人工 QA 尚未执行；后续证据见下文。

本分支是开发工具/测试修复；OCR 质量工具行为与 Windows 验证入口已写入 CHANGELOG，引用 `WIN-NATIVE-01`。
后续 W04–W07 若产生产品修复，使用对应独立分支、需求 ID 和 CHANGELOG，不能混入本工具分支。

### W09/W14 后续本机完整门禁

源码 SHA `28186af7470de96eace2e627bbf134b06034a8ab`，Node 24.21.0：默认 Windows 门禁
20 passed / 0 failed / 2 skipped（录屏 QA、Linux smoke）。Rust 1040 passed / 5 ignored；
前端 74 文件 / 1284 passed；Python 33 项质量和 3 项视觉段落通过。W10 的独立分支录屏 QA
结果绑定其 e4ccc46，不将两次门禁或重叠 Rust 图相加，也不将编译当作桌面验收。

桌面运行时再次经受支持的 `node_repl + @oai/sky` 初始化，仍返回
`trusted Node process exited unexpectedly; kernel reset, rerun your request`；没有实际 UI 操作。
不得绕过 Computer Use skill 的专用 API 协议，Windows 11 单屏人工 QA、Windows 10 与多屏均未完成。

## 修改后同 SHA CI 与官方 Windows QA 包

### 42e52c0 原生与录屏原型 CI

[Run 36816386762](https://github.com/51hhh/Clippy/actions/runs/36816386762) 绑定完整 SHA
`42e52c064aba36bb7e93a5e68a0cfb54f65c5b7b`，整体 completed/success：

| 检查 | 状态 |
|---|---|
| Check (ubuntu-22.04) | completed/success |
| Native Check (windows-latest) | completed/success |
| Native Check (macos-latest) | completed/success |
| Recording Codec Prototype (ubuntu-22.04) | completed/success |
| Recording Codec Prototype (windows-latest) | completed/success |
| Recording Codec Prototype (macos-15) | completed/success |
| Recording Codec Prototype (macos-15-intel) | completed/success |

三项规定的原生检查经 authenticated check-runs 和仓库 `evaluateNativeChecks` 核对；
Windows job 110222252345 的日志为前端 74 文件 / 1284 passed，Python 33 项质量 + 3 项视觉段落，
Rust 1040 passed / 0 failed / 5 ignored。忽略项不计作通过，四项 codec job 不累加进单元测试数。
证据位于 ignored target 的 `native-ci-42e52c0-{check-runs.ndjson,evidence.md}` 与
`windows-ci-42e52c0.log`。这些结果不能套用于后续证据文档提交、W10 新 SHA 或真实桌面场景。

### Windows 官方 QA 安装包身份

同 SHA 的 [Native QA Packages run 36817675012](https://github.com/51hhh/Clippy/actions/runs/36817675012)
仅在三项原生 CI success 后启动；Windows job 110226195519 completed/success。
产物 `qa-windows-x64-42e52c064aba36bb7e93a5e68a0cfb54f65c5b7b`（artifact 11142234087）
已通过已登录 gh 下载，QA-BUILD 的 commit、version=0.1.20、platform=windows-x64、
recording_feature=recording-windows-av-qa、signing=self-signed-qa-only 均核对一致。

| 官方 QA 文件 | 字节数 | SHA-256 |
|---|---:|---|
| `Clippy_0.1.20_x64_en-US.msi` | 21155840 | `d25afe50e0e569cffd64548a5f3f979fca4ca8e2d2fb09babc97189d43b5e912` |
| `Clippy_0.1.20_x64-setup.exe` | 16814408 | `29d7fbb982a9f74cdb10f94e68026665093904ecb254910a0dc0c5a90ce53a21` |
| `QA-BUILD.txt` | 159 | `bda69a22f4e9732ff12863cb9bb153c1aff7f08da4e6c40fbf3ea4a6524b409e` |

三个文件均匹配官方 SHA256SUMS。CI 上传前检查两包 Authenticode 为 Valid 且签名者 thumbprint
为其临时证书 `1C03A60538874852C87573766DCBFF21EC1420B1`，该步骤成功。只读本机查询两包均为
UnknownError，消息为证书链终止于不受信任根；签名主体 CN=Clippy Self-Signed Release，thumbprint
与 CI 预期相同，存在 DigiCert 时间戳。未导入证书或修改本机信任；不能写成本机签名链 Valid。
QA 自签名和包校验不证明 SmartScreen、安装、发行信任或 updater 工作。

官方 MSI 另经 Windows Installer `OpenDatabase(path, 0)` 只读检查，读取前后 SHA-256 相同。
[Microsoft 文档](https://learn.microsoft.com/en-us/windows/win32/msi/installer-opendatabase)明确该模式不持久化修改。
ProductName=Clippy、ProductVersion=0.1.20、INSTALLDIR 位于 ProgramFiles64Folder；ALLUSERS=1，
对应 [per-machine 安装范围](https://learn.microsoft.com/en-us/windows/win32/msi/allusers)。
升级表使用 UpgradeCode `{FD731BCF-0B99-5F17-9EEE-900E4DF2CB97}`，VersionMin=0、无 VersionMax、
Attributes=257、ActionProperty=WIX_UPGRADE_DETECTED；RemoveExistingProducts 序号 1501。
包内有启动菜单、桌面及 msiexec 卸载快捷方式；WebView2 缺失条件触发 DownloadAndInvokeBootstrapper，
与仓库 downloadBootstrapper 配置一致。这里只证明表项存在，未证明升级/卸载、下载失败处理或跨账户行为。
本机已具备 WebView2，缺运行时/离线安装场景仍需独立环境。完整表项记录在
`MSI-READONLY-INSPECTION.json`，只读检查不加入测试通过数。

材料位于 `src-tauri/target/windows-native-qa/42e52c064aba36bb7e93a5e68a0cfb54f65c5b7b/`：
`PACKAGE-VERIFICATION.json`、`LOCAL-SIGNATURE-OBSERVATION.json`、`CI-SIGNATURE-EVIDENCE.json`、
`MSI-READONLY-INSPECTION.json`、`HANDOFF.md`、安装包与官方记录模板。
官方 Windows 11 模板与同 SHA 本地生成模板字节相同，39 项均 not_run；testedAt 未填写。
离线 Unicode/富文本/透明 RGBA PNG 夹具的字节哈希与像素已核对，仅证明测试材料可用。

用户表示暂不能手动执行 Windows 11 单屏验收。W04–W07、Windows 10、真实音频、长时漂移、
安装升级/updater 继续未完成；Linux 本地完整门禁和修改后的 Wayland 回归也未执行。
全平台 QA workflow 最终 completed/success，四个平台 bundle 与 Ubuntu 24 X11 smoke 均成功；
六份产物 manifest 的完整 SHA、run、未过期状态和 digest 均核对一致。Windows 包已下载并核对
实际文件哈希；其它平台包未在本机执行，X11 smoke 不替代 Wayland 回归。
没有合入 dev、发布或开启默认录屏。
