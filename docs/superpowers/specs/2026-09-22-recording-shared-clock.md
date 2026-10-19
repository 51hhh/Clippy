# PX-REC-CLOCK-01 录屏共享会话时钟

## Goal

为录屏视频采集与后续音频采集建立唯一的会话单调时钟。会话 owner 在任何平台帧源初始化前创建
时钟，X11、Wayland/PipeWire、Windows WGC、macOS AVFoundation 与 ScreenCaptureKit 都只能消费
这一个时钟，不能在平台适配器内各自建立时间原点。

## Requirements

- `RecordingSessionClock` 由录屏会话创建一次，可安全复制到采集线程和原生回调线程；时间戳使用
  `Instant` 单调时钟并以会话创建时刻为零点，不读取系统墙钟；
- `DiagnosticRecordingSession` 把同一时钟交给帧源 factory。直接传入合成帧源的测试/诊断入口保持
  兼容，但生产平台 factory 必须显式接收时钟；
- `PlatformFrameSourcePlan::connect` 及所有平台 `connect` 必须接收共享时钟。平台模块中不得再用
  `Instant::now()` 创建视频采集时间原点；
- 原生回调或采集线程按共享时钟给帧加戳。相同采样值仍按已有规则提升至少 1 ns，保证单一帧源
  内严格递增；暂停、继续和停止必须读取同一时钟域；
- 首帧等待超时继续使用独立的本地等待计时，不能把会话创建到平台授权/初始化的耗时误算为
  “等待首帧”；
- 当前视频 `RecordingTimeline` 继续以首个已接受视频帧为呈现零点，VP9 首帧仍为 0。共享会话
  时间戳不得直接写成非零 VP9 首帧；
- 音频原生 PTS 接入时必须映射到同一 `RecordingSessionClock`。回调到达时间只能用于视频当前
  帧源的既有采样策略，不能冒充音频 PTS。

## Acceptance Criteria

- 单元测试证明时钟副本对同一个采样时刻产生完全相同的会话时间戳，且会话 factory 收到由
  session owner 创建的时钟；
- 代码检查证明五条平台视频路径均从 `PlatformFrameSourcePlan::connect` 接收时钟，平台帧源中
  不再存在名为 `clock_origin` 的私有原点或为采集时间戳创建的 `Instant::now()`；
- X11 帧源测试与录屏 session/lifecycle 测试继续覆盖连接、暂停、继续、停止和首帧归零；
- 默认构建、录屏 VP9 原型构建、Rust clippy 与仓库完整本地门禁通过；
- 同一提交的 Ubuntu 主检查、Windows/macOS 原生检查及四平台录屏编码原型 CI 均成功后，才把
  本切片记为跨平台编译验证通过。

## Out of Scope

- 接入 WASAPI、ScreenCaptureKit audio 或 Linux PipeWire 音频节点；
- 原生音频 PTS 校准、重采样、Opus 编码、WebM 音轨和 A/V mux epoch 选择；
- 改变现有视频首帧归零、暂停扣时、分段时间戳或 VP9 mux 合同；
- 用共享时钟单元测试代替四平台真机 A/V 同步、漂移和长时间录制验收；
- 开放默认发布录屏入口或改变现有 QA feature 门控。

## Follow-up contract

下一切片在平台音频源存在后引入 A/V coordinator：用共享会话时间戳校准原生音频 PTS，明确首个
视频帧前的音频如何裁切或保留，并选择统一 mux epoch。当前 VP9 writer 要求首帧呈现时间为 0，
因此在 coordinator 给出双轨策略前，不把 session timestamp 直接当作容器 presentation timestamp。

## Verification

- `cargo test recording::`：142 项通过、2 项需真实 X11 的测试忽略；
- 默认、`recording-vp9-prototype` 与 `recording-wayland-qa` 三组 `cargo check` 通过；
- 默认、VP9 原型与 Wayland QA 三组 `cargo clippy --all-targets -- -D warnings` 通过；
- `./scripts/ci-local.sh`：25 项通过、0 失败、2 项可选检查跳过；Rust 1059 项通过、14 项忽略，
  前端 73 个文件 1251 项通过，真实 X11 录屏闭环、X11 私有剪贴板协议与 Canvas/布局像素 smoke
  通过；
- Windows/macOS 条件编译与同 SHA 原生 CI 尚未由本分支验证，不能由 Linux 本地门禁替代。
