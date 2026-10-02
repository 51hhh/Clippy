# WIN-NATIVE-01 / W74 — Windows 审查与交付状态 inventory

当前被测生产源码 `02005c98ee6ac52fd46e20c878a717f314fd90cc`，前置文档 `b85f462b36af969663f6f7c66e47b74cdd6e0bc0`。本轮仅审查/记录，产品和测试字节保持；新修复/新测试0。

47项修复的源码均在当前历史中，逐项核对 RID、规范、CHANGELOG、原完整门禁源码/成功状态和已保存日志哈希。此前29项详细正文审计保持原SHA含义，未将比较次数计作新通过数。

当前Windows完整门禁33 passed / 0 failed / 1 Linux smoke skipped；默认1308、QA1457，各5 ignored；前端81文件/1403，完整录屏领域491。它们是W72已运行结果，本轮没有重跑，两图重叠不累加。W73同源码默认/QA release编译和PE/CRT来源核对已完成，仅文件证据，未启动/安装。

## 全部原需求与验收边界

R1基线/关键Git节点和本地47项历史；R2 Windows前端/类型/边界/供应链/构建；R3 PowerShell退出状态及默认/QA/vendor/Python门禁；R4 Windows CI前端配置；R5 DPI/权限/私有文件/WGC/WASAPI/控制/恢复源码合同；R6非默认录屏feature；R7独立修复与规范/CHANGELOG；R8 OCR当前用户DACL，均保留原要求。R4配置存在不等于当前CI成功，R5/R7代码合同不等于真机复测。

原九条AC的最后两条继续未勾选；基线与42e52c0的历史CI仍仅证明对应SHA。GitHub当前02005c9提交查询422“No commit found”，已知45769c9查询成功作为访问正例；当前源码尚不可在该仓库取得。combined status/PR-triggered首分页run为空只作附属记录，不能独自证明所有CI不存在；没有推送/触发CI。

保留旧包39项桌面记录：2 passed / 1 failed / 36 not_run，已安装457包不含后续47项修复。当前源码设备/桌面复测、Windows10、多屏/混合DPI/负坐标、权限、安装升级卸载/updater、开发CRT缺席环境、长时同步、Linux本地完整门禁/Wayland、macOS和同SHA七项CI仍未完成。W53/W59/W63/W67历史失败根因仍未明。

## 47项本地修复

