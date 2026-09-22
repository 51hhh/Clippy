# PX-REC-MACOS-MIC-01 — macOS 15 ScreenCaptureKit 麦克风

日期：2026-09-22

关联需求：`PX-REC-MACOS-AUDIO-01`、`PX-REC-AV-SESSION-01`

## Goal

让 macOS 15+ 的显式录屏 QA 构建可以选择系统默认麦克风，并将 ScreenCaptureKit 独立麦克风
输出接入现有共享时钟、48 kHz stereo PCM worker、VP9 + Opus 双轨 session、暂停/继续、异常恢复
和结果库。macOS 12.3–12.x 保持无音频，macOS 13–14 保持只有系统声，默认/release 构建不变。

## Requirements

1. 沿用非默认 `recording-macos-av-qa` feature。只有 macOS 15+ 运行时向前端声明 `microphone`；
   macOS 13–14 只声明 `none` 与 `systemAudio`，macOS 12.3–12.x 只声明 `none`。
2. 麦克风计划只能来自冻结选区的可信平台计划。使用 `SCStreamConfiguration.captureMicrophone`
   与 `SCStreamOutputTypeMicrophone` 捕获系统默认设备；IPC 不接受设备 ID、格式或权限状态。
3. 调用 macOS 15 selector 前必须同时通过系统版本与 Objective-C selector 可用性检查。旧系统不能
   因为 QA 包最低版本仍为 12.3 而收到未知 selector。
4. QA app 的 Info.plist 必须包含非空 `NSMicrophoneUsageDescription`，Native QA 构建必须验证该键；
   默认/release 配置继续不包含麦克风用途声明和录屏 feature。
5. 麦克风 packet 使用独立 `.microphone` 回调。原生格式来自设备，当前切片只接受 48 kHz、
   1/2 声道、32-bit native-endian packed float；mono 上混为 stereo。其它采样率、声道数、布局、
   空 packet、非有限 sample 或越界缓冲必须中止双轨，不能按错误速率写入。
6. 原生 presentation timestamp 继续通过 `MacAudioPtsMapper` 映射到 session owner 的共享时钟；
   有界队列、100 ms 拆块、暂停清空、恢复过滤、停止和 Drop 沿用系统声音源合同。
7. 系统声与麦克风是互斥的单音轨选择，不在本切片混音。输出仍使用现有 schema v2、Opus/WebM、
   结果库、持久缩略图和异常恢复路径。
8. macOS ARM/Intel 原型 CI 必须编译并测试麦克风分支；Native QA 模板增加权限拒绝/允许、静音、
   暂停恢复、设备消失和长时漂移场景。

## Research Basis

- Apple 的 ScreenCaptureKit 示例把麦克风列为 macOS 15 新增能力，并通过独立 `.microphone` output
  接收 sample buffer：<https://developer.apple.com/documentation/screencapturekit/capturing-screen-content-in-macos?language=objc>
- `SCStreamOutputTypeMicrophone` 使用所选麦克风设备的原生格式，不受系统声的 `sampleRate` 与
  `channelCount` 设置控制：<https://developer.apple.com/documentation/screencapturekit/scstreamoutputtype>
- 访问麦克风必须在最终 app Info.plist 中提供 `NSMicrophoneUsageDescription`：
  <https://developer.apple.com/documentation/BundleResources/Information-Property-List/NSMicrophoneUsageDescription>

## Acceptance Criteria

- [x] 纯运行时策略测试覆盖 12.3、13、14、15 的能力矩阵，伪造旧系统 microphone 在消费冻结会话
  前失败。
- [ ] 系统声与麦克风计划选择不同的 configuration/output type，麦克风路径不会接收系统声回调。
- [x] Info.plist、Native QA workflow、供应链脚本和回归测试共同固定 macOS 15+ 麦克风边界。
- [ ] macOS ARM/Intel 原生 feature clippy/test 与本地完整门禁通过。
- [ ] macOS 15 Intel/Apple Silicon 真机记录权限允许/拒绝、默认麦克风、静音、暂停恢复、设备消失、
  强杀恢复和至少 30 分钟 A/V 漂移。

本地 Linux 门禁已通过（25 项检查，0 失败，2 项按配置跳过），另行通过 1155 项
`recording-opus-webm` Rust 测试及 feature clippy。Linux 无 macOS SDK，Apple target 的交叉检查
在 Objective-C exception helper 编译阶段停止，不能代替 macOS ARM/Intel 原生 CI，因此后两项继续
保持未完成。

## Out of Scope

- 系统声与麦克风混音、多音轨、设备枚举/选择、音量表、降噪、回声消除和自动增益。
- 非 48 kHz 麦克风的高质量重采样；遇到此格式必须显式失败，不能线性插值或伪造 48 kHz。
- macOS 14 及更早版本的 AVFoundation 麦克风后备。
- 默认/release 启用录屏，或改变 macOS 12.3 QA 包的最低系统版本。
