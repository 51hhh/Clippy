# PX-REC-LINUX-AUDIO-01 — Linux PipeWire 录屏音频

日期：2026-09-22

关联路线：`PX-REC-01`、`PX-REC-AUDIO-01`、`PX-REC-AV-SESSION-01`

## Goal

让 Linux X11 与 Wayland 录屏 QA 构建可以选择默认系统声或默认麦克风，并把 PipeWire 原生音频
作为 48 kHz stereo Float32 PCM 接入现有共享时钟、VP9 + Opus 双轨 session、暂停/继续、异常恢复
和结果库。默认安装包继续隐藏录屏与音频入口。

## Requirements

1. 新增显式组合 feature `recording-linux-av-qa`。只有 Linux 且启用该 feature 时，后端才声明
   `none`、`systemAudio` 和 `microphone`；默认/release 构建仍只声明 `none`。
2. 系统声使用 PipeWire `stream.capture.sink=true` 自动连接默认输出设备的 monitor；麦克风使用
   默认 capture source。前端不能提交 node id、设备名、格式、延迟或任意 PipeWire property。
3. PipeWire adapter 必须协商唯一格式：interleaved F32LE、48 kHz、2 channels、FL/FR。
   协商成其他格式、空 packet、越界 offset/size、错误 stride、损坏 chunk、非有限 sample 或超过
   100 ms 的原生 packet 都必须让整个双轨 session 明确失败。
4. 每个 buffer 必须带请求得到的 `SPA_META_Header`。其 `pts` 是 packet 首 sample 的原生纳秒时间；
   source 在同一线程用 `CLOCK_MONOTONIC` 与共享 `RecordingSessionClock` 做中点校准，再只按原生 PTS
   增量映射。禁止用回调到达时间为每个 packet 重新打点；时间倒退或区间重叠必须失败，真实空洞保留。
5. 原生 packet 拆成至多 20 ms 的 `CapturedAudioChunk`。gap metadata 产生等长静音，普通 packet
   复制实际 sample；序号、时间戳、frame/sample 长度和数值有限性在进入公共 pipeline 前验证。
6. PipeWire 对象只存在于专用 loop 线程。回调与音频 worker 之间使用最多八个 chunk 的有界桥接；
   队列满时中止，不静默丢音。初始化只有在 stream 进入 Streaming 且格式验证通过后成功，错误、
   提前关闭和五秒超时均可观察。
7. pause/resume 通过 `pw_stream_set_active` 控制，并在回执边界清除迟到 chunk；resume 后只接收不早于
   恢复时刻的 packet。stop 先断开 stream、退出 loop 并 join，再返回不早于最后 PCM frame 末尾的
   共享时钟时间；Drop 走同一有界回收路径。
8. X11 与 Wayland 的平台帧计划都生成同一种 Linux 音频计划。现有双轨 lifecycle、Opus/WebM、
   schema v2、结果库和异常恢复不复制平台分支。
9. Ubuntu 22.04 原型 CI 必须以 `recording-linux-av-qa` 编译和运行 Rust test/clippy；Native QA Linux
   包同时启用 `recording-wayland-qa,recording-linux-av-qa`，并在包内元数据记录两个 feature。

## Acceptance Criteria

- [x] 纯 Rust 合同覆盖中点校准、前后方向 PTS、真实间隔、倒退/重叠、20 ms 拆块、gap 静音、
  非有限 sample、非法长度和序号溢出。
- [x] Linux feature 构建中，X11 与 Wayland 均暴露系统声和麦克风；默认构建保持只有无音频。
- [x] PipeWire source 在格式验证后才初始化成功；有界队列、stream error、暂停清空、恢复过滤、
  stop/join 和 Drop 均有代码级保护。
- [x] `cargo test --features recording-linux-av-qa --lib`、对应 clippy、默认构建和完整
  `./scripts/ci-local.sh` 通过。
- [ ] 同一 SHA 的 Ubuntu 22.04 原型 CI 与 Linux Native QA 包构建通过。
- [ ] GNOME X11、GNOME Wayland、KDE Wayland 和 wlroots 真机分别验证系统声、麦克风、暂停、
  设备消失、强杀恢复与至少 30 分钟 A/V 漂移。

## Out of Scope

- 本切片不做设备枚举/选择、系统声与麦克风混音、音量表、降噪、回声消除或自动增益。
- 不把录屏或音频 feature 加入默认/release；不宣称 PipeWire Portal/Flatpak 权限和所有 session manager
  已经通过真机验收。
- 不改变视频源、Opus 编码、WebM mux、schema v2 或异常恢复协议；发现这些公共合同缺陷时另建切片。
- 不用 PulseAudio 命令、FFmpeg、GStreamer 或外部进程作为产品采集后端。
