# PX-REC-AV-SESSION-01 — 可恢复双轨录屏会话

> 后续 `PX-REC-AV-MERGE-01` 已补齐本文最初列为 Out of Scope 的双轨异常分段恢复；本文其余会话
> 边界保持不变。

## Goal

把已经独立验证的共享时钟、视频/音频采集 worker、A/V epoch、VP9、Opus、双轨 WebM 和 schema v2
接到同一个受 feature 门控的录屏 session。正常停止必须同时产生可播放最终文件与周期恢复分段；错误、
panic 或 owner `Drop` 必须停止两条采集链并保留已经原子提交的双轨前缀。

## Requirements

1. 双轨会话只在显式 `recording-opus-webm` feature 下编译，不改变默认发布和现有纯视频产品入口。
   会话使用一个 `RecordingSessionClock` 创建视频与音频 source；音频 pipeline 原点固定为该时钟的
   `0`，首个有效视频帧继续成为媒体时间 `0`。
2. 编码 worker 必须各自有界地等待视频和音频 pipeline，并按媒体 presentation 顺序消费两轨输入。
   不能按线程到达顺序直接写容器，也不能把任一轨完整缓存到内存。停止与异常必须唤醒所有等待者，
   join 采集、桥接和编码线程，不留下后台线程或未提交 partial。
3. 音频先经 `AvTimelineCoordinator` 丢弃/裁切首视频帧以前的 PCM。进入编码器的音轨从媒体时间
   `0` 开始；起始、packet 间与停止尾部的真实空洞写为零 PCM。时间戳只允许小于一个 48 kHz sample
   的量化误差，更大的倒退或重叠必须失败，不能静默压缩时间。
4. VP9 与 Opus packet 先进入有界双轨 interleaver。只有当另一轨的下一 packet 下界已经越过待写
   timestamp 时才能提交；相同 timestamp 固定视频在前。结束时两轨封尾后再排空剩余 packet。
   `AvWebmPacketMux` 的全局单调检查继续作为最后一道防线。
5. 最终文件使用贯穿会话的一个 VP9 encoder、一个 Opus encoder 和一个双轨 mux。周期分段复用全局
   VP9 packet，并在边界强制 keyframe；每段必须使用独立 Opus encoder、独立双轨 mux 和从 `0`
   开始的本地 timestamp，不能把依赖上一段 decoder state 的 Opus packet直接切开。
6. 分段边界由第一帧达到 `segmentDurationNs` 的视频 presentation 确定。编码 worker 必须在处理该帧
   前把跨边界 PCM 按 48 kHz frame 精确拆分；旧段补齐到边界、封尾、fsync 并用 schema v2 原子提交
   后，才能创建新段并写边界视频帧。最终音频真实 frame 数必须等于所有已提交分段之和。
7. 正常停止分别取得视频和音频 source 的有效时长，`AvTimelineCoordinator` 计算公共 mux 时长；较短
   音轨以最后视频帧或零 PCM 延长到公共时长。会话报告分别保留视频采集/接受/编码/丢帧与音频
   chunk、PCM frame、Opus packet 统计，不能用 packet 数代替 PCM 时长。
8. 暂停与恢复必须同时控制两条 source。第二条控制失败时要尝试回滚第一条；无法恢复一致状态时立即
   中止整个 session。任一 source、pipeline、编码器、mux、journal 或线程错误都必须选择具体根因，
   同步中止另一轨，并把清单留在 `interrupted`。
9. schema v2 音轨参数必须直接取自实际全局 Opus encoder 的 `track_config`。分段与最终提交使用
   `RecordingTrackStats::with_audio`；任何统计、时长或 encoder 配置不一致都不得写成 complete。

## Acceptance Criteria

- 合成视频与连续/分块 PCM 通过真实 VP9 + Opus 生成一个最终双轨 WebM 和至少两个独立可播放恢复
  分段；结构检查确认每个文件各有一条 VP9 与一条 Opus 轨、分段从 keyframe/本地零点开始。
- 首视频帧以前、跨 epoch、跨分段、内部空洞、尾部静音与非 20 ms 结尾均有确定性测试；解码应用
  pre-skip/DiscardPadding 后，PCM frame 数与 manifest 统计完全一致。
- packet interleaver 测试覆盖音频先到、视频先到、相同 timestamp、跨轨乱序风险和封尾排空，队列
  上限有显式断言。
- 正常 stop、暂停/恢复、视频失败、音频失败、编码失败和 owner `Drop` 均回收全部线程；失败会话只
  暴露已校验的连续分段前缀，不能残留 complete manifest 或未受管 partial。
- 默认构建不链接 Opus；`recording-opus-webm` 的 fmt、clippy、定向/完整 Rust 测试、
  `./scripts/ci-local.sh` 与同 SHA 四平台录制原型 CI 通过。

## Out of Scope

- 把双轨开关暴露到录屏 UI，或在默认发布中启用音频；
- macOS ScreenCaptureKit 音频、Linux PipeWire 音频 source、系统声与麦克风混音或设备选择；
- 时钟漂移重采样、自动增益、降噪、回声消除和丢包隐藏；
- 双轨异常分段 remux、波形，以及真机播放器和长时漂移 QA；双轨首帧缩略图由后续
  `PX-REC-AV-THUMBNAIL-01` 单独交付。

## Design Decisions

- 双轨 session 使用两个容量为一的桥接通道把现有视频/音频 pipeline 接到一个编码 owner。桥接线程
  只持有一个未确认事件，因此总内存仍受原 pipeline 与通道容量约束；编码 owner 比较两轨 head，
  不按线程调度顺序决定媒体顺序。
- packet interleaver 只保留两编码器当前 frontier 之间的压缩 packet。VP9 `lag_in_frames = 0`，Opus
  固定 20 ms；若队列超过按两轨 lookahead 推导的固定预算就失败，而不是退化为无界缓存。
- schema v2 的 `pcmFrameCount` 表示实际交给 Opus 的媒体 PCM，包括为保持 A/V 时间线写入的静音，
  不包括 encoder lookahead 或尾部 alignment padding。

## Verification

- `cargo test --features recording-opus-webm --lib recording::av`：15 项通过；覆盖 epoch、跨边界 PCM
  拆分、周期双轨提交、正常 session stop、暂停/恢复、启动失败、单轨中止和 owner Drop。
- `cargo test --features recording-opus-webm --lib av_interleaver`：3 项通过；覆盖音频先到、未来视频
  同 timestamp 时不得抢先、视频优先、同轨严格递增和 32 packet 固定上限。
- `cargo test --features recording-opus-webm --lib`：1130 项通过、0 失败、15 项真实桌面测试忽略；
  loopback HTTP 测试在允许本机监听的环境中执行。
- `cargo clippy --features recording-opus-webm --all-targets -- -D warnings`：通过。
- `./scripts/ci-local.sh`：25 项通过、0 失败、2 项按环境配置跳过；其中默认 Rust 1087 项通过、
  14 项真实桌面测试忽略，前端 73 个文件 / 1252 项测试通过，DOM smoke 14 项、X11 录屏闭环、
  X11 剪贴板隔离、Canvas/Layout 像素 smoke 和生产构建通过。
- 本提交同 SHA 的远程 CI：推送后填写；本地门禁不能替代 Windows/macOS 条件编译和四平台 codec
  原型 job。
