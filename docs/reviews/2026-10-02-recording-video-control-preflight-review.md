# WIN-NATIVE-01 / W59 — 视频控制状态预检

需求 `REC-VIDEO-CONTROL-PREFLIGHT-01`；基线 `827c7028999d2e984800c8bfe98745dde8c92d87`；分支 `codex/recording-video-control-preflight`。
规格 [视频控制状态预检](../superpowers/specs/2026-10-02-recording-video-control-preflight.md)
在生产修改前建立。无音频产品路径使用 DiagnosticRecordingSession，返回 start 时
首帧可能尚未收到；Windows WGC set_running(false) 会停止流并清除旧帧缓存。

## 复现与修复

原 worker 先调用源 pause/resume，再检查 timeline 状态。八项旧 API 合同首轮
2 passed / 6 failed。合成推送源依照真实源的暂停/恢复合同运行在原 worker 中；早期
Pause 返回 PauseBeforeFirstFrame，却留下 hooks=[pause]、running=false、frame=false，
worker/无音频 MJPEG/VP9 会话 Stop 随后为 FinishBeforeFirstFrame。重复 Pause
留下两次 pause，首帧前和录制中的无效 Resume 也调用了源 resume。
正常控制与原始 source 错误两项已经通过。

修复只在调用平台源前检查三种状态：首帧前 Pause、重复 Pause、未暂停 Resume。
返回原有 timeline 错误并继续 worker，不改变源、缓存或时钟。有效请求、源控制
失败传播、Stop、音频、帧率/队列/预算、原测试和期限保持。未增加启动等待或延长
WGC 首帧期限。源接口调用是合同证据，不是实际 WGC/WASAPI 控制或设备 QA。

## 证据

最终同字节八项夹具的 Git 原生产实现重放仍2/6，finally精确恢复 worker。
修复领域401/0=原393+新8；七项两图、一项VP9仅QA。原完整 worker 测试模块保持。
MJPEG/VP9 在拒绝早期暂停后接收首帧、停止并提交 complete 清单；核对文件长度/
SHA-256/私有权限、AVI结构与索引，VP9输出再经原严格 remux reader 完整读取，
帧数/时长匹配。文件和codec是真实I/O，帧源是合成输入。

首次修复领域400/1，新八项全部通过；未修改的旧 Opus 用例
dual_track_webm_contains_opus_timing_metadata_and_tail_padding 在 SeekPreRoll
断言得到 None，预期80000000。日志保留；同源码孤立复核1/0。其辅助函数按原始字节
查找ID，可能误命中载荷，但未保留该次失败的媒体，原因尚未证明。不得将后续成功
写成该失败根因已解决；本分支不修改该模块、断言或期限。

干净 `4eb65d8218c22e9909ab7dd9d5d16c59c548a4a4` 完整Windows门禁33 passed / 0 failed / 1 Linux smoke skipped；
默认Rust1247/QA1367各5ignored，两图重叠不累加；前端81文件/1403passed。
新7两图/新1仅QA的实际运行次数核对，已含在Rust总数；原source错误与旧控制合同保持。

## 保留边界

桌面继续停止，未启动设备、应用、安装或发布。当前SHA三宿主/codec CI、其它宿主
本地门禁、实际WGC/WASAPI/长时同步、安装器/无CRT启动、Win10/多屏仍未验。
本SHA release未构建；W58默认/QA的830b12b产物与旧安装不含本修复，证据保留。
W53历史AVI一次30秒超时没有worker原错，根因仍未知；上述旧Opus失败也保留。
证据在 `src-tauri/target/video-control-preflight-contract/` 与冻结SHA完整Windows门禁目录。
