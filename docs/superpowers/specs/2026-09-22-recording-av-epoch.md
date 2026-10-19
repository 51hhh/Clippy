# PX-REC-AV-EPOCH-01 — 录屏双轨共同起点合同

## Goal

在引入 Opus 与 WebM 音轨前，固定视频首帧归零和音频会话起点之间的唯一映射。协调层必须以首个
已接受视频帧作为容器时间零点，裁掉该时刻以前的 PCM，并让暂停后的音频与现有视频 presentation
timeline 继续使用相同坐标，避免编码器各自猜测偏移。

## Requirements

1. 会话 owner 仍只创建一个 `RecordingSessionClock`。音频 pipeline 的 origin 是该时钟的会话起点；
   视频 pipeline 继续把首个有效帧映射为 presentation `0`，不改变现有纯视频文件时间线。
2. A/V epoch 只能由首个已接受视频帧建立一次；该帧的 presentation 必须是 `0`，source timestamp
   不能早于音频会话 origin。
3. epoch 前完整结束的音频块必须丢弃；跨越 epoch 的块按 48 kHz sample 边界向上取整裁切前缀，
   保留第一个不早于 epoch 的 PCM frame，不能把真实采样点强行改写到 `0`。
4. epoch 后音频 presentation 为原音频 presentation 减去 epoch offset。输出块必须保持序号、格式、
   样本长度、单调区间和真实 gap；协调失败不得推进已提交的输出时间线。
5. 视频与音频暂停/恢复必须由未来的 session owner 使用同一对会话时钟边界驱动。两条 pipeline 都
   扣除同一暂停区间后，减去固定 epoch offset 必须继续得到相同 presentation 坐标。
6. 正常结束同时保留视频和音频各自的实际时长，并计算谁领先及差值；容器时长取两轨最大值，不能
   静默截断较长轨道或用补帧掩盖漂移。
7. 本切片只建立无 I/O 的协调合同，不启动平台音源、不编码 Opus、不修改 WebM、manifest 或 UI。

## Acceptance Criteria

- 首视频帧合法性、重复 epoch、source/origin 溢出均有确定错误。
- 自动化覆盖 epoch 前丢弃、整 sample 裁切、非整 sample 向上取整、epoch 后平移、真实 gap 与重叠拒绝。
- 使用真实 `RecordingPipeline` 与 `AudioPipeline` 的集成测试证明首帧起点和共同暂停区间在双轨上对齐。
- 结束结果保留两轨时长、容器最大时长和音频领先/落后量；音频在 epoch 前结束会被拒绝。
- 默认、VP9、Wayland QA 和 Windows audio feature 编译图保持通过；仓库完整本地门禁通过。
- 同一 SHA 的 Ubuntu、Windows、macOS 原生 CI 是跨平台条件编译门槛，Linux 本地结果不能替代。

## Out of Scope

- Opus encoder、`OpusHead`、codec delay、seek pre-roll 和 WebM 双轨 mux。
- 音频空洞补静音、重采样、漂移拉伸、响度处理、混音和回声消除。
- 把音频 worker 接入 `DiagnosticRecordingSession`，或改变 pause/resume 的平台调用顺序。
- macOS ScreenCaptureKit audio、Linux PipeWire 音频、设备 UI 和默认发布能力。

## Timing Decision

WebM 的公共时间轴沿用现有视频语义：首个已接受视频帧是 `0`。音频在会话时钟上可能更早开始，
因此必须在 sample 边界裁掉首视频帧以前的前缀；之后只做固定平移。这样纯视频输出保持兼容，音频
不会因回调先到而制造播放器开头的黑帧，也不会把跨 epoch 的真实首个 sample 提前。

## Verification

- `cargo test --lib recording::av_timeline::tests`：7 项通过，覆盖 epoch 建立、前缀丢弃、整 sample
  与非整 sample 裁切、共同暂停边界、事务性错误和双轨结束偏差。
- `cargo test --lib recording::`：165 项通过，2 项按环境忽略。
- 默认、`recording-vp9-prototype`、`recording-wayland-qa` 与 `recording-windows-audio` 编译图均通过
  `cargo clippy --all-targets -- -D warnings`；VP9 和 Wayland QA 编译图下的本模块 7 项测试分别通过。
- `./scripts/ci-local.sh`：25 项通过、0 失败、2 项按平台条件跳过；Rust 共 1096 项，其中
  1082 项通过、14 项忽略；前端 73 个文件、1251 项测试通过，DOM/Xvfb、Canvas 像素、主窗口
  布局、Vite 生产构建和构建产物入口 smoke 均通过。
- Windows 与 macOS 条件编译和原生运行仍以本提交同一 SHA 的远程 CI 为准，本地 Linux 结果不替代。
