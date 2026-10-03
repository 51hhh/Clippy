# WIN-NATIVE-01 / W79 — 当前49项修复与 Windows 验证证据审查

被测源码 `d05cd3e478934722273a33fb88c841648aeb1ef5`，前置文档 `6448584685cf71633ce365fe63a925ddb7503f7d`。本轮只修正证据关联并审查，源码/测试未改，新产品修复/新测试0；桌面操作继续暂停。

## 已修正的记录问题

W78之前的`localWindowsGate`汇总已含d05cd3e的总数，却仍将`sourceSha/evidence/verificationAudit`指向旧02005c9门禁，并保留491的旧领域数字。这是一组混合来源的汇总，不能用作任一SHA的完整证明。现按原W77 RESULT、GATE-AUDIT、原日志哈希和1064个冻结输入替换为d05cd3e的规范记录；整个旧汇总保留在不可变PREVIOUS-W78文件和supersededLocalWindowsGateSummary，旧结果文件没有改写。

Windows完整门禁实际运行于W77：33 passed / 0 failed / 1 Linux smoke skipped；默认Rust1324、QA1475，各5 ignored，重叠不累加；前端81文件/1405。完整QA日志中509项recording用例包含在1475内，没有本轮定向领域重跑。独立剪贴板31和vendor18保留各阶段归属。

## 原需求与测试保留性

原8条Requirements、9条AC、47条Tasks全部保留并逐项索引；49项子规范的263条Requirements、233条AC（21条未勾选）也按原文索引。规范中的历史勾选不是当前端到端验收证明。机器RESULT和CUMULATIVE记录每项的证据范围与缺口。

49项源码均是当前源码祖先；各自分支、RID、规范、CHANGELOG、原完整成功门禁及原stdout/stderr哈希已核对。每次旧门禁中真实通过的Rust全名仍在当前相同阶段通过。原修复提交所改Rust文件的测试正文比较1428次：1423次词法相同，5次仅匹配W46已审计的物理夹具字段适配，当前正文仍精确一致。8份前端历史来源的111个测试调用AST均保留，含原callback/dataset/timeout参数；无未解释删除。

比较包含重复来源和非当前宿主的测试正文，不算1428次测试；111个调用也不算新增通过数。正文/调用比较不证明共享辅助函数完全未变或系统行为正确。先前详细审计保留原SHA含义。

R1保留关键Git节点与基线；R2/R3有当前Windows前端、类型、静态合同、默认/QA/vendor/Python完整门禁；R4只有当前CI配置和W78远程读取证据；R5有本地回归及有限静态审查，原生矩阵仍未完成；R6由Cargo配置与W78编译器feature指纹确认录屏非默认；R7完成49项本地追踪；R8保留当前质量合同和Windows本机DACL合同。源代码/合成文件/CI/Native人工QA各层不能替代。

## 本轮有限代码审查

配置保存、私有文件工具、Windows ACL/替换、settings/tmux保存、窗口位置worker和截图提示共7个当前冻结文件已核对。查到的运行时配置落盘路径均持有config锁；settings的shortcut transition及最终合并保留窗口/提示/tmux字段；私有写入在截断/内容前准备权限，Windows替换和当前用户保护DACL传播错误。现有门禁包含准备失败、真实本机DACL修复/替换和配置回滚用例。本轮未确认新的产品缺陷，不能扩展为跨账户/路径竞争/断电/外部OS行为已经安全。

W78的同源码默认/QA release均已编译、PE/feature/profile及QA10个CRT来源/部署文件核对，产物哈希本轮再次读取匹配。它们未启动/安装，不算运行测试，release panic=abort也不借用测试unwind回收保证。

## 未完成与下一步

原最后两条AC和所有真实验收缺口不勾选。W78 GitHub读取于2026-10-02 14:45:30 UTC：d05cd3e返回422“No commit found”，已知45769c9成功作访问正例。本轮未再次查询网络；当前同SHA成功CI仍无证明，没有推送/触发。