| 需求 | 原被测源码 | 规范 |
|---|---|---|
| WIN-PIN-TOOLBAR-01 | `7aa6cf6` | [规范](../superpowers/specs/2026-10-01-windows-pin-toolbar-height.md) |
| WIN-PRIVATE-WRITE-01 | `f788b1f` | [规范](../superpowers/specs/2026-10-01-windows-private-write-order.md) |
| WIN-LONGSHOT-CURSOR-01 | `d8dff80` | [规范](../superpowers/specs/2026-10-01-windows-longshot-cursor-restore.md) |
| WIN-CF-HTML-01 | `50b7778` | [规范](../superpowers/specs/2026-10-01-windows-cf-html-bounds.md) |
| WIN-CLIP-IMAGE-BUDGET-01 | `531d791` | [规范](../superpowers/specs/2026-10-01-windows-image-decode-budget.md) |
| WIN-DIBV5-PIXEL-01 | `25fb5d7` | [规范](../superpowers/specs/2026-10-01-windows-dibv5-pixel-offset.md) |
| WIN-PASTE-RECHECK-01 | `14bf616` | [规范](../superpowers/specs/2026-10-01-windows-paste-input-recheck.md) |
| WIN-WASAPI-STOP-TAIL-01 | `a463c3b` | [规范](../superpowers/specs/2026-10-01-windows-wasapi-stop-tail.md) |
| WIN-WGC-CLOSE-01 | `03b4cb8` | [规范](../superpowers/specs/2026-10-01-windows-wgc-close-retry.md) |
| REC-AV-BRIDGE-JOIN-01 | `091b5cb` | [规范](../superpowers/specs/2026-10-01-windows-av-bridge-join.md) |
| WIN-WGC-INIT-ROLLBACK-01 | `61d6823` | [规范](../superpowers/specs/2026-10-01-windows-wgc-init-rollback.md) |
| WIN-REGISTRY-BUFFER-01 | `bb38cc6` | [规范](../superpowers/specs/2026-10-01-windows-registry-buffer.md) |
| WIN-WINDOW-SCALE-01 | `78bd83f` | [规范](../superpowers/specs/2026-10-01-windows-window-candidate-scaling.md) |
| WIN-OVERLAY-FOCUS-01 | `f577996` | [规范](../superpowers/specs/2026-10-01-windows-overlay-focus.md) |
| WIN-NATIVE-MONITOR-01 | `2f0e225` | [规范](../superpowers/specs/2026-10-01-windows-physical-monitor-bounds.md) |
| WIN-PIN-ORIGIN-01 | `888127a` | [规范](../superpowers/specs/2026-10-01-windows-pin-physical-origin.md) |
| WIN-PIN-WORKAREA-01 | `4a58101` | [规范](../superpowers/specs/2026-10-02-windows-pin-workarea.md) |
| WIN-PIN-LIVE-DPI-01 | `e339042` | [规范](../superpowers/specs/2026-10-02-windows-pin-live-dpi.md) |
| WIN-WGC-BRIDGE-ROLLBACK-01 | `7922457` | [规范](../superpowers/specs/2026-10-02-windows-wgc-bridge-rollback.md) |
| WIN-MAIN-TARGET-01 | `c9d5512` | [规范](../superpowers/specs/2026-10-02-windows-main-window-target.md) |
| WIN-CONTROL-ROLLBACK-01 | `a52ecaa` | [规范](../superpowers/specs/2026-10-02-windows-control-rollback.md) |
| REC-DELETE-OWNER-01 | `0321355` | [规范](../superpowers/specs/2026-10-02-recording-delete-owner.md) |
| WIN-EXPORT-IDENTITY-01 | `69cf0b4` | [规范](../superpowers/specs/2026-10-02-windows-export-identity.md) |
| REC-MEDIA-REVOKE-01 | `6944b68` | [规范](../superpowers/specs/2026-10-02-recording-media-revoke.md) |
| REC-PLAYBACK-LIFECYCLE-01 | `ba26a83` | [规范](../superpowers/specs/2026-10-02-recording-playback-lifecycle.md) |
| REC-LIBRARY-READY-01 | `1907809` | [规范](../superpowers/specs/2026-10-02-recording-library-ready.md) |
| WIN-SHORTCUT-SHARED-01 | `0f793c9` | [规范](../superpowers/specs/2026-10-02-windows-shortcut-shared.md) |
| WIN-CLIP-SNAPSHOT-01 | `b541e87` | [规范](../superpowers/specs/2026-10-02-windows-clipboard-snapshot.md) |
| WIN-PASTE-CLEANUP-01 | `f5ad5da` | [规范](../superpowers/specs/2026-10-02-windows-paste-key-cleanup.md) |
| WIN-QA-CRT-01 | `5d900ca` | [规范](../superpowers/specs/2026-10-02-windows-qa-crt-deployment.md) |
| WIN-QA-CRT-DISCOVERY-01 | `ae2fdb6` | [规范](../superpowers/specs/2026-10-02-windows-qa-crt-discovery.md) |
| REC-AV-STARTUP-GATE-01 | `1c66112` | [规范](../superpowers/specs/2026-10-02-recording-av-startup-gate.md) |
| REC-FIRST-FRAME-AUDIO-01 | `2dccc43` | [规范](../superpowers/specs/2026-10-02-recording-first-frame-audio.md) |
| REC-AV-GAP-DRAIN-01 | `80e084c` | [规范](../superpowers/specs/2026-10-02-recording-av-gap-drain.md) |
| REC-AV-CFR-SEGMENT-01 | `66ceffd` | [规范](../superpowers/specs/2026-10-02-recording-av-cfr-segments.md) |
| REC-WINDOWS-IDLE-AV-01 | `6518661` | [规范](../superpowers/specs/2026-10-02-recording-av-idle-frontier.md) |
| REC-PENDING-FRAME-PCM-01 | `120b3ad` | [规范](../superpowers/specs/2026-10-02-recording-pending-frame-budget.md) |
| REC-MANIFEST-SHARING-01 | `d7c66bb` | [规范](../superpowers/specs/2026-10-02-recording-manifest-sharing.md) |
| REC-ARTIFACT-SHARING-01 | `830b12b` | [规范](../superpowers/specs/2026-10-02-recording-artifact-sharing.md) |
| REC-VIDEO-CONTROL-PREFLIGHT-01 | `4eb65d8` | [规范](../superpowers/specs/2026-10-02-recording-video-control-preflight.md) |
| REC-VIDEO-CONTROL-FAILURE-01 | `7579222` | [规范](../superpowers/specs/2026-10-02-recording-video-control-failure.md) |
| REC-AV-CONTROL-TIMELINE-01 | `003fe2d` | [规范](../superpowers/specs/2026-10-02-recording-av-control-timeline.md) |
| REC-AUDIO-ACTIVATION-BOUNDARY-01 | `56d750c` | [规范](../superpowers/specs/2026-10-02-recording-audio-activation-boundary.md) |
| REC-WASAPI-QPC-PRECISION-01 | `aac5e0a` | [规范](../superpowers/specs/2026-10-02-wasapi-qpc-precision.md) |
| REC-MIXED-FRAME-BOUNDARY-01 | `4bc1977` | [规范](../superpowers/specs/2026-10-02-mixed-audio-frame-boundary.md) |
| REC-MIXED-CONTROL-SKEW-01 | `b0b51f1` | [规范](../superpowers/specs/2026-10-02-mixed-control-skew.md) |
| REC-MIXED-PAUSED-STOP-01 | `02005c9` | [规范](../superpowers/specs/2026-10-02-mixed-paused-stop.md) |

## 下一项代码疑点

活跃状态MixedAudioSource Stop在audio_mixer.rs的stop_both里一次性pop_ready并积累Vec，输出大小随剩余时间段增长；PX-REC-AUDIO-MIX-01的固定ready上限未由现有短尾块覆盖证明。尚未在原API复现，不计已确认缺陷。下一步先建立REC-MIXED-STOP-READY-BOUND-01和适度有限间隔的原API对照，再决定修复；不能通过拒绝所有长合法尾部、截断PCM、降低时间戳、弱化背压或改旧期限完成。

机器记录 `C:\win\Clippy\src-tauri\target\windows-readiness-inventory-02005c9\RESULT.json`；原47项门禁、W73构建、旧inventory和历史失败记录均保留。全局任务仍进行中，本轮没有桌面/设备操作。
