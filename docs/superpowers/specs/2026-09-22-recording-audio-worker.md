# PX-REC-AUDIO-WORKER-01 音频采集线程与控制合同

## Goal

把平台音频回调与 `AudioPipeline` 之间的线程、控制和错误传播固定为可测试合同，使后续 WASAPI、
ScreenCaptureKit audio 与 PipeWire 音频适配器都在采集线程内创建、使用和销毁，并把原生 PTS 映射
到同一个 `RecordingSessionClock`。

## Requirements

- `RecordingAudioSource` 只在音频采集线程内使用，不要求 `Send`；跨线程移动的 factory 必须接收
  session owner 创建的 `RecordingSessionClock`，平台源不能另建时间原点；
- source 每次按 worker 给出的最长等待时间返回一个 PCM 块或 `None`。该等待必须有界，使暂停、继续、
  停止和 Drop 回收不会被永久阻塞；
- 每个块先由现有 `AudioPipeline` 完成 48 kHz、mono/stereo、样本长度、时间线和一秒预算校验。队列
  背压必须终止当前生产路径并保留已入队前缀，不能丢块后继续录制；
- 暂停先停止平台采集并取得同一会话时钟的时间戳，再暂停 pipeline；暂停期间不得调用取块。继续
  先由平台丢弃旧回调并返回恢复时间戳，再恢复 pipeline；
- 正常停止先关闭平台流并取得同一时钟域终点，再封尾音频时间线。初始化、取块、控制、背压、线程
  panic 或控制通道断开都中止 pipeline，不能把异常路径标成正常完成；
- 初始化结果必须同步返回给 session owner。线程启动、factory 失败或 panic 时不得遗留活动线程；
  `Drop` 必须请求中止并 join；
- worker 不选择系统声音或麦克风，不负责重采样、Opus、静音补齐、混音或 WebM mux。

## Acceptance Criteria

- 测试证明非 `Send` source 在音频线程内创建和销毁，并确实接收共享会话时钟；
- 测试覆盖正常采集/停止、暂停期间零取块、继续后的同一时间线、平台 hook 顺序和控制错误；
- 测试覆盖一秒队列背压：第一个超预算块明确失败、pipeline 进入异常终态、已入队 PCM 前缀保持可读；
- 测试覆盖 factory 错误、factory panic、source 错误、Drop 回收和控制通道终态；
- `cargo test recording::audio_worker::tests`、`cargo clippy --all-targets -- -D warnings` 与仓库完整本地
  门禁通过；同一 SHA 的 Windows/macOS 原生 CI 仍是跨平台编译门槛。

## Out of Scope

- 实现任一平台设备枚举、权限提示或原生音频 source；
- 把 worker 接入当前产品录屏 session、manifest、控制窗或设置 UI；
- 引入 Opus、WebM 音轨或决定首帧前音频的 mux epoch；
- 用合成 source 测试代替真机系统声音/麦克风的格式、PTS、漂移、热插拔和权限验收。

## Verification

- `cargo test recording::audio_worker::tests`：9 项通过，覆盖正常停止、暂停/继续、平台控制错误、
  pipeline 控制错误、严格背压、source/factory 失败、factory panic、`!Send` 线程亲和与 Drop 回收；
- `cargo test recording::`：151 项通过、2 项按真实 X11 环境要求忽略；
- 默认、`recording-vp9-prototype` 与 `recording-wayland-qa` 三套编译图的 `cargo check` 和
  `cargo clippy --all-targets -- -D warnings` 均通过；
- `./scripts/ci-local.sh`：25 项通过、0 失败、2 项可选检查跳过；Rust 1068 项通过、14 项忽略，
  前端 73 个文件 1251 项通过，真实 X11 录屏、私有剪贴板、DOM、Canvas、布局与生产构建均通过；
- Windows/macOS 条件编译与同一 SHA 原生 CI 需在本分支推送后验证，Linux 本地结果不能替代。
