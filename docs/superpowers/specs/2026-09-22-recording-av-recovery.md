# PX-REC-AV-MERGE-01 — 双轨异常录屏恢复

日期：2026-09-22

关联路线：`PX-REC-01`、`PX-REC-MERGE-01`、`PX-REC-AV-SESSION-01`

## Goal

让 schema v2 的异常 VP9 + Opus 录屏可以从已经原子提交的连续分段恢复为一个可播放、可导出、
时间线连续的双轨 WebM。视频复用原 VP9 packet；音频按每段独立编码器的边界正确应用 pre-skip
和 DiscardPadding，再编码为一条连续 Opus 轨，避免把互不共享 decoder state 的分段 packet 直接
串接。

## Requirements

1. 只有 `interrupted`、schema v2、`vp9-prototype` + `opus` + WebM 且具有非空连续分段的会话
   显示恢复入口。schema v1 继续走 `PX-REC-MERGE-01` 的纯视频无损 remux；其他组合保持不可恢复。
2. Rust 只能从经过不透明会话 ID、manifest、普通文件、长度和 SHA-256 校验的内部路径读取分段。
   前端不能提交轨道参数、分段路径、packet 顺序、时间戳或输出文件名。
3. 双轨读取器只接受 Clippy schema v2 writer 的固定子集：500 us 时间基、轨道 1 VP9、轨道 2
   Opus、无 lacing、正确尺寸与 OpusHead、每段首个视频 packet 为关键帧、两轨时间戳单调、packet
   和真实 PCM 统计与 manifest 一致。额外轨道、未知生产 block、损坏 EBML、非法 BlockGroup 或
   非尾包 DiscardPadding 必须失败。
4. 每个分段使用新的 Opus decoder。解码后丢弃该段 `preSkipFrames`，并只接收 manifest 声明的
   `pcmFrameCount`；容器末包的 DiscardPadding 必须与 `packetCount × 960 - preSkip - pcmFrameCount`
   完全一致。每段得到的真实 PCM 依 manifest 顺序接入一个新的全局 Opus encoder，不保留分段间
   decoder state，也不重复计算 pre-skip。
5. VP9 packet 不解码、不重编码；其局部时间戳加 `startedAtNs` 后与连续重编码的 Opus packet 进入
   现有有界 A/V interleaver。相同时间戳保持视频优先，队列与单调性检查不能因恢复路径而绕过。
6. 输出 `durationNs`、视频帧数、音频 packet 数和真实 PCM frame 数必须由实际 mux/encoder 结果
   产生并与 manifest 汇总一致。分段时长换算出的 48 kHz frame 边界必须与累计
   `pcmFrameCount` 一致；不允许用静默补齐掩盖损坏清单或缺失 packet。
7. 提交协议沿用纯视频恢复：先写私有 `.recording.webm.partial`，完成封尾、fsync、大小和哈希后，
   原子写入带音轨统计的 `finalOutput` 与 `finalizing`，提升文件并写 `complete`。失败时保留所有
   已提交分段和 interrupted manifest，清理未提交 partial，并允许重试。
8. 双轨恢复只在 `recording-opus-webm` feature 下编译。默认构建不引入 Opus decoder 或改变发布
   能力；结果库仍使用单一恢复锁，避免多个长录屏同时占用 CPU、磁盘和内存。

## Acceptance Criteria

- [x] 两个以上由生产 `SegmentedAvRecordingWriter` 生成的独立 VP9 + Opus 分段可恢复为一个完整
  WebM；VP9 payload 逐帧不变，解码后真实 PCM frame 数等于分段 manifest 之和。
- [x] 非 20 ms 整数边界的两个分段能正确去除每段 pre-skip/尾 padding，并由一个连续 Opus encoder
  重新封装；恢复输出仅有一个 Opus pre-skip 与一个最终 DiscardPadding。
- [ ] 单分段、mono/stereo、分段边界同时间戳视频优先、跨 cluster 和尾 padding 为零/非零均有测试。
- [ ] 篡改哈希、错误轨道/时间基/时间戳/关键帧、音频 packet 数、PCM 数、OpusHead、lacing、
  BlockGroup 形状和 DiscardPadding 均确定失败，且不改变原清单或分段。
- [x] 结果库对合格 schema v2 interrupted 会话暴露恢复入口；成功后记录完整双轨 `finalOutput`，
  播放、缩略图、导出和删除继续工作；失败可以重试。
- [ ] `recording-opus-webm` 定向与完整 Rust test/clippy、默认构建、本地完整门禁，以及同一 SHA 的
  Ubuntu、Windows、macOS 和四平台录制原型 CI 均通过。

## Out of Scope

- 不恢复未提交尾段，不修补损坏 packet，不允许用户重排、剪辑或跨会话合并。
- 不承诺音频压缩 payload 无损；异常恢复会把每段 Opus 解码成真实 PCM 后进行一次连续 Opus 重编码。
  正常停止仍直接使用会话期间持续写入的最终双轨文件，不经过该路径。
- 不在本切片加入 Linux 音频 source、macOS 麦克风、系统声/麦克风混音、漂移重采样、波形或录制 UI。
- 不用系统 FFmpeg、GStreamer 或外部命令完成产品恢复；`ffprobe` 只可作为测试环境的附加验证。

## Design Decision

双轨分段不能按纯视频方案直接 remux 音频。每段由独立 Opus encoder 产生，段首需要独立 pre-skip，
段尾也可能有 alignment padding；WebM 单一 Opus 轨只有一个 CodecDelay，无法表达中间段的 decoder
重置与再次 pre-skip。恢复因此保留 VP9 压缩数据，仅把每段 Opus 恢复成 manifest 定义的真实 PCM，
再交给一个全局 Opus encoder。读取与输出按 packet 流式进行，常驻内存受单个压缩 packet、单个
20 ms PCM block 和现有 interleaver 上限约束。

## Verification

- `cargo test --features recording-opus-webm --lib webm_remux`：6 项通过；覆盖双分段非 20 ms
  边界、VP9 payload 不变、连续 Opus 解码帧数，以及统计、codec 与 padding 篡改。
- `cargo test --features recording-opus-webm --lib recording::manifest::tests`：31 项通过；生产
  `SegmentedAvRecordingWriter` 双分段恢复成功，并覆盖失败时 manifest、分段与 partial 的事务边界。
- `cargo test --features recording-opus-webm --lib`：1148 项通过、0 失败、15 项真实桌面测试忽略；
  loopback HTTP 测试在允许本机监听的环境中通过。
- `cargo clippy --features recording-opus-webm --all-targets -- -D warnings`：通过。
- `./scripts/ci-local.sh`：25 项通过、0 失败、2 项按环境配置跳过；默认 Rust 1096 项通过、
  14 项真实桌面测试忽略，Vitest 73 个文件 / 1260 项、DOM smoke 14 项及私有 X11 回归通过。
- 同一 SHA 的 Ubuntu、Windows、macOS 与四平台录制原型 CI：推送后填写；本地 Linux 门禁不能
  替代这些条件编译与原生依赖检查。
