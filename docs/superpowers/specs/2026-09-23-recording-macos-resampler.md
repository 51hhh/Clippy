# PX-REC-MACOS-RESAMPLE-01 — macOS 麦克风高质量重采样

日期：2026-09-23

关联需求：`PX-REC-MACOS-MIC-01`、`PX-REC-AV-SESSION-01`

## Goal

让 macOS 15+ 录屏 QA 构建可以把系统默认麦克风返回的常见原生采样率安全转换为现有
48 kHz stereo PCM 合同，避免 44.1/88.2/96 kHz 设备直接中止录屏。转换必须保持跨 packet
连续性、原生 PTS、暂停边界和正常停止尾部，默认/release 构建继续不启用录屏。

## Requirements

1. 只扩展 `recording-macos-av-qa` 的默认麦克风路径。ScreenCaptureKit 系统声仍请求并校验
   48 kHz；Windows、Linux、默认 Cargo feature 和正式 release 行为不变。
2. 原生输入仍只接受 1/2 声道、32-bit native-endian packed Float32 PCM。采样率必须是有限、
   可精确表示为整数 Hz 且位于 8–192 kHz；空 packet、非有限 sample、异常布局或超出预算继续
   中止双轨，不能猜测格式。
3. 48 kHz 输入走零转换直通。其它采样率使用固定版本、关闭日志与 FFT feature 的 Rubato
   band-limited asynchronous sinc resampler，固定 1,024-frame 输入块、两声道和不可在运行时调节
   的比率。不得使用线性插值、简单抽样或只修改格式标签。
4. ScreenCaptureKit 串行回调只复制和校验原生 packet，再写入容量固定的桥接队列。重采样、拆块
   和分配发生在现有音频 worker 线程；桥接满必须终止会话，不能阻塞系统回调或静默丢包。
5. 原生 PTS 按原生采样率计算 packet 结束时间，再映射到 session owner 的共享时钟。连续 packet
   共用同一 resampler 状态；PTS 空洞先封尾旧连续段，后续段从新时间戳开始，仍由下游用显式静音
   表达空洞。重叠、倒退或连续段内采样率变化必须失败。
6. 启动延迟必须从输出前缀裁掉；正常 Stop 必须补入有限零样本刷新滤波尾部，并把输出精确裁到
   `ceil(native_frames × 48000 / native_rate)`。音频 worker 在 `pipeline.finish` 前提交这些尾块。
   Pause/Drop/错误不封尾：它们清空原生队列和 resampler 状态，恢复后不得泄漏暂停前样本。
7. 归一化输出继续是 48 kHz、stereo、interleaved `f32`，单块不超过 100 ms。所有输出 sample
   必须有限，frame/sample 数、序号和时间戳使用检查运算；单个连续段和内部 staging 都受现有
   一秒音频队列及新的固定 resampler 工作集约束。
8. Rubato 及本次新增的传递依赖必须精确固定版本、lock checksum 并随包记录所选许可证。CI 至少
   在 Linux 纯合同测试以及 macOS ARM/Intel `recording-macos-av-qa` 原生 clippy/test 中覆盖该分支；
   Native QA 模板增加 44.1/96 kHz 设备、
   暂停恢复、正常停止尾部与 30 分钟 A/V 漂移。

## Research Basis

- Apple 明确说明 ScreenCaptureKit 的 `.microphone` 是独立输出；官方示例在 macOS 15+ 通过
  `addStreamOutput(..., type: .microphone, ...)` 接收麦克风 `CMSampleBuffer`：
  <https://developer.apple.com/documentation/screencapturekit/capturing-screen-content-in-macos>
- Apple 的 Audio Converter Services 把 PCM sample-rate conversion 列为独立转换能力，并区分
  转换质量与复杂度；这支持“真实重采样而不是改标签”的产品边界：
  <https://developer.apple.com/documentation/audiotoolbox/audio-converter-services>
- Rubato 5.0.0 的官方文档说明 asynchronous sinc 带抗混叠滤波，适合真实时间流；官方同时建议
  系统音频回调只写共享缓冲，由独立循环等待足够帧后重采样：
  <https://github.com/HEnquist/rubato/tree/v5.0.0>

## Acceptance Criteria

- [x] 48 kHz 直通逐 sample 不变；44.1→48、88.2→48、96→48 的输出 frame 数和时间戳精确，
  同一输入按不同 packet 边界切分得到相同输出。
- [x] 固定正弦语料证明语音频段幅度保持、96 kHz 输入中高于 24 kHz 的分量被抗混叠滤波显著抑制；
  非有限 sample、异常速率、重叠 PTS 和连续段采样率变化明确失败。
- [x] 正常 Stop 输出滤波尾部且不超过理论 frame 数；Pause/Resume、Drop 和失败清空旧状态，旧 packet
  不会进入新时间段。
- [x] 回调到 worker 的队列保持有界，满队列失败；输出仍满足 48 kHz stereo、100 ms、序号连续和
  一秒 pipeline 背压合同。
- [x] Rubato 与新增传递依赖的版本、Cargo checksum、所选许可证资源与 bundle 资源由供应链脚本固定。
- [ ] 本地完整门禁、macOS ARM/Intel 原生 feature clippy/test 与同一 SHA 三平台 CI 通过。
- [ ] macOS 15 Intel/Apple Silicon 真机分别记录 44.1/48/96 kHz 内建或外接麦克风、权限、静音、
  暂停恢复、正常停止、设备消失、强杀恢复和至少 30 分钟 A/V 漂移。

## Verification State

- 2026-09-23 Linux 本地完整门禁通过：25 项通过、0 失败；非宿主交叉 lint 与 AppImage 可视 smoke
  按配置跳过，不计作通过。
- `recording-macos-av-qa` 的纯 Rust 重采样/时间线测试与 `-D warnings` feature clippy 通过；本机
  Apple target 交叉检查在进入 Clippy 平台代码前因缺少 macOS Objective-C 编译器停止。
- macOS ARM/Intel 原生 feature CI、同一 SHA 三平台 CI 与 macOS 真机矩阵仍待远程和人工证据，
  不能由上述 Linux 结果替代。

## Out of Scope

- 系统声与麦克风混音、多音轨、设备枚举/选择、音量表、降噪、回声消除和自动增益；
- 整数 PCM、24-bit packed PCM、超过双声道或超过 192 kHz 的设备格式；
- 根据 A/V 漂移动态改变重采样比率；本切片只做固定原生率到 48 kHz，长时漂移先由 QA 量化；
- macOS 14 及更早版本的 AVFoundation 麦克风后备，或把录屏加入默认/release feature。
