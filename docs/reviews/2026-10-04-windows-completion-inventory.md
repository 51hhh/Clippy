# WIN-NATIVE-01 — 当前Windows审查完成状态清单

2026-10-04，W88；当前测试源码`228cc935bc0767ed9906f03ff5680cc89e72febe`，前置文档`e87cd2be62aace8fcae6aed1f2c0bfd0b6018c41`；分支`codex/windows-current-completion-audit`。
原[计划](../superpowers/plans/2026-10-01-windows-native-review.md)的8R/9AC/47任务保持工作范围。
旧W79清单只有49项修复；本轮补入后续三项，并另外核对W80测试基础设施。

## 核对结果与边界

52代码/部署修复、51历史，282子需求/247子AC，24子AC仍未勾选。
单独1项测试基础设施的规范/原日志/当前阶段名称保留已核对，不计第53项产品修复；
其末条跨平台CI/设备/交付AC仍未完成，test-profile unwind不证明release panic=abort恢复。
所有52源码为当前祖先，独立分支/规范RID/CHANGELOG/原门禁日志身份已核对。
历史Rust通过用例仍在当前同名阶段通过；1479次Rust测试正文Token对照中1474一致，
5次准确匹配既有W46夹具适配证据。九份前端源/138次测试调用AST对照无删除或改写，
包含回调和数据集；共享夹具/辅助函数/所有运行行为不由正文Token或AST等价推定。
这些是保留比较，不是新测试执行数；不同历史源和两Rust图有重叠，不能相加计通过。

同源码完整Windows门禁保持33passed/0failed/1Linux smoke skipped；默认1341/QA1493
各5ignored，前端81/1409，QA录屏523已含1493。没有重跑门禁或增加修复/测试数。
同源码默认/显式QA release及四个NSIS/MSI程序/CRT/canonical文件来源已核对；
四包实际NotSigned。编译、PE、签名读取、包文件核对不证明API/设备/加载/安装行为。
原8R/9AC全部索引；末两条全局AC保留未完成。W07/W08/W47只同步状态栏，
全部47任务的ID/优先级/工作及验收内容不改，其余44条状态不改。

## 当前外部和桌面证据

实际只读GitHub查询228cc93返回422 No commit found，同仓库45769c9读取成功。
查询UTC2026-10-03 17:41:48，原响应/正对照已保存；证明当前提交尚不可用，
不将空PR-only workflow列表或旧42/457/b2fd/888状态作为当前同SHA七项CI成功。
没有推送、PR、workflow或发布动作；原记录与新的当前SHA证据分开保存。
只读安装文件当前31,371,064字节，SHA256为
`349c30737e007e433cdbe039ce4d973751c8ded2b78b86a33942e0855236f8d7`，
与旧457 QA安装文件记录一致；没有应用/进程/设备/桌面操作，不当作当前52修复复测。
原旧包39桌面项2passed/1failed/36not_run、当前停止指令和旧失败根因未明保持。

## 52项规范索引

下表R/AC数仅为规范项数，末列为规范未勾选数。已勾选亦不自动证明当前端到端完成；
每条原需求/AC全文、Source/Git/规范原字节哈希及阶段/正文对照见CUMULATIVE-SOURCE-AUDIT。

