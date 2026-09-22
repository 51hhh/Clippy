# PX-REC-WINDOWS-AUDIO-01 — Windows WASAPI 音频源

## Goal

在不改变默认产品入口的前提下，为录屏音频线程提供可编译、可测试的 Windows WASAPI source，
分别支持默认扬声器的系统声音 loopback 与默认麦克风输入。source 必须输出现有音频合同要求的
48 kHz、双声道、交错 `f32` PCM，并把 WASAPI 的 QPC 时间戳映射到视频共用的
`RecordingSessionClock`。

## Requirements

1. 新能力只进入显式 `recording-windows-audio` feature；默认构建、正式 release 和现有录屏 UI
   不自动开启音频。
2. 平台计划可选择默认系统声音或默认麦克风；一次会话只创建一个音源，不做混音。
3. COM apartment、默认 endpoint、`IAudioClient`、`IAudioCaptureClient` 与事件句柄必须在音频
   worker 线程内创建、使用和销毁。
4. 系统声音使用 shared-mode loopback；麦克风使用 shared-mode capture。两者统一请求
   48 kHz stereo IEEE float，并显式启用 Windows Audio Engine 的 PCM 自动转换。
5. source 使用事件驱动的有界等待；每次 `GetBuffer` 必须完整复制并及时 `ReleaseBuffer`，静音包
   转成全零 PCM，时间戳错误必须终止，数据不连续由 QPC 间隔显式保留。
6. QPC 映射使用 WASAPI 已换算为 100 ns 的 packet timestamp，并通过一次性能计数器校准映射到
   会话时钟。块时间戳和拆块偏移须检查溢出且严格单调。
7. 原生 packet 拆为不超过 20 ms 的 PCM 块，为后续 Opus 帧长预留稳定边界；不能静默丢包或用
   回调到达时间代替采集时间。
8. Pause 必须停止并重置客户端，Resume 丢弃旧缓存后重新开始，Stop 返回不早于最后 PCM frame
   末尾的时间戳；设备失效或默认设备切换导致的流错误本阶段明确终止会话。
9. Windows 原型 CI 必须编译并 lint 该 feature；无声卡的 CI 不实例化真实 endpoint，只验证纯
   endpoint 选择、时间映射、格式、flags、静音、拆块和控制时间下界合同。

## Acceptance Criteria

- 默认 Linux/Windows/macOS 编译图不因新依赖改变产品行为。
- `cargo check --target x86_64-pc-windows-msvc --features recording-windows-audio` 可编译平台 API。
- Windows 原型矩阵使用 `recording-vp9-source-build,recording-windows-audio` 运行严格 clippy。
- 自动化覆盖系统声/麦克风 endpoint 与 flags、QPC 正负偏移、20 ms 拆块、静音包、序号和停止
  时间下界。
- `./scripts/ci-local.sh` 通过；同一提交的 Windows 原生 CI 通过后，才可声明条件编译验证完成。
- 真机 QA 仍需分别录制系统播放与默认麦克风，验证暂停、恢复、设备拔出和 30 分钟 A/V 漂移。

## Out of Scope

- 音源 UI、权限说明、系统声与麦克风同时混音。
- Opus 编码、WebM 音轨、A/V mux epoch 与产品会话接线。
- endpoint 热切换、指定设备选择、独占模式和每应用 loopback。
- macOS ScreenCaptureKit audio 与 Linux PipeWire 音频。

## Platform Notes

- [Microsoft WASAPI loopback recording](https://learn.microsoft.com/windows/win32/coreaudio/loopback-recording)
  要求从 render endpoint 以 shared mode 和 `AUDCLNT_STREAMFLAGS_LOOPBACK` 取得
  `IAudioCaptureClient`；Windows 10 1703 起支持 event-driven loopback。
- [`IAudioCaptureClient::GetBuffer`](https://learn.microsoft.com/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)
  返回的 QPC position 已换算为 100 ns 单位，packet 必须完整释放。
- [`AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM`](https://learn.microsoft.com/windows/win32/coreaudio/audclnt-streamflags-xxx-constants)
  与 `SRC_DEFAULT_QUALITY` 由 Audio Engine 完成采样率和声道矩阵转换。

## Verification

- 纯合同：7 项通过，覆盖 endpoint/loopback、QPC 正负偏移、20 ms 拆块、静音、输入拒绝和控制
  时间下界。
- Linux 默认与显式 `recording-windows-audio` 编译图的严格 Clippy 通过。
- 使用仅含 `windows = 0.62.2` 与本模块的隔离 MSVC target harness 执行
  `cargo clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings` 通过；完整应用的本机
  交叉检查在进入 Clippy 代码前受缺失 `lib.exe` 阻断，不能替代原生 Windows CI。
- `./scripts/ci-local.sh`：25 项通过、0 失败、2 项可选检查跳过；Rust 1075 项通过、14 项忽略，
  前端 73 个文件 1251 项通过，真实 X11 录屏、私有剪贴板、DOM、Canvas、布局与生产构建通过。
- Windows/macOS 条件编译和原生 API 最终结论等待本分支同一 SHA 的远程 CI；Windows 真机采集
  仍属于后续 Native QA。
