# PX-REC-01 可恢复录屏技术设计

## 结论

Clippy 现有截图后端不能直接循环调用成录屏。冻结截图优化的是一次性首帧延迟；录屏需要长寿命采集
会话、单调时钟、背压、编码、容器封尾和崩溃恢复。第一阶段只交付单区域、无音频的视频，之后再
增加单一音轨。发布默认入口必须等待采集、编码和恢复三条链在同一平台同时通过；当前只开放显式
feature 下的原生 X11 与 Windows QA 验证入口。

xcap 0.9 提供跨平台 `VideoRecorder`，但官方仍把 video recording 标为 WIP；公开 `Frame` 只有
`width`、`height` 和 RGBA `raw`，不含采集时间戳，示例在接收端自行读取 `Instant`。本仓库
`Cargo.lock` 中候选 xcap 0.9.6 的 Linux X11 实现还会以约 1 ms 间隔向无界 `mpsc` 发送整屏 RGBA；
编码器落后时没有背压，内存会随队列增长。Wayland 实现自行建立 Portal/PipeWire 会话，不能由一次
成功截图推断录制可靠。
上游仍有录制配置、Wayland 性能和停止后重新初始化等公开问题。因此 xcap 可以作为候选帧源，不能
承担 Clippy 的时间线、内存和恢复合同，也不能为了录屏直接替换当前 Linux 截图依赖。

## 当前实现状态（2026-09-21）

第一条产品入口已经接到独立的 Recording 选区覆盖层。Linux X11 与 Windows 现进入受门控的真机
QA 阶段，默认发布能力仍保持关闭：

### Goal

在不扩大普通截图权限、也不对未验收平台作发布可用承诺的前提下，把既有 X11/Windows + VP9 录屏
领域链连接到用户可发现、后端可信的区域选择入口与可安装 QA 包。

### Requirements

- 仅在显式录屏 QA feature 中开放入口：Linux X11 与 Windows 使用 VP9 原型，Linux Wayland 还要求
  `recording-wayland-qa`，macOS 12.3+ 要求 ScreenCaptureKit feature；默认构建和未知会话继续隐藏；
- 覆盖层复用多屏冻结、窗口命中和逻辑到物理 crop，窗口标签使用独立
  `recording-overlay-*` 调用域，只允许读取冻结帧、取消和开始录屏；
- 普通截图会话不能升级为录屏，录屏会话也不能调用复制、标注、扫码、翻译或长截图命令；
- 前端只提交后端签发会话中的逻辑选区。30 fps、包含光标、VP9 编码器、输出目录和录制 session ID
  由后端固定或生成；
- Windows 开始录制前必须应用现有 `WDA_EXCLUDEFROMCAPTURE` 控制窗排除；失败时回滚，不得录入控制窗；
- Linux/Windows 原型仍需完成下文的原生运行与真机验收，不能因代码可编译而记为发布可用。

本地 Linux X11 开发使用 `cargo tauri dev --features recording-vp9-prototype`，Wayland 使用
`cargo tauri dev --features recording-wayland-qa`；Windows Native QA
使用 `recording-vp9-source-build`。手动 QA workflow 只为 Linux X11 与 Windows 生成带入口的原型包，
并在 `QA-BUILD.txt` 记录 feature；正式 release workflow 不启用它们。

### Acceptance Criteria

- 策略测试证明 X11、Wayland、Windows Native 与 macOS 只在各自显式 QA feature 下开放，未知会话
  保持关闭；
- 后端测试证明录屏会话签发 `recording-overlay-*` 标签及 `recording` payload，普通截图会话请求录屏
  返回 `capture_intent_mismatch` 且不释放或转换当前 gate；
- IPC 权限测试证明录屏覆盖层不能提交截图、扫码、翻译、长截图或动作；
- 前端测试证明录屏选区只显示开始/取消，提交精确会话选区，启动失败后恢复可操作状态；
- 默认与 feature 构建均通过 Rust check，feature 构建通过 `clippy --all-targets -D warnings`，仓库完整
  本地门禁通过。

### Out of Scope

- 把录屏 feature 设为发布默认值；
- 把 macOS、Wayland 或 Windows 原型升级为默认发布能力；
- 音频、摄像头、自动进入剪贴板历史、录制参数 UI；
- 用本次代码门禁替代 X11/Windows 真机画质、性能、控制窗排除、文件播放和崩溃恢复验收。

本阶段的验收边界是“X11/Windows 可信 QA 入口已接线且默认关闭”。两平台的画质、性能、控制窗
排除、停止后的文件播放与崩溃恢复仍属于第一阶段剩余验收；通过这些门槛后才能评估把 feature 设为
发布默认值。完整合同见
[`2026-09-21-recording-windows-qa-entry.md`](../superpowers/specs/2026-09-21-recording-windows-qa-entry.md)。

音频第二阶段已完成 48 kHz mono/stereo PCM 时间线、一秒有界队列和线程内采集 worker；Windows
另有显式 QA feature 下的默认扬声器 WASAPI loopback 与默认麦克风 source。双轨协调以首个有效
视频帧为容器零点，在 sample 边界裁切更早的 PCM，并在共同暂停区间后保持固定平移；结束报告保留
真实轨道偏差。ScreenCaptureKit audio、Linux PipeWire 音频节点、Opus、WebM 音轨与产品 session
接线尚未完成，当前入口仍只录视频。详细合同见
[`2026-09-21-recording-audio-contract.md`](../superpowers/specs/2026-09-21-recording-audio-contract.md)、
[`2026-09-22-recording-audio-worker.md`](../superpowers/specs/2026-09-22-recording-audio-worker.md)、
[`2026-09-22-recording-windows-audio.md`](../superpowers/specs/2026-09-22-recording-windows-audio.md) 与
[`2026-09-22-recording-av-epoch.md`](../superpowers/specs/2026-09-22-recording-av-epoch.md)。

`PX-REC-CLOCK-01` 进一步把 X11、Wayland/PipeWire、Windows WGC、macOS AVFoundation 与
ScreenCaptureKit 各自创建的时间原点收回 `DiagnosticRecordingSession`。平台 factory 现在必须
显式接收同一个 `RecordingSessionClock`，帧、暂停、继续和停止都在该会话时间域内加戳；首帧等待
另用连接后的局部计时。现有视频 presentation timeline 仍以首帧归零，避免破坏 VP9 writer；原生
Windows QPC 音频 PTS 校准和双轨共同 epoch 合同已经补齐；macOS/Linux 原生 PTS、session 接线与
实际双轨 mux 仍待后续完成。共享时钟合同见
[`2026-09-22-recording-shared-clock.md`](../superpowers/specs/2026-09-22-recording-shared-clock.md)。

