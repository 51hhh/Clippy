# WIN-NATIVE-01 / W46 — Windows 代码审查累计证据

本机被测源码：`f5ad5da5bfb9cacb92c4cf0f0932f89a31345415`。最近产品修复是 W45；其完整门禁后的文档为 `132726a`。
本轮只整理累计证据，未修改产品源码/测试，未重复运行已经通过的门禁，也未恢复桌面操作。
原 Windows 任务保持进行中，本文不把代码阶段、旧 CI 或旧安装包当作完整 Windows 验收。

## 当前可以证明的范围

29 个被测修复提交都在当前历史中；各历史门禁的源码 SHA、成功状态、原始 stdout/stderr
哈希逐项核对。每个历史 Rust 用例在当前相同阶段仍通过，未仅用默认/QA 并集替代分组证明。
当前完整 Windows 门禁为 30 passed / 0 failed / 1 Linux smoke skipped，native child 和
终端 exit 0；默认 Rust 1193、QA Rust 1256（各 5 ignored，两图重叠不累加）。独立剪贴板
31、WGC/registry 十八项另列；前端 79 文件 / 1323 passed，Python 33 + 3。

616 次 Rust 测试正文比较有 611 次词法正文一致。另五次仅新增冻结物理元数据夹具字段，
按精确新增 token 单独核对，原逻辑输入/全部断言保持；manager 用例还断言新增物理输出。
它们不是原字节相同。比较含重复文件及未进入 Windows 图的源码，**616 不计测试通过数**。
五份产品前端测试文件的 31 个 `it/test` 调用（含回调/参数数据）经 TypeScript AST 比较保持，
**31 不计测试通过数**。参数化调用可包含多行用例；实际计数以 1323 的运行结果为准。
W42 的 79/1323 发现清单哈希保持，之后前端跟踪源码没有变化，与当前最小日志总数一致。

已读 Windows CI 的 Node/前端/OCR/native 步骤；当前源码 SHA 的 GitHub 查询返回零个运行，
因此当前 CI 缺失。Cargo 默认 feature 不含录屏，QA feature 保持显式。当前 Windows OCR
五项 DACL 合同实际 `ok`，源代码核对区分真实私有目录查询与受控失败/SDDL 负例。

## 原需求与剩余任务

| 原项 | 当前证据与边界 |
|---|---|
| R1 / 关键节点与基线 | 基线/关键提交及当前 29 项历史核对；历史 CI 不替代当前 CI |
| R2 / Windows 前端与安全合同 | 当前完整前端、类型、静态边界、供应链、构建/入口通过；测试正文保留另审计 |
| R3 / 原生 PowerShell 门禁 | 当前默认/QA 图、vendor、Python 与退出状态验证；Linux skip 保留 |
| R4 / Windows CI 前端 | 配置已接线；当前源码没有 GitHub run，未证明其远程运行成功 |
| R5 / DPI、权限、捕获/音频/恢复 | 源码与受控合同已审；真实设备、输入竞争、控制窗/强杀、漂移与坐标矩阵未验 |
| R6 / 非默认录屏 QA | feature 门控保持；当前 release QA 可执行文件仍未构建 |
| R7 / 独立修复与需求记录 | 29 个独立 branch/source/spec/CHANGELOG 引用存在；原生复测/跨平台 CI 尚未闭环 |
| R8 / OCR 私有目录 | 当前用户保护 DACL 与失败关闭合同通过；不延伸为跨账户验证 |
| W04–W07 / 原生与安装 QA | 当前仅保存旧包 39 项的 2 passed / 1 failed / 36 not_run；修复后实际复测未执行 |
| W08 / 跨平台与 Wayland | 当前同 SHA 三 native + 四 recording CI、Linux 完整本地门禁及 Wayland 回归仍缺失 |

原计划的八条 Requirements、九条 Acceptance Criteria、W01–W45，以及 29 个子需求的
验收项/边界均留在机器可读 inventory。最后两条全局 AC 保持未完成；子需求本地 AC 完成
不等于平台/安装/真机交付完成。Windows 10 与多屏环境仍不可用；当前用户要求桌面停止。

## 29 个当前历史中的被测源码

