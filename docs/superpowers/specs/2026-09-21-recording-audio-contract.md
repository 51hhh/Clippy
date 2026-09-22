# PX-REC-AUDIO-01 — 单音轨采集与 A/V 时间线合同

## Goal

在接入任一平台音频权限或设备 API 前，先固定录屏单音轨的内存、格式、时钟、暂停和背压语义，避免
各平台各自生成无法与视频可靠对齐的时间戳。该切片只建立可测试的领域合同，不开放音频 UI，也不把
现有录屏 QA 入口升级为默认能力。

## Requirements

1. 第一阶段只允许一个音频来源，类型为系统声音或麦克风；不能在没有混音、回声和权限设计时同时
   打开两个来源。
2. 平台层必须先归一化为 48 kHz、单声道或双声道、交错 `f32` PCM。领域层拒绝 NaN、无穷值、
   样本数量不匹配和超过 100 ms 的单块输入。
3. 每个块的时间戳表示第一个 PCM frame 在会话单调时间基中的时刻。平台层必须用 WASAPI QPC、
   CMSampleBuffer PTS 或 PipeWire/SPA PTS 等原生时间戳映射到该时间基；不能把回调到达时间当作稳定
   音频时钟。音频时间线使用会话显式起点，因而能保留“视频先开始、音频稍后到达”的真实起始空白；
   不能把每条轨道的首块都强制改写为 0。
4. 暂停区间从音频呈现时间中扣除。块序号、采集时间和呈现区间必须单调且不重叠；允许设备回调产生
   可观测空洞，后续编码层决定补静音或终止，领域层不能静默挤压时间线。
5. 音频队列最多持有一秒双声道 `f32` PCM。队列满时返回明确背压错误并保持队列和时间线不变，
   不允许像视频那样丢弃中间块造成听感破裂和 A/V 漂移。
6. 20 ms 作为后续 Opus 默认编码块；2.5/5/10/20/40/60 ms 编码边界、重采样、编码器 lookahead、
   WebM `CodecPrivate`、`CodecDelay` 和 `SeekPreRoll` 在后续 mux 切片单独实现和验证。

## Acceptance Criteria

- [x] 固定测试覆盖 48 kHz mono/stereo、精确样本长度、非有限样本、空块和 100 ms 上限。
- [x] 显式会话起点能保留首块偏移；连续块、真实空洞、重叠、重复序号和倒退时间戳结果确定。
- [x] 暂停期间输入不进入队列，恢复后的呈现时间扣除完整暂停区间，停止时长与同一时间基一致。
- [x] 一秒队列预算严格按实际 PCM 字节计算；背压失败不推进序号或时间线，重试同一块仍可成功。
- [x] 代码、规格、路线图和 CHANGELOG 对“已完成内部合同、尚无平台采集/编码/入口”的边界一致。

## Platform direction

- Windows 系统声音使用 WASAPI shared-mode loopback；麦克风使用 capture endpoint。两个来源不会在
  第一阶段同时开放。
- macOS 12.3+ 继续复用 ScreenCaptureKit 会话，配置 48 kHz、mono/stereo，并从 audio sample buffer
  取得 PCM；不能再启动第二个与视频无共同时间基的采集会话。
- Linux 的 ScreenCast Portal 只定义 monitor/window/virtual 视频来源，不据此声称获得系统声音。
  PipeWire 音频节点、Portal/桌面权限差异和麦克风权限需要独立能力探测，未支持时入口保持隐藏。

## Out of Scope

- 本切片不接 WASAPI、ScreenCaptureKit audio 或 PipeWire 音频节点；
- 不引入 Opus 依赖，不修改 WebM 轨道，不更改 manifest schema；
- 不增加设备选择、音量表、混音、回声消除、摄像头、降噪或默认产品入口；
- 不用合成时间线测试替代 Windows、macOS、X11、Wayland 的真机 A/V 同步验收。

## Verification

- `cargo test recording::audio::tests`：8 项通过，覆盖格式、显式起点、空洞/重叠、暂停、停止、
  背压原子性和异常终态；
- `cargo clippy --all-targets -- -D warnings`：通过；
- `./scripts/ci-local.sh`：25 项通过、0 失败、2 项可选检查跳过；Rust 1056 项通过、14 项忽略，
  前端 73 个文件 1250 项通过，X11 私有协议与 Canvas/布局像素 smoke 通过；
- Windows/macOS 条件编译与同 SHA 原生 CI 尚未由本分支验证，不能由 Linux 本地门禁替代。

## Primary references

- Microsoft WASAPI loopback recording: https://learn.microsoft.com/windows/win32/coreaudio/loopback-recording
- Apple ScreenCaptureKit audio configuration: https://developer.apple.com/videos/play/wwdc2022/10156/
- XDG ScreenCast Portal: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html
- Opus frame durations: https://www.rfc-editor.org/rfc/rfc6716#section-2.1.4
- Opus encapsulation and pre-roll: https://www.rfc-editor.org/rfc/rfc7845#section-4.6