旧安装包45769c9的39项桌面记录仍为2 passed / 1 failed / 36 not_run，旧Pin失败和未复测保持，包不含后续49项修复。当前Win11单屏真机、Windows10/多屏混合DPI/负坐标、权限、设备、有声录屏/30分钟同步、完整安装升级卸载/updater、缺开发CRT环境、Linux完整门禁/Wayland及macOS/同SHA七项CI继续未完成。W53/W59/W63/W67历史失败根因未明，已丢失的原TempDir媒体不能重建为当时证据。

下一项可做的代码审查是原AV/清单sharing测试的TempDir/worker所有权和超时退出：判断能否在未来失败时保留绑定源码的诊断文件，同时保持原测试正文、期限、背压和非零退出。此处只是静态候选，不计已确认产品缺陷；补诊断也不能解释历史失败。

## 49项本地修复

| 需求 | 原被测源码 |
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
| [`WIN-QA-CRT-01`](../superpowers/specs/2026-10-02-windows-qa-crt-deployment.md) | `5d900ca` |
| [`WIN-QA-CRT-DISCOVERY-01`](../superpowers/specs/2026-10-02-windows-qa-crt-discovery.md) | `ae2fdb6` |
| [`REC-AV-STARTUP-GATE-01`](../superpowers/specs/2026-10-02-recording-av-startup-gate.md) | `1c66112` |
| [`REC-FIRST-FRAME-AUDIO-01`](../superpowers/specs/2026-10-02-recording-first-frame-audio.md) | `2dccc43` |
| [`REC-AV-GAP-DRAIN-01`](../superpowers/specs/2026-10-02-recording-av-gap-drain.md) | `80e084c` |
| [`REC-AV-CFR-SEGMENT-01`](../superpowers/specs/2026-10-02-recording-av-cfr-segments.md) | `66ceffd` |
| [`REC-WINDOWS-IDLE-AV-01`](../superpowers/specs/2026-10-02-recording-av-idle-frontier.md) | `6518661` |
| [`REC-PENDING-FRAME-PCM-01`](../superpowers/specs/2026-10-02-recording-pending-frame-budget.md) | `120b3ad` |
| [`REC-MANIFEST-SHARING-01`](../superpowers/specs/2026-10-02-recording-manifest-sharing.md) | `d7c66bb` |
| [`REC-ARTIFACT-SHARING-01`](../superpowers/specs/2026-10-02-recording-artifact-sharing.md) | `830b12b` |
| [`REC-VIDEO-CONTROL-PREFLIGHT-01`](../superpowers/specs/2026-10-02-recording-video-control-preflight.md) | `4eb65d8` |
| [`REC-VIDEO-CONTROL-FAILURE-01`](../superpowers/specs/2026-10-02-recording-video-control-failure.md) | `7579222` |
| [`REC-AV-CONTROL-TIMELINE-01`](../superpowers/specs/2026-10-02-recording-av-control-timeline.md) | `003fe2d` |
| [`REC-AUDIO-ACTIVATION-BOUNDARY-01`](../superpowers/specs/2026-10-02-recording-audio-activation-boundary.md) | `56d750c` |
| [`REC-WASAPI-QPC-PRECISION-01`](../superpowers/specs/2026-10-02-wasapi-qpc-precision.md) | `aac5e0a` |
| [`REC-MIXED-FRAME-BOUNDARY-01`](../superpowers/specs/2026-10-02-mixed-audio-frame-boundary.md) | `4bc1977` |
| [`REC-MIXED-CONTROL-SKEW-01`](../superpowers/specs/2026-10-02-mixed-control-skew.md) | `b0b51f1` |
| [`REC-MIXED-PAUSED-STOP-01`](../superpowers/specs/2026-10-02-mixed-paused-stop.md) | `02005c9` |
| [`REC-MIXED-STOP-READY-BOUND-01`](../superpowers/specs/2026-10-02-mixed-stop-ready-bound.md) | `2bb12f9` |
| [`REC-AUDIO-CATALOG-ORDER-01`](../superpowers/specs/2026-10-02-audio-catalog-refresh-order.md) | `d05cd3e` |

证据目录：`C:\win\Clippy\src-tauri\target\windows-current-review-inventory-d05cd3e`。全局任务仍未完成；本轮没有应用、设备或桌面操作。