### 2026-09-21 同 SHA CI 证据

提交 `3daa487b4435da783afdd203ff7fe0b8f18fb3dd` 的
[CI Check 35537211962](https://github.com/51hhh/Clippy/actions/runs/35537211962) 已同时通过 Ubuntu
主检查、Windows/macOS 原生 check、clippy 与 tests，以及 Ubuntu x64、Windows x64、macOS ARM 和
macOS Intel 四个录屏编码原型 job。这关闭了跨平台编译、链接、mux 和强杀恢复 fixture 的远程门禁
缺口；它不证明原生帧源、权限提示、光标、混合 DPI、控制窗排除、长时间资源预算或系统播放器兼容，
这些仍须按下文平台矩阵做真机验收。

## PX-REC-01 结果与恢复库切片

### Goal

让正常停止与异常中断后的录屏都有用户可发现的本地出口，同时继续把应用数据目录、恢复清单和真实
文件路径保留在 Rust 信任边界内。录屏停止成功后打开结果库；启动恢复得到的已提交分段也在同一处
列出，不再只写一条日志。

### Requirements

- 结果库只列出清单状态为 `complete` 或 `interrupted` 的会话，活动录制与封尾中的会话不能出现；
- WebView 只取得会话和产物的不透明身份、尺寸、时长、帧数、大小和状态，不取得应用数据目录路径、
  清单路径或可拼接的任意文件名；
- 完整会话以最终输出为主产物；中断会话逐段列出经过清单约束的已提交分段；
- 导出、在文件管理器中定位和删除都由后端重新解析清单与不透明身份，拒绝符号链接、路径穿越、
  大小或 SHA-256 不一致的产物；导出先写目标目录中的私有临时文件，核对后原子替换；
- 删除只接受 `complete` 或 `interrupted` 会话，并且只删除清单声明的普通文件和清单本身。出现未知
  文件、符号链接或活动状态时整次拒绝，不能递归删除一个未经验证的目录；
- 结果库使用独立窗口标签和最小 IPC 清单。正常停止成功后由后端打开结果库，不能依赖已经销毁的
  控制 WebView 消费返回路径；
- 录屏结果不自动进入剪贴板历史。用户主动导出或在文件管理器中定位。

### Acceptance Criteria

- 领域测试覆盖完整会话、中断会话、活动会话排除、损坏清单、符号链接、篡改产物和未知目录项；
- 导出测试证明输出与清单长度及 SHA-256 一致，篡改源不会产生目标文件；
- 删除测试证明有效会话可清理，活动会话、未知文件和符号链接均被拒绝；
- IPC 测试证明结果库只能加载、导出、定位、删除和关闭，不能调用截图、录屏控制、Pin 或设置命令；
- 前端测试覆盖加载、空态、完整结果、中断分段、错误重试与删除确认；
- 默认构建和各平台录屏 QA feature 构建均通过仓库门禁，同一 SHA 的三平台原生 CI 继续作为
  发布门槛。

### Out of Scope

- 把多个独立 WebM 分段按字节拼接成一个文件；异常恢复由独立切片使用经过验证的 packet remux，
  普通字节拼接始终不是有效实现；
- 持久缩略图、剪辑、重编码、云同步与自动加入剪贴板；
- 音频轨、摄像头，以及把任一平台 QA 入口升级为默认发布能力；
- 用结果库的存在替代 X11 真机播放、性能、强杀恢复和文件管理器集成验收。

### `PX-REC-PLAYBACK-01` 后续切片（2026-09-21）

结果库现已增加按需 WebM 播放。播放准备在 blocking worker 中重新解析不透明会话/产物 ID，校验
普通文件、清单长度和 SHA-256，并确认校验前后长度与修改时间没有变化；通过后才签发结果窗专属的
有界租约。`recording-media` 自定义协议只接受 `recordings` WebView 的 `GET`/`HEAD`，单次只响应
一个最大 2 MiB 的 byte range，不把真实路径、整文件或恢复清单送进 JS。租约最多 8 个，切换、关闭、
删除会话和关闭窗口都会撤销；关闭发生在大文件校验期间时，窗口代次会拒绝迟到签发。

前端同一时间只保留一个带原生 controls 的播放器，首个解码帧承担按需缩略预览，不自动播放；WebView
不支持解码时仍保留导出与在文件夹中显示。AVI 诊断产物不显示播放入口。持久缩略图缓存、音频与
其他平台的产品入口仍不在该切片；验收规格见
[`2026-09-21-recording-library-playback.md`](../superpowers/specs/2026-09-21-recording-library-playback.md)。

### `PX-REC-MERGE-01` 异常分段无损恢复（2026-09-21）

结果库现可把 `interrupted + vp9-prototype + webm` 会话恢复成单个完整录屏。前端只提交会话 ID；
Rust 按 manifest 顺序打开已提交分段并重新核对普通文件、长度和 SHA-256，再用受限流式 EBML 读取器
验证单 VP9 轨、尺寸、1 ms 时间基、无 lacing `SimpleBlock`、关键帧起点、固定帧率时间戳和帧数。
每个 VP9 packet 保持原字节，时间戳按分段 `startedAtNs` 平移后交给现有 libwebm muxer 重建 cluster、
seek 和 duration，因此不会解码/重编码，也不会拼接独立 WebM 容器。

恢复输出写入私有 `.recording.webm.partial`，封尾、fsync、长度与 SHA-256 完成后先把 manifest 提交为
`finalizing`，再原子提升文件并写 `complete`。提交前失败会删除临时输出；进程若在提交点附近退出，
启动恢复会清理未提交 partial，或沿用最终输出恢复协议完成提升。一个进程同一时间只允许一个 remux，
失败始终保留原分段和 `interrupted` manifest。成功后结果库刷新为普通最终 WebM，继续复用既有播放、
导出、定位和删除能力；AVI 诊断会话保持逐段导出。完整合同见
[`2026-09-21-recording-recovery-merge.md`](../superpowers/specs/2026-09-21-recording-recovery-merge.md)。

### `PX-REC-THUMBNAIL-01` 持久首帧缩略图（2026-09-21）

结果库现为 `vp9-prototype + webm` 会话提供持久首帧：完整会话解析最终产物，中断会话解析第一个
已提交分段；默认构建与 AVI 只显示占位。前端只提交会话 ID，并由 `IntersectionObserver` 在卡片
接近视口时请求；离开视口会释放 data URL，失败不会覆盖播放、导出、恢复和删除状态。

后端先重新解析 manifest 并核对普通文件、长度与 SHA-256，再复用恢复 remux 的受限 EBML 解析器
验证单 VP9 轨、尺寸、时间基、无 lacing 与首关键帧。libvpx 只解一个 8-bit I420 帧，拒绝额外帧、
尺寸或 stride 越界，并以录制端相同的 BT.709 limited 矩阵转回 RGBA；缩略图最长边 320 px、PNG
上限 512 KiB、解码上限 16777216 像素。冷生成由进程级单槽串行化。

缓存位于应用数据目录的独立私有 `recording-thumbnails/<session-id>/<artifact-sha256>.png`，不修改
恢复 manifest，也不放宽录屏会话目录的删除白名单。缓存命中仍检查普通文件、PNG 和尺寸；产物 SHA
变化只保留新键，恢复合并和删除会话会清理对应缓存。完整合同见
[`2026-09-21-recording-persistent-thumbnails.md`](../superpowers/specs/2026-09-21-recording-persistent-thumbnails.md)。

参考：

- [xcap 官方录屏示例与 WIP 声明](https://docs.rs/crate/xcap/latest/source/examples/)
- [xcap 公开问题列表](https://github.com/nashaofu/xcap/issues)
- [Microsoft `GraphicsCaptureSession.IsCursorCaptureEnabled`](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.iscursorcaptureenabled)
- [Apple `AVCaptureScreenInput`](https://developer.apple.com/documentation/avfoundation/avcapturescreeninput)
- [XDG Desktop Portal ScreenCast 接口](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html)
- [X.Org XFixes 协议](https://gitlab.freedesktop.org/xorg/proto/xorgproto/-/blob/master/fixesproto.txt)

## 产品流程

1. 用户从独立录屏入口进入区域选择，选择过程复用截图覆盖层的显示器、窗口命中和物理 crop 规则；
2. 开始后冻结覆盖层退出，只保留输入透明边框与小型控制窗；边框和控制窗不进入录制帧；
3. 控制窗显示时长、暂停、继续和停止；异常只停止当前会话，不丢弃已经提交的分段；
4. 正常停止生成一个可播放文件；崩溃恢复页列出可恢复的已提交分段，用户决定合并、导出或删除；
5. 录屏文件不自动进入剪贴板历史，用户主动复制文件或打开目录。

录屏使用独立 `Recording` 模式所有权，不能与普通截图或长截图并行占用桌面采集资源。选区身份由
后端冻结会话转换，录制命令不接受前端提交的任意显示器句柄或物理坐标。

## 分层结构

```text
recording/
  manager.rs       单活动会话、代次 token、暂停/停止/取消状态机
  manifest.rs      原子 journal、分段哈希、启动恢复
  timeline.rs      单调时间戳、帧丢弃策略、暂停区间
  frame.rs         有界 RGBA 帧合同与固定物理 crop
  encoder_worker.rs 阻塞消费有界队列、错误联动与线程回收
  session.rs       journal、临时分段、采集与编码的单一资源 owner
  thumbnail.rs     首关键帧单帧解码、私有持久缓存与删除失效
  mux/             编码帧与可恢复容器，不读取桌面
  platform/
    linux/         X11 帧源；Wayland Portal/PipeWire 帧源
    windows/       WGC/DXGI 帧源
    macos/         ScreenCaptureKit/AVFoundation 帧源
```

平台帧源只产出 `CapturedFrame { sequence, captured_at_ns, width, height, stride, rgba }`。时间戳在帧进入
Clippy 边界时读取单调时钟，不能使用系统墙钟或接收端编码完成时间。帧队列固定为 3 个槽位；编码器
落后时丢弃中间帧并记录 dropped count，保留最新画面和真实持续时间。任何实现都不得使用无界队列。
帧源进入队列前必须归一化为紧凑 RGBA 行布局；单帧最多 64 MiB，队列最多持有 192 MiB RGBA。
队列使用定长像素所有权，不能用大 capacity 的 `Vec` 绕过字节预算。

编码与封装只接收已经裁好的固定尺寸帧。首帧确定像素尺寸，后续显示器缩放、旋转、热插拔或像素
格式变化会终止当前分段并给出明确错误，不能静默拉伸。光标是否录入是帧源能力，第一阶段固定为
包含光标；以后改变为显式设置。

## Journal 与恢复协议

每个会话位于应用数据目录的私有子目录：

```text
recordings/<session-id>/
  manifest.json
  .segment-000000.<container>.partial
  segment-000000.<container>
  segment-000001.<container>
```

`manifest.json` 使用版本化 schema，至少记录 session ID、单调时间基、选区/像素尺寸、帧率目标、
编码器身份、容器、创建时间、状态、丢帧数和已提交 segment。每个 segment 记录序号、起始/持续
时间、帧数、字节数与 SHA-256。

提交顺序固定为：完成并 `sync_all` 临时分段 → 计算长度与 SHA-256 → 用私有临时文件原子提交
manifest → 原子提升为最终分段并同步目录。manifest 是提交点；进程若在提交后、提升前退出，启动
恢复只会在 `.partial` 的长度与 SHA-256 和 manifest 完全一致时完成提升。未进入 manifest 的临时
分段不会被当作有效数据，并在启动恢复时删除。随后按连续序号、预算、文件大小和 SHA 验证已提交
前缀；坏尾段只截断恢复点，不使此前分段失效。启动扫描限制会话数、manifest 大小、分段数和总
字节，拒绝符号链接、路径分隔符与未知 schema。

正常停止先完成最后分段，再生成单一最终文件。最终合并失败时保留已提交分段和 manifest，允许重试，
不能删除唯一可恢复数据。异常中断的 VP9/WebM 会话也可在结果库显式发起同一提交协议的 packet
remux；只有最终文件通过结构、时长、帧数、长度与哈希检查后，才能把会话标成 complete。

最终文件现已具备与分段相同的持久化提交合同：写入私有 `.recording.<container>.partial`，核对它的
时长/帧数与已提交分段总和一致，`sync_all` 后记录长度和 SHA-256，再以 manifest 作为提交点原子提升
为 `recording.<container>`。启动若遇到 manifest 已提交但 rename 未完成，会校验并完成提升后写
complete；最终文件损坏时会删除坏输出、保留连续有效分段并写 interrupted。VP9 会话现用同一个
低延迟 libvpx 编码器把每个压缩 packet 同时写入连续最终 mux 与当前恢复分段 mux；正常停止会提交
`recording.webm`，不会解码后重编码，也不会拼接独立 WebM 字节流。

## 编码与容器门槛

第一阶段不预设 H.264。候选实现必须用相同 1080p/4K、15/30 fps、静态文本、滚动页面和高运动
语料比较：

- 60 秒编码的 CPU、峰值 RSS、掉帧、文件大小和封尾时间；
- 小字号文字与透明 UI 边缘的 SSIM/PSNR 和可读性；
- 依赖许可、安装包增量、Ubuntu 22.04 基线及 Windows/macOS 构建；
- 进程强杀后已提交分段的 `ffprobe`/系统播放器可读性；
- 暂停/继续后的真实时长与帧时间线。

候选顺序为：可分段的 WebM/Matroska 软件编码、三平台原生 H.264 适配、MJPEG/AVI 诊断后备。
MJPEG 实现简单但文件大、文字边缘有损；它可以验证 journal 和时间线，不能在没有质量数据时成为
默认交付。MP4 若使用单一尾部 `moov`，强杀后不可恢复；只有 fragmented MP4 或独立可播放分段
满足本需求。

当前仓库已经加入流式 MJPEG/AVI 1.0 诊断分段：它按单调呈现时间补帧，写入 `idx1`，每段限制为
18,000 帧和 4 GiB，并由可用时的 `ffprobe` 回归验证 codec、尺寸、帧率、帧数与时长。该实现只用于
把 journal、暂停/丢帧时间线和独立分段播放跑通；AVI 1.0 上限与 JPEG 有损画质使它仍不能成为默认。
journal 已能创建私有会话、创建独占 `.partial`、提交分段元数据、提升最终文件、写入累计丢帧并以
`recording → finalizing → complete` 原子更新状态。诊断会话 owner 已把 journal、临时分段、三槽
pipeline、采集线程和编码线程接成一个生命周期：正常 Stop 只有 owner 能提交和完成，任一线程错误
或 `Drop` 会 join 两条线程、删除未提交 partial 并记录 interrupted；联动产生的 `Pipeline::Aborted`
不会遮住原始采集或编码错误。单活动注册表也已实现 `Starting → Recording → Stopping → Idle`，启动中
和停止中都占槽，并以不可复用的 generation token 拒绝迟到的暂停、停止与取消；截图桌面资源交接
状态机已经固定；Tauri 桌面恢复适配器、控制窗口宿主和受 caller 限制的暂停/继续/停止/取消 IPC
现已接入。可信开始入口已在原生 X11/Windows、macOS ScreenCaptureKit 与 Wayland Portal 的各自显式
QA feature 中开放；默认构建继续关闭。

### 编码器 A/B 工具与当前结论

仓库提供可选工程基准 `scripts/benchmark-recording-encoders.mjs`。它用仓库内 Noto CJK 字体生成包含
中英日、数字、货币与数学符号、小字号滚动文字、网格和运动区域的确定性样本，以 FFV1 作为无损
参考，并对每个候选记录墙钟时间、FFmpeg CPU/RSS、文件大小、SSIM、PSNR 以及 `ffprobe` 解码结果。
产物写入显式目录或 `/tmp`，不进入仓库，也不在日常 CI 中执行：

```bash
node scripts/benchmark-recording-encoders.mjs \
  --duration 60 --width 1920 --height 1080 --fps 30 \
  --candidate mjpeg-avi --candidate vp9-webm \
  --output /tmp/clippy-recording-codecs-1080p
```

`--candidate` 可重复使用；省略时仍运行全部候选。未知 ID 会在生成无损参考前拒绝，避免 60 秒/4K
基准意外带上已经确定不实时的候选。

2026-09-20 在 Intel Core Ultra 5 125H、FFmpeg 8.0.1 上先运行 1280×720、30 fps、2 秒的工具自检，
得到以下本地证据：

| 候选 | 墙钟 | 峰值 RSS | 文件大小 | SSIM | PSNR |
|---|---:|---:|---:|---:|---:|
| MJPEG / AVI | 0.126 s | 230.9 MiB | 3.52 MiB | 0.991269 | 34.7485 dB |
| libvpx VP9 / WebM | 0.331 s | 351.1 MiB | 0.14 MiB | 0.993904 | 49.7496 dB |
| rav1e AV1 / Matroska | 4.341 s | 406.0 MiB | 0.17 MiB | 0.993561 | 49.2045 dB |

这次短样本只验证基准工具和候选方向。VP9 在本机样本上同时显著缩小文件并改善客观画质，作为下一
个嵌入原型；只有 libvpx 与 WebM 封装能在同一 SHA 的 Linux、Windows、macOS CI 构建，实际嵌入式
writer + 真实帧源的 60 秒 1080p/4K 分层语料满足实时、内存和掉帧预算，且周期分段经过强杀恢复，
才可确定为产品默认。
rav1e 当前参数明显未达到实时，本机也没有 NASM，尚不能验证仓库从源码构建时的 x86 汇编优化链，
所以保留为后续可选项。这里不把系统 FFmpeg 已链接的 rav1e 二进制性能归因于本机是否安装 NASM。

同一主机随后完成 60 秒长样本；每个输出均由 `ffprobe` 解码为 1,800 帧、30 fps、60 秒：

| 档位 / 候选 | 墙钟 | 相对素材时长 | 峰值 RSS | 文件大小 | SSIM | PSNR |
|---|---:|---:|---:|---:|---:|---:|
| 1080p MJPEG / AVI | 3.062 s | 5.1% | 423.3 MiB | 187.02 MiB | 0.993479 | 36.3662 dB |
| 1080p VP9 / WebM | 8.378 s | 14.0% | 543.9 MiB | 4.73 MiB | 0.994484 | 50.8951 dB |
| 4K MJPEG / AVI | 9.844 s | 16.4% | 1,459.2 MiB | 662.51 MiB | 0.994681 | 36.6361 dB |
| 4K VP9 / WebM | 25.705 s | 42.8% | 1,557.9 MiB | 10.75 MiB | 0.995103 | 51.7475 dB |

这组数据证明系统 FFmpeg 的相同 VP9 参数在该机器上有 1080p/4K 实时吞吐余量，且没有缺帧；它也
暴露了 4K 编码进程约 1.56 GiB 的峰值工作集。基准进程、FFmpeg 构建和嵌入式 canary 绑定不是同一
二进制，故这些数字只作为参数与预算方向：启用产品默认前仍须用实际嵌入式 writer + 真实帧源记录
CPU/RSS/背压丢帧、停止封尾和安装包增量，并取得四目标同 SHA CI。

仓库另提供 feature-gated 的 `recording_vp9_benchmark` 工程二进制。它直接调用生产
`Vp9WebmWriter`，生成有静态渐变、网格和移动方块的确定性 RGBA 帧，完整经过
RGBA→I420→libvpx→WebM，不经过 FFmpeg，也不复制 writer。输出使用 `create_new`，单帧沿用产品
64 MiB 上限，最长 18,000 帧；因此可重复测实际嵌入路径且不会覆盖已有文件。优化构建和运行方式：

```bash
cd src-tauri
cargo build --profile bench --bin recording_vp9_benchmark \
  --features recording-vp9-source-build
/usr/bin/time -v target/release/recording_vp9_benchmark \
  --duration 60 --width 1920 --height 1080 --fps 30 \
  --output /tmp/clippy-embedded-vp9-1080p-60s.webm
```

2026-09-20 在同一 Intel Core Ultra 5 125H 上，以固定源码归档构建的实际嵌入 writer 得到：

| 档位 | writer 用时 | 相对素材时长 | 吞吐 | 峰值 RSS | 文件大小 | `ffprobe` |
|---|---:|---:|---:|---:|---:|---|
| 1080p / 30 fps（标量） | 19.496 s | 32.5% | 92.3 fps | 214.1 MiB | 3.51 MiB | VP9，1,800 帧，60.000 s |
| 4K / 30 fps（标量） | 74.263 s | 123.8% | 24.2 fps | 723.1 MiB | 12.70 MiB | VP9，1,800 帧，60.000 s |

分项计时确认 4K 标量链的 75.695 秒中，RGBA→I420 占 46.658 秒（61.6%），libvpx+WebM 占
28.968 秒（38.3%）。原实现逐像素跑两遍并为每帧重新分配三个 plane，因此先优化色彩转换，没有
通过增大 `cpu-used` 降低文字质量。原型现精确固定纯 Rust
[`yuv 0.8.19`](https://github.com/awxkee/yuvutils-rs)，用 Professional 精度、运行时 SSE4.1/AVX2/ARM
RDM 分派执行 BT.709 limited-range，并复用上一帧的 I420 plane。64×48 全色域确定性样本的 Y/U/V
相对旧标量实现最大偏差均不超过一个 8-bit 级别；黑、白、红、绿端点另有固定断言。
该 crate 已单独通过 `x86_64-pc-windows-msvc`、`x86_64-apple-darwin` 和
`aarch64-apple-darwin` 编译；完整应用的原生链接与运行仍由四目标远程 job 判定。

相同优化构建和输入得到：

| 档位 | writer 用时 | RGBA→I420 | libvpx+WebM | 吞吐 | 峰值 RSS | 文件大小 |
|---|---:|---:|---:|---:|---:|---:|
| 1080p / 30 fps（SIMD） | 10.108 s | 2.092 s | 7.994 s | 178.1 fps | 213.6 MiB | 3.51 MiB |
| 4K / 30 fps（SIMD） | 39.023–47.637 s | 8.059–9.870 s | 30.887–37.678 s | 37.8–46.1 fps | 723.4–723.5 MiB | 12.70 MiB |

最终固定 `yuv 0.8.19` 后重复跑 4K，主机负载使 libvpx 阶段出现明显调度波动，因此表中保留两次
实测范围，并以较慢一次给出结论：相对 75.695 秒分项标量基线，总耗时至少降低 37.1%，色彩转换
至少提速 4.73×。两档仍由 `ffprobe` 核对为 VP9、1,800 帧、30 fps、60.000 秒；两次 4K 文件大小
相同且解码结果 SSIM 为 1.000000，说明耗时差来自调度而非编码内容；WebM 元数据不同，文件本身不
是二进制一致。新旧转换器的 4K 解码结果 SSIM 为 0.996961，这只是防止输出发生大幅漂移的对照，
不替代对原始 RGBA 的质量评分。合成链现在通过 4K 实时吞吐预算，但约 724 MiB 峰值和真实桌面
采集、三槽背压、控制窗排除仍未验收，所以 4K 与产品 UI 继续保持不可用；下一步转入真实 X11
帧源端到端长样本。

仓库已为这一步加入 feature-gated 的 `recording_x11_vp9_benchmark`。它要求原生 Linux X11，直接
串起 RandR 物理裁剪、生产 `X11RegionFrameSource`、限帧采集 worker、三槽 pipeline、VP9 周期恢复
分段和连续 `recording.webm`；Wayland/Xwayland 会被明确拒绝，避免把 Xwayland 根窗口数据登记成
原生 X11 证据。输出目录使用 create-only 和私有权限，已存在的目录不会被覆盖。真机运行方式：

```bash
cd src-tauri
cargo build --profile bench --bin recording_x11_vp9_benchmark \
  --features recording-vp9-source-build
/usr/bin/time -v target/release/recording_x11_vp9_benchmark \
  --duration 60 --width 1920 --height 1080 --fps 30 \
  --output-dir /tmp/clippy-x11-vp9-1080p-60s
```

可用 `--monitor-id` 选择 RandR output，并以 `--crop-left/--crop-top` 指定显示器内物理裁剪。JSON 同时
记录请求采样数、按实际停止时钟补齐的输出帧槽、实际采集帧、采集短缺、三槽背压丢帧、编码器
输入/输出、停止封尾耗时、进程峰值 RSS、分段数和最终文件大小。
`capturedFrames < requestedCaptureFrames` 表示帧源或调度未跟上；
`droppedByBackpressure > 0` 表示编码消费者未跟上，二者不能合并为一个“掉帧”数字。Linux 原型 CI
会在隔离 Xvfb 中跑 1 秒完整链，只验证 X11 协议、队列、分段和最终提交。当前开发会话是 Wayland，
所以尚未生成 60 秒原生 X11 真机性能结果；路线状态继续保持未验收。

OpenH264 的源码本身可嵌入构建，但自行编译的库与 Cisco 分发的预编译二进制不具有同一分发条件；
在许可、专利和安装包策略独立审查前，不把它作为第一阶段默认依赖。MJPEG 继续只承担诊断闭环。

仓库现以非默认 `recording-vp9-prototype` feature 加入第一条嵌入式 VP9/WebM 探针。它把 RGBA 按
BT.709 limited-range 转成 I420，以固定帧率补齐背压造成的时间空洞，每两秒强制关键帧，并用
libwebm `File` 模式写入 seek 信息和显式分段时长；本地回归由 `ffprobe` 核对 VP9、帧数和 200 ms
时长。纯 Rust `ebml-webm 0.2.1` 探针能解码 30 帧，但没有写 `Duration`/`DefaultDuration`，因此没有
进入实现。

该原型仍不具备默认依赖资格：`shiguredo_libvpx 2026.2.0-canary.1` 的上游 build script 会从 GitHub
下载预编译 libvpx，并从同一 Release 动态取得校验文件。Clippy 已 vendor 对应上游提交，只保留归档
下载，并把 Ubuntu 22/24/26 x64、Windows x64 与 macOS arm64 的 SHA-256 固定在仓库；内容漂移或
未知 target 会立即失败。`shiguredo_libvpx`、libvpx、`webm`/`webm-sys`、libwebm 与 `yuv` 的许可证
和 NOTICE 也已进入安装资源。CI 已配置 Ubuntu 22 x64、Windows x64、macOS arm64 的独立 feature
Clippy 与 mux/session 测试，并为没有上游预编译归档的 macOS x86_64 增加固定 SHA-256 源码归档
构建。四目标远程同 SHA 结果已经取得；归档下载仍需网络，绑定仍是 canary，原生运行矩阵也尚未
完成，因此不能启用默认 feature。2026-09-20 在 Linux x86_64 以隔离 NASM 和 Rust `llvm-tools`
实际冷构建同一源码
feature，用时 11 分 01 秒；4 个 VP9 mux/`ffprobe` 测试通过。这只验证本机的固定源码输入、编译、
符号重写、链接与运行链；macOS Intel runner 已由上述同 SHA CI 单独覆盖。

编码消费线程也已从 MJPEG 具体类型收敛为 `RecordingSegmentWriter` 合同：writer 只能接收时间线已经
归一化的 RGBA 帧并返回同一个最终时长下的 writer 与帧数；pipeline 排空、原始错误优先级、异常中止
和 join 只实现一次。MJPEG 会话继续使用原类型别名，默认行为不变；VP9 feature 的线程回归已经证明
同一三槽 pipeline 可封尾 200 ms WebM。会话配置现以类型化枚举选择编码器，非默认 feature 下的
VP9 已完整经过 capture worker、三槽 pipeline、WebM 封尾、私有分段原子提交和 complete manifest；
清单中的 encoder/container、分段扩展名、时长与帧数均由同一选择产生。生命周期启动合同现由后端
传入类型化编码器策略，默认测试继续使用 MJPEG 诊断 writer，feature 回归则证明同一生命周期能够
选择 VP9 并提交最终 WebM；该选择不从 IPC 接受前端字符串或参数。平台开始适配器已接入领域生命周期，
可信 Tauri 开始 IPC 也已接到显式 VP9 feature 的原生 X11 与 Windows QA 入口；默认策略仍未开放，
因此这不代表 VP9 已成为默认。编码线程现在默认每 60 秒、
最多允许 120 秒一个周期分段；跨过边界即原子
提交，最后一段则在 Stop 后由会话 owner 核对时长与背压再把清单标为 complete。MJPEG 子进程强杀 fixture 已
证明首段提交、次段仍打开时退出，启动恢复会保留可播放首段、删除未提交尾段并写 interrupted；同一
分段 writer 的 VP9 回归已生成两个独立 WebM；原型 CI 也会在四目标分别强制终止独立子进程，验证
已提交 WebM 前缀与未提交尾段的恢复合同。VP9 实现内部已经把固定帧率/RGBA→I420/libvpx 编码与
WebM packet mux 拆开，并显式设置零 lookahead：边界先排空上一帧、提交独立可播放分段，再强制下一
packet 为关键帧；同一 packet 同时进入连续最终 mux。`ffprobe` 会分别核对各段与 `recording.webm`
的 VP9 codec、帧数和时长。X11 受门控入口与四目标编码 CI 已完成；真实平台采集、资源、画质和交互
验收尚未完成，不能据此勾选第一阶段整体验收。

## 平台顺序

1. 先实现与平台无关的 manifest、时间线、有界队列和合成帧 fixture；
2. Linux X11 用长寿命帧源验证区域 crop、背压和控制窗排除；
3. Windows 使用 WGC/DXGI，macOS 使用系统屏幕录制权限与长寿命帧源；
4. Wayland 使用用户授权的 ScreenCast Portal 会话和其返回的 PipeWire remote FD/stream node，
   不复用一次性 Screenshot Portal，也不借 XWayland 截取原生窗口；
5. 三平台视频闭环完成后再引入单一音轨，先分别建立系统音频/麦克风能力矩阵，再做 A/V 同步。

## 控制窗排除

控制窗必须在开始采集前得到可验证的排除策略，不能仅设置 always-on-top 后假设不会进入视频：

- Windows 对 Clippy 自有顶层窗口使用
  [`SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity)，
  并把 API 失败当作启动失败；该能力要求 DWM，Windows 10 2004 前只能退化为 `WDA_MONITOR`；
- macOS 的 ScreenCaptureKit 使用
  [`SCContentFilter(display:excludingWindows:)`](https://developer.apple.com/documentation/screencapturekit/sccontentfilter/init%28display%3Aexcludingwindows%3A%29)
  或排除 Clippy 应用，控制窗可以位于选区内；
- Wayland 的
  [ScreenCast Portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html)
  只定义源类型、光标模式和 restore token，没有调用方指定任意排除窗口的参数。因此这是从官方接口
  推导出的限制：控制窗必须位于流区域外，否则只使用托盘与快捷键；
- X11 当前通过根窗口 `GetImage` 取帧，协议请求没有排除窗口列表。控制窗必须完整位于物理选区外；
  若选区覆盖全部显示器，隐藏控制窗并使用托盘/快捷键，不能逐帧隐藏窗口造成闪烁与采样竞态。

仓库已加入物理坐标规划器：原生排除平台优先把控制窗放在选区底部；仅几何排除的平台会在所有
显示器的上、下、左、右与角落候选中选择离选区最近且含 margin 的安全位置，支持负坐标多屏；没有
安全矩形时明确返回 `TrayAndShortcutsOnly`。控制窗 Tauri 宿主现按实际物理窗口尺寸再次规划并隐藏
建窗，页面 ready 与 token bind 都满足后才显示。Windows 建窗后必须成功设置
`WDA_EXCLUDEFROMCAPTURE`；只有 Windows 10 2004（build 19041）及以上才使用原生排除布局，旧版本
继续要求控制窗完整位于选区外，避免 `WDA_MONITOR` 退化在视频中留下黑块。Linux Wayland、没有
选区外安全位置和未来需要原生排除的路径会明确拒绝启动；macOS 排除调用与托盘/快捷键后备仍待实现。

Linux X11 已加入持久 x11rb 连接的根窗口区域帧源：每次只请求选区物理矩形，依据服务器 visual mask
和字节序转为紧凑 RGBA，并在进入 Clippy 边界时写入单调时间戳。硬件光标通过 XFixes
`GetCursorImage` 取得；其预乘 ARGB 像素按 hotspot 与选区求交后合成，异常尺寸或数据长度会使当前帧
明确失败。它没有采用 xcap 公开的已缩放 `Monitor::x/y`，也没有整屏捕获后裁切。

冻结会话到 RandR 物理矩形的可信 handoff 也已固定：后端核验 caller、会话 identity、显示器和逻辑
选区，再生成不能由 IPC 反序列化的物理 crop；X11 以冻结帧的 monitor output ID 重新查询 RandR
物理原点和尺寸，显示器缺失、重复映射或几何变化都会在消费 Ordinary 会话前失败。只有平台帧源建立
成功，模式 gate 才会原子转换为 Recording。

持续采集 worker 已建立独立线程合同：帧率只允许 1–120 fps，取帧耗时会从当前间隔扣除，慢帧不会
触发追赶式突发采集；停止通过有界控制通道立即唤醒，线程错误和 pipeline 错误会交给 owner，
`Drop` 也会停止并 join，禁止会话结束后留下桌面读取线程。worker 只写三槽 pipeline，不接触编码、
文件或 UI。
暂停/继续命令也由 worker 串行执行，时间戳来自同一平台帧源的单调时钟，暂停期间完全停止桌面取帧；
重复命令保留当前状态并返回明确错误。pipeline 已增加正常封尾与异常中止终态：正常路径携带扣除
暂停区间后的最终呈现时长，异常路径保留已经接受的帧；阻塞消费者会先排空三槽队列，再收到终态，
且生产、暂停、继续与重复封尾均不能越过终态继续修改会话。正常 Stop 现在由采集线程从帧源读取
同一时钟域的终点并封尾，暂停中停止也不会把开放暂停区间计入时长；帧源、控制时钟、pipeline、
控制通道错误及 `Drop` 回收都会中止 pipeline。独立诊断编码线程已闭合 capture → 三槽 pipeline →
MJPEG/AVI：它阻塞等待帧，先排空已接受前缀，再使用同一最终时长封尾；编码失败或线程回收会反向
中止 pipeline，停止仍在运行的采集线程。桌面资源恢复领域状态机、Tauri 控制窗宿主和控制 IPC 已
接入；可信开始 IPC 现已在 X11/Windows、macOS 12.3+ 与 Wayland 的显式 QA 构建接通。各平台真机
性能、权限和回收证据仍未闭合，因此默认构建仍不能从 UI 开始录屏。

Windows 平台已加入第一条 WGC 区域帧源骨架：冻结截图的显示器 ID 和物理像素尺寸会再次与当前
`xcap` 显示器核对，WGC 整屏帧进入零容量通道后由 Clippy 单槽桥接只保留最新一帧，再按可信 crop
无缩放复制为紧凑 RGBA。整屏源超过 64 MiB、显示器几何变化、帧长度不符、首帧五秒未到和通道
关闭都会明确失败；静态桌面暂时没有新帧时每 50 ms 返回控制循环，由既有时间线补齐显示持续时间。
暂停/继续/停止现在也经过平台 hook，暂停会关闭 WGC runtime，继续会丢弃暂停前缓存帧。选择 WGC
而不是 xcap 默认 DXGI recorder，是因为后者的内部采集线程没有终止出口，不满足 Stop/Drop 回收
合同。仓库现固定 xcap 0.9.6 的完整发布源码，只把 WGC 录制会话的
`SetIsCursorCaptureEnabled(false)` 改为 `true`；来源、原件哈希、Apache-2.0 许可证、Cargo path
解析和唯一行为差异由脚本门禁。光标属性从 Windows 10 2004 才提供，上游保留 best-effort 语义；
模块已通过同一 SHA 的 Windows 原生 check、clippy 与 tests，仍需移动光标像素真机测试；完成前
不能把第一阶段“包含光标”记为通过。

macOS 平台已加入 AVFoundation 区域帧源骨架：准备阶段以 CoreGraphics display ID 重新取得实际
backing-pixel 尺寸并核对冻结几何，再把覆盖层的左上角物理 crop 转换为
`AVCaptureScreenInput.cropRect` 所需的左下角屏幕点，并用 `scaleFactor` 直接产出选区 backing
pixels。它不会先分配 6K/8K 整屏 RGBA；因此 64 MiB 预算只约束目标区域。首帧和后续帧仍必须与
冻结选区尺寸和紧凑 RGBA 长度完全一致，任何变化都终止会话，不做拉伸。Retina 的半点边界会原样
传入，横纵倍率不一致会在启动前拒绝。

xcap 的 macOS delegate 使用零容量同步通道。Clippy 在采集 worker 内创建非 `Send` 的
`AVCaptureSession`，同时用独立桥接线程持续接收回调并只保留最新一帧，避免停止会话时 delegate
阻塞在发送上。暂停、继续和停止均调用同一 recorder hook，继续前丢弃旧缓存帧。仓库只为 xcap
增加了受边界检查的 macOS 区域录制入口，版本、原件哈希、七个补丁文件、许可证和调用形态均由
脚本固定。同一 SHA 的 macOS 原生 check、clippy 与 tests 已通过；屏幕录制权限、光标、Retina、
旋转屏、负坐标混合 DPI 和 4K/6K 仍须设备验证。当前 AVFoundation 路径也
不能排除控制窗：Apple 已把
[`NSWindow.SharingType.none`](https://developer.apple.com/documentation/appkit/nswindow/sharingtype-swift.enum)
标为系统不再使用的旧常量，不能把 Tauri 的 content protection 当成录屏过滤证据；若控制窗必须
位于选区内，仍须迁移到提供
[`init(display:excludingWindows:)`](https://developer.apple.com/documentation/screencapturekit/sccontentfilter/init%28display%3Aexcludingwindows%3A%29)
的 ScreenCaptureKit `SCContentFilter`。因此该骨架不开放产品入口，也不把 macOS 录制记为通过。

Wayland 已建立 ScreenCast Portal + PipeWire 区域帧源骨架。准备阶段先按冻结帧的稳定 output ID
重新枚举原生 Wayland 输出，并核对旋转后的物理像素尺寸；Portal 只请求单个 Monitor 源和 Embedded
光标，禁止多流与 restore token。用户在系统选择器授权后，多屏会话必须由 Portal 返回的逻辑位置和
尺寸精确证明是冻结选区所在显示器；只有单屏时才允许 position/size 都缺失，并继续由 PipeWire
协商尺寸做第二次强校验。选错屏、返回多流、源类型不符、显示器热插拔或缩放变化均明确失败，不做
静默拉伸。

Portal 提供整屏 stream，Clippy 只在验证完整帧恰好等于冻结显示器后按可信 crop 裁切，因此整块
显示器 RGBA 必须落在 64 MiB 单帧预算内。截图和录屏现共用一份 PipeWire 原始视频解析器：只接受
八种 32-bit RGB 内存排列、正 stride、有效 offset 和共享内存/MemFd，显式拒绝 DMA-BUF；格式协商
阶段在分配像素前先核对物理宽高。专用 PipeWire main loop 线程通过单槽桥只保留最新帧，暂停/继续
调用 `pw_stream_set_active`，停止断开 stream、退出 loop 并关闭 Portal session。初始化与控制均有
五秒上限；初始化卡在第三方库时调用线程按时返回，Portal 随后关闭，迟到线程不会进入产品会话。

`PX-REC-WAYLAND-QA-01` 在此基础上加入非默认 QA 入口。Rust 创建短生命周期授权窗，并从该窗口的
Wayland surface/display 导出 xdg-foreign parent；WebView 不接触 parent、Portal token 或 node。授权
等待由 registry 内的取消令牌控制，Cancel 或窗口销毁会让等待中的 Portal session 关闭。系统返回
唯一显示器流并通过身份复核后，采集线程先隐藏授权窗，再打开 PipeWire remote，因此首帧不依赖
合成器排除 Clippy 窗口。

录制阶段没有可见浮动控制窗，原生托盘从 lifecycle 当前 Active slot 取得 exact generation，提供
Pause、Resume 与 Stop；全屏选区也不要求找到选区外的窗口位置。GNOME、KDE 与 wlroots 上的授权
允许/拒绝/取消、单/多屏元数据、分数缩放、旋转屏、静态帧、4K 带宽、光标、托盘控制和 Stop/Drop
回收仍需要真机证据。完成这些以前，该入口只存在于 `recording-wayland-qa` 包，默认/release 构建
继续关闭。

四平台现在统一为 `PlatformFrameSourcePlan → PlatformFrameSource`：计划阶段仍在 Ordinary 截图会话
内按平台重新枚举显示器，只保存可跨线程移动的选区、来源描述和原生参数；生命周期消费截图并恢复
桌面后，采集 worker 才创建 X11 connection、WGC/AVFoundation recorder 或 Portal/PipeWire session。
X11 与 Windows 在连接时再次计算来源描述，macOS 与 Wayland 再次生成计划；任何 handoff 期间的
显示器 ID、几何、缩放或区域变化都会在首帧前失败并回滚控制面、会话和 Recording gate。Linux
Unknown/Native 会话不会猜成 X11。X11 隔离闭环已经覆盖两阶段计划；Windows/macOS 的相同代码图
分别通过隔离交叉 lint，真实平台结果仍由同一 SHA Native Check 和真机 QA 判定。

`ci-local.sh` 现会在隔离 Xvfb 中显式运行原生闭环：RandR 可信区域 → 持久 X11 帧源 → 采集线程 →
三槽 pipeline → MJPEG/AVI → 私有分段与 complete manifest，并核对清单帧数和文件权限。它验证真实
X11 协议与落盘接线，不代表真实桌面合成器、4K 带宽、光标移动或控制窗排除的性能验收。
X11 帧源在原生 RandR 核验完成后还会产出唯一的物理来源描述；录制清单与后续控制窗规划均应消费
这份 `source_id + physical rect`，不允许再从前端逻辑坐标或冻结帧重复推导。

普通截图到录屏的桌面生命周期也已固定：先在 Ordinary 仍完整时核验平台、权限、显示器和物理区域，
把结果收敛为只含可信参数且可跨线程移动的采集计划，再原子移交为 Recording；随后关闭截图覆盖层、
恢复贴图和来源窗口、等待合成器、发布已排除或位于选区外的控制面。原生帧源直到此时才在采集 worker
内创建、使用和析构，因此 macOS 的 Objective-C capture session 无需不安全地伪装为 `Send`。
worker 会等初始化成功才把活动句柄交给会话 owner；初始化错误或 panic 会同步中止 pipeline，并沿
manager/lifecycle 关闭控制面、清理会话、释放 Recording gate。采集取得 generation token 后还必须
由后端把它绑定到控制面；绑定失败会立即
取消刚启动的会话。启动失败、正常停止、取消、迟到 token 和控制面关闭失败均会回收会话并显式
释放 gate；桌面动作会在首个失败后继续执行剩余恢复项。当前完成的是可注入、可测试的领域状态机，
Tauri 桌面动作适配器现已复用截图覆盖层关闭、Pin/来源窗口恢复和 settle 合同；控制窗意外销毁、
显示失败、正常停止与取消都会按 exact token 回收会话。平台计划已经接入这条生命周期；可信 Tauri
开始命令已由独立 `recording-overlay-*` 调用域接入，并固定使用后端生成的会话身份、30 fps、光标
和 VP9 原型；产品策略目前只对显式 VP9 构建的原生 X11 与 Windows Native 返回可用。

控制窗 registry 已独立固定 `Preparing → Bound → Closing → Empty`：窗口只携带后端生成且不可复用的
label，暂停/继续/停止命令从 registry 读取 exact generation token，不接收前端提交的 session ID 或
generation。同一 session ID 再次录制也会得到新 label，旧窗口和迟到 bind 都不能命中新会话；窗口
意外销毁只会交出一次清理 token，关闭失败进入终态并阻止创建替代控制窗。页面 ready 与 token bind
现为顺序无关的双条件屏障：任一方可先到，只有后到的一方取得一次 reveal 责任；重复 ready、伪造
label、旧窗口和 closing 窗口都不能再次显示或控制会话。Tauri 已接入隐藏建窗、物理安全位置复核与
受 caller 限制的控制命令；主界面与普通截图工具条仍没有开始入口，显式 VP9/X11 构建从托盘进入
独立录屏选区。

## 第一阶段验收

- 单区域无音频视频正常停止后可播放，尺寸、时长和选区内容正确；
- 编码阻塞时进程内 RGBA 不超过固定槽位预算，丢帧计数与时间线一致；
- 强制终止后恢复全部已提交分段，损坏或缺失尾段不影响此前连续前缀；
- 暂停时间不进入输出时长，继续后时间戳保持单调；
- 选择取消、权限拒绝、磁盘不足、显示器变化和编码失败不会留下活动采集会话；
- 每个平台单独记录帧源、权限、编码器、容器和播放器证据。
