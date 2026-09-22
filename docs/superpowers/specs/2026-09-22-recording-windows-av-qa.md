# PX-REC-WINDOWS-AV-QA-01 — Windows 双轨录屏 QA 接线

## Goal

在不改变默认发布能力的前提下，把已经独立验证的 Windows WGC 视频源、WASAPI 系统声/默认麦克风
音源和可恢复 VP9 + Opus session 接到同一个可信录屏生命周期。显式 QA 构建中的用户可以在冻结
选区工具条选择无音频、系统声或麦克风；后端仍决定物理来源、格式、帧率和编码参数。

## Requirements

1. 新增单一 `recording-windows-av-qa` feature，组合 Windows VP9 源码构建、WASAPI 与 Opus/WebM；
   它不进入默认 feature 或正式 release 构建。
2. 录屏开始能力由后端返回。普通构建及非 Windows 平台只返回 `none`；只有 Windows AV QA 构建
   返回 `none`、`systemAudio` 和 `microphone`。前端不得自行根据操作系统猜测能力。
3. 录屏覆盖层默认选择 `none`，并只显示后端允许的音频模式。开始请求携带枚举值；后端在消费
   冻结截图会话前再次验证，伪造或过期能力不能启动录音。
4. `none` 沿用现有单轨 `DiagnosticRecordingSession`。系统声和麦克风分别使用默认 render
   endpoint loopback 与默认 capture endpoint，并把 WGC 与 WASAPI factory 交给同一个
   `AvRecordingSession`；两者必须收到同一个 `RecordingSessionClock`。
5. 单活动会话注册表同时拥有单轨或双轨 session。暂停、继续、停止、取消、启动回滚、控制窗销毁
   和 `Drop` 对两种 session 保持同一代次与资源回收语义，不能在双轨旁路创建第二个 manager。
6. 停止结果保留现有前端所需的视频摘要；双轨详细统计继续以 schema v2 manifest 为权威来源。
   只有双轨 session 完整核对视频、音频、编码和 journal 统计后才能进入 `complete`。
7. 音频初始化失败、设备丢失、队列满、编码失败或控制分歧必须中断两轨，关闭控制窗并释放
   Recording gate；控制窗以有界、不可重入的健康检查发现 worker 提前退出，并复用唯一 lifecycle
   stop 路径清理。已经原子提交的恢复分段仍可在结果库显示。
8. Windows 录屏原型 CI 使用组合 feature 编译并运行 Rust clippy、VP9/Opus session 测试和 WASAPI
   合同测试；默认、Linux、macOS 与 Wayland 编译图不得因此链接 Windows 音频或 Opus。

## Acceptance Criteria

- 后端能力矩阵测试证明：只有 `target_os = windows` 且启用 `recording-windows-av-qa` 时暴露两种
  有声模式；未知模式由 serde/IPC 拒绝，支持范围不会由前端扩大。
- 前端测试覆盖能力加载、默认无音频、模式切换、开始请求参数和能力加载失败时安全回退。
- manager/lifecycle 的合成源测试覆盖单轨与双轨 start、暂停/继续、正常 stop、取消和失败回收；
  双轨结果的 manifest 为 schema v2，含一条 VP9 和一条 Opus 轨。
- 控制窗测试覆盖健康检查串行化与失败冻结；manager 测试证明视频或音频 worker 提前退出可被观察，
  stop 后同一个 generation 槽回到 Idle。
- Windows 原生检查能编译 `recording-windows-av-qa`；现有完整本地门禁和同一 SHA 远程矩阵通过。
- 真机 QA 记录系统声、麦克风、暂停/继续、设备拔出、控制窗排除以及至少 30 分钟 A/V 漂移之前，
  本需求只标记为 QA 能力，不标记为默认发布可用。

## Out of Scope

- 同时混合系统声与麦克风、设备枚举/指定设备、音量控制、自动增益、降噪、回声消除；
- macOS ScreenCaptureKit 音频和 Linux PipeWire 音频；
- 双轨异常分段 remux、双轨缩略图/波形、剪辑与跨会话合并；
- 在默认 Cargo feature、正式安装包或更新通道中启用录屏或音频。

## Verification

- `cargo clippy --features recording-opus-webm --all-targets -- -D warnings`：通过；
- `cargo test --features recording-opus-webm --lib`：1135 项通过、15 项忽略，覆盖双轨 manager、
  schema v2、VP9/Opus、暂停/继续、正常停止和音频 worker 异常退出；
- `npx vitest run`：73 个文件、1260 项通过，覆盖能力校验、默认无音频、模式切换、健康轮询、IPC
  参数和 Windows Native QA 合同；TypeScript、JS lint、IPC/HTML 边界和 Vite 生产构建通过；
- `./scripts/ci-local.sh`：25 步通过、0 失败、2 跳过；默认 Rust 1090 项通过、14 项忽略，X11
  录屏/剪贴板、DOM、Canvas、布局和生产入口 smoke 通过。跳过项是非宿主交叉 lint 与未请求的
  AppImage 可视 smoke；
- Linux 主机无法用 MSVC `lib.exe` 链接完整 Windows crate，且组合 feature 的固定 libvpx 源构建
  需要 Windows 工具链。Windows 条件编译、WGC/WASAPI 链接与同一 SHA 矩阵由远程原生 CI 判定；
  系统声、麦克风、设备拔出和 30 分钟 A/V 漂移仍须 Windows 10/11 真机记录。