| # | 稳定RID/规范 | 源码 | R/AC | 未勾选AC |
| ---: | --- | --- | ---: | ---: |
| 1 | [WIN-PIN-TOOLBAR-01](../superpowers/specs/2026-10-01-windows-pin-toolbar-height.md) | `7aa6cf6` | 5/4 | 1 |
| 2 | [WIN-PRIVATE-WRITE-01](../superpowers/specs/2026-10-01-windows-private-write-order.md) | `f788b1f` | 5/4 | 0 |
| 3 | [WIN-LONGSHOT-CURSOR-01](../superpowers/specs/2026-10-01-windows-longshot-cursor-restore.md) | `d8dff80` | 5/4 | 0 |
| 4 | [WIN-CF-HTML-01](../superpowers/specs/2026-10-01-windows-cf-html-bounds.md) | `50b7778` | 6/4 | 0 |
| 5 | [WIN-CLIP-IMAGE-BUDGET-01](../superpowers/specs/2026-10-01-windows-image-decode-budget.md) | `531d791` | 5/5 | 0 |
| 6 | [WIN-DIBV5-PIXEL-01](../superpowers/specs/2026-10-01-windows-dibv5-pixel-offset.md) | `25fb5d7` | 5/4 | 0 |
| 7 | [WIN-PASTE-RECHECK-01](../superpowers/specs/2026-10-01-windows-paste-input-recheck.md) | `14bf616` | 5/4 | 0 |
| 8 | [WIN-WASAPI-STOP-TAIL-01](../superpowers/specs/2026-10-01-windows-wasapi-stop-tail.md) | `a463c3b` | 5/4 | 0 |
| 9 | [WIN-WGC-CLOSE-01](../superpowers/specs/2026-10-01-windows-wgc-close-retry.md) | `03b4cb8` | 5/4 | 0 |
| 10 | [REC-AV-BRIDGE-JOIN-01](../superpowers/specs/2026-10-01-windows-av-bridge-join.md) | `091b5cb` | 5/4 | 0 |
| 11 | [WIN-WGC-INIT-ROLLBACK-01](../superpowers/specs/2026-10-01-windows-wgc-init-rollback.md) | `61d6823` | 5/4 | 0 |
| 12 | [WIN-REGISTRY-BUFFER-01](../superpowers/specs/2026-10-01-windows-registry-buffer.md) | `bb38cc6` | 5/4 | 0 |
| 13 | [WIN-WINDOW-SCALE-01](../superpowers/specs/2026-10-01-windows-window-candidate-scaling.md) | `78bd83f` | 5/4 | 0 |
| 14 | [WIN-OVERLAY-FOCUS-01](../superpowers/specs/2026-10-01-windows-overlay-focus.md) | `f577996` | 5/4 | 0 |
| 15 | [WIN-NATIVE-MONITOR-01](../superpowers/specs/2026-10-01-windows-physical-monitor-bounds.md) | `2f0e225` | 6/5 | 0 |
| 16 | [WIN-PIN-ORIGIN-01](../superpowers/specs/2026-10-01-windows-pin-physical-origin.md) | `888127a` | 6/5 | 0 |
| 17 | [WIN-PIN-WORKAREA-01](../superpowers/specs/2026-10-02-windows-pin-workarea.md) | `4a58101` | 6/5 | 0 |
| 18 | [WIN-PIN-LIVE-DPI-01](../superpowers/specs/2026-10-02-windows-pin-live-dpi.md) | `e339042` | 6/5 | 0 |
| 19 | [WIN-WGC-BRIDGE-ROLLBACK-01](../superpowers/specs/2026-10-02-windows-wgc-bridge-rollback.md) | `7922457` | 6/5 | 0 |
| 20 | [WIN-MAIN-TARGET-01](../superpowers/specs/2026-10-02-windows-main-window-target.md) | `c9d5512` | 5/5 | 0 |
| 21 | [WIN-CONTROL-ROLLBACK-01](../superpowers/specs/2026-10-02-windows-control-rollback.md) | `a52ecaa` | 7/5 | 0 |
| 22 | [REC-DELETE-OWNER-01](../superpowers/specs/2026-10-02-recording-delete-owner.md) | `0321355` | 6/5 | 0 |
| 23 | [WIN-EXPORT-IDENTITY-01](../superpowers/specs/2026-10-02-windows-export-identity.md) | `69cf0b4` | 6/5 | 0 |
| 24 | [REC-MEDIA-REVOKE-01](../superpowers/specs/2026-10-02-recording-media-revoke.md) | `6944b68` | 6/5 | 0 |
| 25 | [REC-PLAYBACK-LIFECYCLE-01](../superpowers/specs/2026-10-02-recording-playback-lifecycle.md) | `ba26a83` | 6/5 | 0 |
| 26 | [REC-LIBRARY-READY-01](../superpowers/specs/2026-10-02-recording-library-ready.md) | `1907809` | 6/5 | 0 |
| 27 | [WIN-SHORTCUT-SHARED-01](../superpowers/specs/2026-10-02-windows-shortcut-shared.md) | `0f793c9` | 6/5 | 0 |
| 28 | [WIN-CLIP-SNAPSHOT-01](../superpowers/specs/2026-10-02-windows-clipboard-snapshot.md) | `b541e87` | 6/5 | 0 |
| 29 | [WIN-PASTE-CLEANUP-01](../superpowers/specs/2026-10-02-windows-paste-key-cleanup.md) | `f5ad5da` | 6/5 | 0 |
| 30 | [WIN-QA-CRT-01](../superpowers/specs/2026-10-02-windows-qa-crt-deployment.md) | `5d900ca` | 7/6 | 1 |
| 31 | [WIN-QA-CRT-DISCOVERY-01](../superpowers/specs/2026-10-02-windows-qa-crt-discovery.md) | `ae2fdb6` | 5/5 | 1 |
| 32 | [REC-AV-STARTUP-GATE-01](../superpowers/specs/2026-10-02-recording-av-startup-gate.md) | `1c66112` | 5/5 | 1 |
| 33 | [REC-FIRST-FRAME-AUDIO-01](../superpowers/specs/2026-10-02-recording-first-frame-audio.md) | `2dccc43` | 5/5 | 1 |
| 34 | [REC-AV-GAP-DRAIN-01](../superpowers/specs/2026-10-02-recording-av-gap-drain.md) | `80e084c` | 5/4 | 1 |
| 35 | [REC-AV-CFR-SEGMENT-01](../superpowers/specs/2026-10-02-recording-av-cfr-segments.md) | `66ceffd` | 6/5 | 1 |
| 36 | [REC-WINDOWS-IDLE-AV-01](../superpowers/specs/2026-10-02-recording-av-idle-frontier.md) | `6518661` | 5/5 | 1 |
| 37 | [REC-PENDING-FRAME-PCM-01](../superpowers/specs/2026-10-02-recording-pending-frame-budget.md) | `120b3ad` | 4/5 | 1 |
| 38 | [REC-MANIFEST-SHARING-01](../superpowers/specs/2026-10-02-recording-manifest-sharing.md) | `d7c66bb` | 4/5 | 1 |
| 39 | [REC-ARTIFACT-SHARING-01](../superpowers/specs/2026-10-02-recording-artifact-sharing.md) | `830b12b` | 4/5 | 1 |
| 40 | [REC-VIDEO-CONTROL-PREFLIGHT-01](../superpowers/specs/2026-10-02-recording-video-control-preflight.md) | `4eb65d8` | 5/5 | 1 |
| 41 | [REC-VIDEO-CONTROL-FAILURE-01](../superpowers/specs/2026-10-02-recording-video-control-failure.md) | `7579222` | 5/5 | 1 |
| 42 | [REC-AV-CONTROL-TIMELINE-01](../superpowers/specs/2026-10-02-recording-av-control-timeline.md) | `003fe2d` | 5/5 | 1 |
| 43 | [REC-AUDIO-ACTIVATION-BOUNDARY-01](../superpowers/specs/2026-10-02-recording-audio-activation-boundary.md) | `56d750c` | 5/5 | 1 |
| 44 | [REC-WASAPI-QPC-PRECISION-01](../superpowers/specs/2026-10-02-wasapi-qpc-precision.md) | `aac5e0a` | 5/6 | 1 |
| 45 | [REC-MIXED-FRAME-BOUNDARY-01](../superpowers/specs/2026-10-02-mixed-audio-frame-boundary.md) | `4bc1977` | 5/5 | 1 |
| 46 | [REC-MIXED-CONTROL-SKEW-01](../superpowers/specs/2026-10-02-mixed-control-skew.md) | `b0b51f1` | 5/5 | 1 |
| 47 | [REC-MIXED-PAUSED-STOP-01](../superpowers/specs/2026-10-02-mixed-paused-stop.md) | `02005c9` | 5/5 | 1 |
| 48 | [REC-MIXED-STOP-READY-BOUND-01](../superpowers/specs/2026-10-02-mixed-stop-ready-bound.md) | `2bb12f9` | 6/5 | 1 |
| 49 | [REC-AUDIO-CATALOG-ORDER-01](../superpowers/specs/2026-10-02-audio-catalog-refresh-order.md) | `d05cd3e` | 6/5 | 1 |
| 50 | [WIN-LONGSHOT-INPUT-CANCEL-01](../superpowers/specs/2026-10-03-longshot-input-cancel.md) | `00f40cc` | 7/5 | 1 |
| 51 | [WIN-WASAPI-ACTIVATION-TAIL-01](../superpowers/specs/2026-10-03-wasapi-activation-tail.md) | `8889192` | 7/5 | 1 |
| 52 | [WIN-QA-MSI-PROVENANCE-01](../superpowers/specs/2026-10-04-windows-qa-msi-provenance.md) | `228cc93` | 5/4 | 1 |

## 剩余完整任务

- 当前SHA三平台原生及四项录屏CI；Linux完整本地门禁/Ubuntu Wayland回归、macOS图。
- 当前源Windows11权限/几何/设备/WGC/WASAPI/声音/暂停恢复/30分钟同步及修复后Pin复测。
- Win10、多屏混合DPI/负坐标/无开发CRT启动和原安装升级卸载/updater/WebView2运行。
- W53/W59/W63/W67历史失败根因与已丢失原媒体证据；后续绿测及保留器不能解释旧失败。

下一步可准备当前228cc93同SHA七项CI的具体提案，先核对现有workflow触发/发布条件及
精确源分支，不执行push/dispatch。规范既有不推送范围下需显式授权才能做外部写入；
桌面/设备/安装继续停止。本清单是证据与剩余项审计，全局WIN-NATIVE-01未完成。
辅助旧字段名KeyError失败和过大PS5摘要输出均保存诊断；仅改证据读取，原门禁/源码不改。
证据位于主仓库src-tauri/target/windows-current-review-inventory-228cc93。
