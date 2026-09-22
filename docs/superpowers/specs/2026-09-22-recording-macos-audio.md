# PX-REC-MACOS-AUDIO-01 — macOS ScreenCaptureKit 系统音频 QA

## Goal

在不改变默认发布能力的前提下，把 macOS ScreenCaptureKit 的系统音频输出接入现有共享时钟、
有界 PCM worker 与 VP9 + Opus session。显式 QA 构建中的用户可以在冻结选区工具条选择无音频或
系统声；ScreenCaptureKit 视频和音频仍由同一个 lifecycle、generation token 和结果库管理。

## Requirements

1. 新增单一 `recording-macos-av-qa` feature，组合 ScreenCaptureKit、Opus/WebM 和音频缓冲区绑定；
   它不进入默认 feature 或正式 release 构建。无声视频继续支持 macOS 12.3+，系统音频只在 Apple
   官方声明支持的 macOS 13.0+ 运行时开放。
2. 只有 macOS AV QA 构建且运行时达到 macOS 13.0 才向前端声明 `none` 与 `systemAudio`；12.3–12.x
   及普通构建保持 `none`。`microphone` 在本切片继续由后端拒绝，前端不得自行扩大能力。
3. 系统声音源必须使用冻结选区所属显示器构造应用级 ScreenCaptureKit 过滤器，启用 48 kHz、
   stereo Float32 音频并排除 Clippy 自身进程声音。音频源不得从 IPC 接收显示器 ID 或格式参数。
4. `CMSampleBuffer` 必须先核验有效的 Linear PCM、48 kHz、1/2 声道、32-bit native-endian
   packed float，再把 interleaved 或 non-interleaved `AudioBufferList` 复制为 stereo interleaved
   `f32`；空缓冲、越界长度、非有限 sample 和未知布局均明确失败。
5. 每个 PCM 块的时间戳来自 `CMSampleBuffer` 原生 presentation timestamp。首个原生 PTS 锚定
   到 session owner 传入的 `RecordingSessionClock`，后续只按原生 PTS 差值映射；倒退、重叠、
   无效 timescale 和溢出必须中止会话，不能用回调抵达时刻逐块替代。
6. 单个 native packet 超过 100 ms 时按 sample frame 边界拆块。ScreenCaptureKit 回调只向有界
   队列 `try_send`；队列满、回调错误或流关闭都显式失败，不能静默丢音造成 A/V 漂移。
7. 暂停、继续、停止必须停止或启动原生 stream、清除迟到缓冲，并返回不早于最后一个 PCM frame
   末尾的共享时钟时间。初始化失败、任一轨失败和控制分歧继续由现有双轨 session 同步回收。
8. macOS ARM/Intel 编码 CI 与 Native QA 包改用组合 feature；默认 macOS 11 构建继续不强链接
   ScreenCaptureKit。供应链验证必须固定新增 feature 与 CoreAudioTypes 依赖声明。

## Acceptance Criteria

- 纯 Rust 合同测试覆盖 PTS 锚定、空洞保留、倒退/重叠/溢出拒绝、100 ms 拆块，以及 mono、
  stereo interleaved/non-interleaved PCM 归一化。
- lifecycle 能力矩阵测试证明 macOS 13+ AV QA 只开放 `none` 与 `systemAudio`，12.x 只开放
  `none`，伪造 microphone 在消费冻结会话前失败；无音频仍走原单轨 session，有系统声走现有
  双轨 session。
- macOS 原生 clippy/test 能编译 ScreenCaptureKit 音频回调、CoreMedia/CoreAudioTypes 缓冲读取、
  暂停/继续/停止和 VP9 + Opus 接线；本地完整门禁与同一 SHA 远程矩阵通过。
- 真机 QA 记录系统声、静音片段、暂停/继续、应用自身音频排除、控制窗排除、睡眠/设备切换失败
  路径和至少 30 分钟 A/V 漂移前，本需求只标记为 QA 能力，不标记为默认发布可用。

## Out of Scope

- ScreenCaptureKit microphone、系统声与麦克风混音、设备枚举、音量控制、降噪和回声消除；
- Linux PipeWire 音频、Windows WASAPI 行为变更；
- 双轨异常分段合并、波形、剪辑或正式 release 启用录屏；
- 把两个独立 ScreenCaptureKit stream 重构为一个共享原生 stream；该优化需先取得真机时延与资源
  数据，不能在本切片改变现有线程所有权。

## Verification

- 2026-09-22 Linux 本地完整门禁 `./scripts/ci-local.sh`：25 项通过、0 项失败、2 项按环境跳过；
  默认 Rust 测试 1096 项通过、14 项忽略，前端 73 个文件 / 1260 项测试通过，类型检查、clippy、
  X11 隔离回归、像素 smoke 与生产构建通过。
- `cargo test --features recording-opus-webm --lib`：1144 项通过、15 项按真实桌面条件忽略；包含
  macOS PTS/PCM 纯 Rust 合同以及共享双轨 session 回归。
- Linux 无法编译 Apple framework，macOS ARM/Intel 原生 CI 是该层的权威编译证据；真机 QA 尚未
  执行，当前能力继续保持 QA feature，未进入默认发布包。