| 需求与规范 | 原被测源码 |
|---|---|
| [`WIN-PIN-TOOLBAR-01`](../superpowers/specs/2026-10-01-windows-pin-toolbar-height.md) | `7aa6cf6` |
| [`WIN-PRIVATE-WRITE-01`](../superpowers/specs/2026-10-01-windows-private-write-order.md) | `f788b1f` |
| [`WIN-LONGSHOT-CURSOR-01`](../superpowers/specs/2026-10-01-windows-longshot-cursor-restore.md) | `d8dff80` |
| [`WIN-CF-HTML-01`](../superpowers/specs/2026-10-01-windows-cf-html-bounds.md) | `50b7778` |
| [`WIN-CLIP-IMAGE-BUDGET-01`](../superpowers/specs/2026-10-01-windows-image-decode-budget.md) | `531d791` |
| [`WIN-DIBV5-PIXEL-01`](../superpowers/specs/2026-10-01-windows-dibv5-pixel-offset.md) | `25fb5d7` |
| [`WIN-PASTE-RECHECK-01`](../superpowers/specs/2026-10-01-windows-paste-input-recheck.md) | `14bf616` |
| [`WIN-WASAPI-STOP-TAIL-01`](../superpowers/specs/2026-10-01-windows-wasapi-stop-tail.md) | `a463c3b` |
| [`WIN-WGC-CLOSE-01`](../superpowers/specs/2026-10-01-windows-wgc-close-retry.md) | `03b4cb8` |
| [`REC-AV-BRIDGE-JOIN-01`](../superpowers/specs/2026-10-01-windows-av-bridge-join.md) | `091b5cb` |
| [`WIN-WGC-INIT-ROLLBACK-01`](../superpowers/specs/2026-10-01-windows-wgc-init-rollback.md) | `61d6823` |
| [`WIN-REGISTRY-BUFFER-01`](../superpowers/specs/2026-10-01-windows-registry-buffer.md) | `bb38cc6` |
| [`WIN-WINDOW-SCALE-01`](../superpowers/specs/2026-10-01-windows-window-candidate-scaling.md) | `78bd83f` |
| [`WIN-OVERLAY-FOCUS-01`](../superpowers/specs/2026-10-01-windows-overlay-focus.md) | `f577996` |
| [`WIN-NATIVE-MONITOR-01`](../superpowers/specs/2026-10-01-windows-physical-monitor-bounds.md) | `2f0e225` |
| [`WIN-PIN-ORIGIN-01`](../superpowers/specs/2026-10-01-windows-pin-physical-origin.md) | `888127a` |
| [`WIN-PIN-WORKAREA-01`](../superpowers/specs/2026-10-02-windows-pin-workarea.md) | `4a58101` |
| [`WIN-PIN-LIVE-DPI-01`](../superpowers/specs/2026-10-02-windows-pin-live-dpi.md) | `e339042` |
| [`WIN-WGC-BRIDGE-ROLLBACK-01`](../superpowers/specs/2026-10-02-windows-wgc-bridge-rollback.md) | `7922457` |
| [`WIN-MAIN-TARGET-01`](../superpowers/specs/2026-10-02-windows-main-window-target.md) | `c9d5512` |
| [`WIN-CONTROL-ROLLBACK-01`](../superpowers/specs/2026-10-02-windows-control-rollback.md) | `a52ecaa` |
| [`REC-DELETE-OWNER-01`](../superpowers/specs/2026-10-02-recording-delete-owner.md) | `0321355` |
| [`WIN-EXPORT-IDENTITY-01`](../superpowers/specs/2026-10-02-windows-export-identity.md) | `69cf0b4` |
| [`REC-MEDIA-REVOKE-01`](../superpowers/specs/2026-10-02-recording-media-revoke.md) | `6944b68` |
| [`REC-PLAYBACK-LIFECYCLE-01`](../superpowers/specs/2026-10-02-recording-playback-lifecycle.md) | `ba26a83` |
| [`REC-LIBRARY-READY-01`](../superpowers/specs/2026-10-02-recording-library-ready.md) | `1907809` |
| [`WIN-SHORTCUT-SHARED-01`](../superpowers/specs/2026-10-02-windows-shortcut-shared.md) | `0f793c9` |
| [`WIN-CLIP-SNAPSHOT-01`](../superpowers/specs/2026-10-02-windows-clipboard-snapshot.md) | `b541e87` |
| [`WIN-PASTE-CLEANUP-01`](../superpowers/specs/2026-10-02-windows-paste-key-cleanup.md) | `f5ad5da` |

## 证据与下一步

本机证据目录：`C:\win\Clippy\src-tauri\target\windows-review-completion-audit-f5ad5da`。
`RESULT.json` 是逐项 inventory；`CUMULATIVE-SOURCE-AUDIT.json`、
`RUST-FIXTURE-ADAPTER-AUDIT.json`、`FRONTEND-AST-AUDIT.json` 记录原始哈希、阶段和正文；
`CURRENT-SHA-CI.json` 为本次当前 SHA 只读结果。安装包/保存实际 QA 与模板哈希保持。

下一项安全代码验证是使用已有工具链，从冻结被测源码构建 Windows release QA 可执行
文件。已读当前 Tauri CLI help、Windows/CI 配置和 release profile：使用显式
`recording-windows-av-qa`、`--ci --no-bundle --no-sign` 与 CI 配置关闭 updater artifact，
Rust 参数 `--locked --offline`。正常 release 的 fat LTO/单 codegen unit 未由 test profile
证明，不能直接将 check/test 代替这项构建。产物只读核对哈希/PE/来源，既非官方包也非
安装器，不启动、不安装、不生成签名证书或修改信任、不下载安装器工具/新系统依赖。
真实运行、NSIS/MSI、当前 CI 和其它平台/Wayland 保留未验；不推送/合入/发布。
