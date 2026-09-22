# PX-REC-AUDIO-MIX-01 — 录屏系统声与麦克风有界混音

日期：2026-09-23

关联需求：`PX-REC-AUDIO-01`、`PX-REC-AUDIO-WORKER-01`、`PX-REC-AV-SESSION-01`

## Goal

让受门控的 Windows、Linux 与 macOS 录屏 QA 构建可以同时录制系统声和默认麦克风，并在进入现有
单一 48 kHz stereo Opus/WebM 音轨前完成确定、可恢复且有界的 PCM 混音。默认构建和正式 release
继续不启用录屏。

## Requirements

1. 新增稳定 wire 模式 `systemAndMicrophone`。只有后端确认当前构建和运行时同时支持系统声与
   麦克风时才返回该模式；前端不能通过伪造模式单独构造第二个音源。
2. 两个原生 source 必须在现有音频 worker 线程内用同一个 `RecordingSessionClock` 创建、控制和
   销毁。平台对象仍可保持 `!Send`；不得为绕过线程亲和把原生对象移到未受控子线程。
3. 混音只接受现有归一化合同：48 kHz、mono/stereo、interleaved finite `f32`、单块不超过
   100 ms。mono 在混音时复制到左右声道，输出固定为 stereo。
4. 每路输入按其 `captured_at_ns` 映射到公共 48 kHz sample grid；序号倒退、同一路重叠、非有限
   sample、格式变化、算术溢出或迟到超过已经提交的输出必须中止会话，不能静默截掉音频。
5. 混音允许最多 100 ms 的 packet 抖动。source 暂无 packet 时，用同一 source 的控制时钟在该
   窗口之后确认静音区间；两路共同 watermark 以前才允许输出。每路 staging 上限一秒，输出以
   最多 20 ms 块提交，保持现有一秒 pipeline 背压上限。
6. 每路使用固定 0.5 线性增益（−6.0206 dB），然后相加并防御性钳制到 `[-1, 1]`。首阶段不做
   自动增益、降噪、回声消除、ducking 或动态 limiter，避免不可测试的音量泵动。
7. Pause 必须暂停两路并清除未提交输入、watermark 和输出；Resume 从新的共同时间段开始。
   任一路控制失败时必须尝试回滚另一条并使会话失败，不能留下单边继续采集。Drop/错误不封尾。
8. 正常 Stop 必须停止两路、取得各自有限尾块、完成混音后才让现有 audio pipeline 封尾；返回的
   控制时间戳不得早于任一路最后样本末尾。任一 source 初始化、采集或停止失败都中止双轨录屏并
   保留已经提交的恢复分段。
9. Windows 使用现有 WASAPI loopback + default capture，Linux 使用现有 PipeWire default sink
   monitor + default source，macOS 13+/15+ 使用两个现有 ScreenCaptureKit audio output source。
   设备选择、热切换与 macOS 14 麦克风后备不在本切片内。
10. 自动化必须覆盖不对齐 packet、单路静音、mono/stereo、固定增益、抖动窗口、迟到/重叠、
    bounded staging、暂停丢弃、停止尾块和任一路错误。四平台原型 CI 必须编译该模式；真机 QA
    增加系统声+麦克风、耳机/扬声器、静音、暂停、设备消失、强杀恢复与 30 分钟漂移。

## Acceptance Criteria

- [x] 同时输入两路同相满幅信号时输出不超过满幅；只有一路有信号时保持固定 −6 dB，不随另一
  路是否静音而改变增益。
- [x] 任意合法 packet 边界和最多 100 ms 到达抖动得到相同 sample 输出；超过已提交 watermark 的
  迟到块、同路重叠与格式变化明确失败。
- [x] 一路长时间无 packet 时另一条在有限延迟后继续输出；两路 staging 与 ready 输出均有固定上限。
- [x] Pause/Resume 不泄漏暂停前样本，正常 Stop 混入两路有限尾块，错误与 Drop 不伪造完成状态。
- [x] 后端能力、IPC 类型和截图覆盖层只在可信能力列表中显示 `systemAndMicrophone`，未知模式保持
  fail closed。
- [ ] 默认、单音源与既有恢复/缩略图行为不变；本地完整门禁及同一 SHA 四平台录制原型 CI 通过。
- [ ] Windows、macOS Intel/Apple Silicon、Linux X11/Wayland 真机分别记录双源权限、听感、暂停、
  设备消失、强杀恢复和至少 30 分钟 A/V 漂移。

## Verification

- 2026-09-23 Linux x86_64：`./scripts/ci-local.sh` 25 项通过、0 失败；非宿主平台交叉 lint 与
  AppImage 可视 smoke 按配置跳过。
- `recording-linux-av-qa`：严格 Clippy 通过；Rust 库测试 1188 项通过、17 项按平台或外部工具配置
  忽略、0 失败。
- 前端音频模式、工具栏与人工 QA 合同由全量 Vitest 覆盖：73 个文件、1273 项通过。
- Windows、macOS Intel/Apple Silicon 与 Ubuntu 录屏原型 CI，以及各平台真机 QA 仍待执行。

## Out of Scope

- 选择非默认设备、保存设备 ID、设备热切换、独立多音轨或后期重新调节两路音量；
- 音量表、mute/solo、用户可调增益、ducking、noise suppression、AEC、AGC 与动态 limiter；
- 摄像头、直播推流、H.264/AAC，以及把录屏或双源混音加入默认/release feature；
- 修改 schema v2 的 Opus 音轨格式。混音仍产出一条现有 48 kHz stereo 音轨；来源元数据在未来
  显式 manifest 版本升级时再加入，不能静默改变当前严格 schema。
