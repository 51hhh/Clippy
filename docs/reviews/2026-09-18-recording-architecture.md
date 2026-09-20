# PX-REC-01 可恢复录屏技术设计

## 结论

Clippy 现有截图后端不能直接循环调用成录屏。冻结截图优化的是一次性首帧延迟；录屏需要长寿命采集
会话、单调时钟、背压、编码、容器封尾和崩溃恢复。第一阶段只交付单区域、无音频的视频，之后再
增加单一音轨。产品入口必须等待采集、编码和恢复三条链在同一平台同时通过。

xcap 0.9 提供跨平台 `VideoRecorder`，但官方仍把 video recording 标为 WIP；公开 `Frame` 只有
`width`、`height` 和 RGBA `raw`，不含采集时间戳，示例在接收端自行读取 `Instant`。本仓库
`Cargo.lock` 中候选 xcap 0.9.6 的 Linux X11 实现还会以约 1 ms 间隔向无界 `mpsc` 发送整屏 RGBA；
编码器落后时没有背压，内存会随队列增长。Wayland 实现自行建立 Portal/PipeWire 会话，不能由一次
成功截图推断录制可靠。
上游仍有录制配置、Wayland 性能和停止后重新初始化等公开问题。因此 xcap 可以作为候选帧源，不能
承担 Clippy 的时间线、内存和恢复合同，也不能为了录屏直接替换当前 Linux 截图依赖。

参考：

- [xcap 官方录屏示例与 WIP 声明](https://docs.rs/crate/xcap/latest/source/examples/)
- [xcap 公开问题列表](https://github.com/nashaofu/xcap/issues)
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
分段不会被当作有效数据。随后按连续序号、预算、文件大小和 SHA 验证已提交前缀；坏尾段只截断
恢复点，不使此前分段失效。启动扫描限制会话数、manifest 大小、分段数和总字节，拒绝符号链接、
路径分隔符与未知 schema。

正常停止先完成最后分段，再生成单一最终文件。最终合并失败时保留已提交分段和 manifest，允许重试，
不能删除唯一可恢复数据。只有最终文件通过容器探测和时长检查后，才能把会话标成 complete。

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
和停止中都占槽，并以不可复用的 generation token 拒绝迟到的暂停、停止与取消；产品 IPC、桌面资源
恢复和控制窗口尚未接入。

### 编码器 A/B 工具与当前结论

仓库提供可选工程基准 `scripts/benchmark-recording-encoders.mjs`。它用仓库内 Noto CJK 字体生成包含
中英日、数字、货币与数学符号、小字号滚动文字、网格和运动区域的确定性样本，以 FFV1 作为无损
参考，并对每个候选记录墙钟时间、FFmpeg CPU/RSS、文件大小、SSIM、PSNR 以及 `ffprobe` 解码结果。
产物写入显式目录或 `/tmp`，不进入仓库，也不在日常 CI 中执行：

```bash
node scripts/benchmark-recording-encoders.mjs \
  --duration 60 --width 1920 --height 1080 --fps 30 \
  --output /tmp/clippy-recording-codecs-1080p
```

2026-09-20 在 Intel Core Ultra 5 125H、FFmpeg 8.0.1 上先运行 1280×720、30 fps、2 秒的工具自检，
得到以下本地证据：

| 候选 | 墙钟 | 峰值 RSS | 文件大小 | SSIM | PSNR |
|---|---:|---:|---:|---:|---:|
| MJPEG / AVI | 0.126 s | 230.9 MiB | 3.52 MiB | 0.991269 | 34.7485 dB |
| libvpx VP9 / WebM | 0.331 s | 351.1 MiB | 0.14 MiB | 0.993904 | 49.7496 dB |
| rav1e AV1 / Matroska | 4.341 s | 406.0 MiB | 0.17 MiB | 0.993561 | 49.2045 dB |

这次短样本只验证基准工具和候选方向。VP9 在本机样本上同时显著缩小文件并改善客观画质，作为下一
个嵌入原型；只有 libvpx 与 WebM 封装能在同一 SHA 的 Linux、Windows、macOS CI 构建，60 秒
1080p/4K 分层语料满足实时、内存和掉帧预算，且周期分段经过强杀恢复，才可确定为产品默认。
rav1e 当前参数明显未达到实时，本机也没有 NASM，尚不能验证仓库从源码构建时的 x86 汇编优化链，
所以保留为后续可选项。这里不把系统 FFmpeg 已链接的 rav1e 二进制性能归因于本机是否安装 NASM。

OpenH264 的源码本身可嵌入构建，但自行编译的库与 Cisco 分发的预编译二进制不具有同一分发条件；
在许可、专利和安装包策略独立审查前，不把它作为第一阶段默认依赖。MJPEG 继续只承担诊断闭环。

仓库现以非默认 `recording-vp9-prototype` feature 加入第一条嵌入式 VP9/WebM 探针。它把 RGBA 按
BT.709 limited-range 转成 I420，以固定帧率补齐背压造成的时间空洞，每两秒强制关键帧，并用
libwebm `File` 模式写入 seek 信息和显式分段时长；本地回归由 `ffprobe` 核对 VP9、帧数和 200 ms
时长。纯 Rust `ebml-webm 0.2.1` 探针能解码 30 帧，但没有写 `Duration`/`DefaultDuration`，因此没有
进入实现。

该原型仍不具备默认依赖资格：`shiguredo_libvpx 2026.2.0-canary.1` 会在 build script 中从 GitHub
下载按平台预编译的 libvpx 并另取 SHA-256 文件校验，干净构建需要网络，且绑定仍是 canary。
Clippy 已精确固定 crate 版本，并把整个模块隔离在非默认 feature；启用默认产品构建、会话 owner 或
UI 前，必须改成可复现的依赖获取方式，补齐 MPL/libvpx 第三方声明，并让同一 SHA 的 Ubuntu 22.04、
Windows x64、macOS Intel/Apple Silicon 均编译和运行该 feature。当前 Linux 探针成功不替代这些门禁。

编码消费线程也已从 MJPEG 具体类型收敛为 `RecordingSegmentWriter` 合同：writer 只能接收时间线已经
归一化的 RGBA 帧并返回同一个最终时长下的 writer 与帧数；pipeline 排空、原始错误优先级、异常中止
和 join 只实现一次。MJPEG 会话继续使用原类型别名，默认行为不变；VP9 feature 的线程回归已经证明
同一三槽 pipeline 可封尾 200 ms WebM。会话 journal 选择和周期分段仍未切换到 VP9。

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
安全矩形时明确返回 `TrayAndShortcutsOnly`。窗口句柄排除调用和控制窗宿主仍待平台实现。

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
中止 pipeline，停止仍在运行的采集线程。桌面资源恢复领域状态机已经接入；产品 IPC、Tauri 控制窗
宿主和 X11 真机性能证据仍未接入，因此还不能从 UI 开始录屏。

`ci-local.sh` 现会在隔离 Xvfb 中显式运行原生闭环：RandR 可信区域 → 持久 X11 帧源 → 采集线程 →
三槽 pipeline → MJPEG/AVI → 私有分段与 complete manifest，并核对清单帧数和文件权限。它验证真实
X11 协议与落盘接线，不代表真实桌面合成器、4K 带宽、光标移动或控制窗排除的性能验收。
X11 帧源在原生 RandR 核验完成后还会产出唯一的物理来源描述；录制清单与后续控制窗规划均应消费
这份 `source_id + physical rect`，不允许再从前端逻辑坐标或冻结帧重复推导。

普通截图到录屏的桌面生命周期也已固定：先在 Ordinary 仍完整时连接平台帧源，再原子移交为
Recording；随后关闭截图覆盖层、恢复贴图和来源窗口、等待合成器、发布已排除或位于选区外的控制面，
最后才启动采集线程。采集取得 generation token 后还必须由后端把它绑定到控制面；绑定失败会立即
取消刚启动的会话。启动失败、正常停止、取消、迟到 token 和控制面关闭失败均会回收会话并显式
释放 gate；桌面动作会在首个失败后继续执行剩余恢复项。当前完成的是可注入、可测试的领域状态机，
Tauri 桌面动作适配器和产品 IPC 仍未接入。

控制窗 registry 已独立固定 `Preparing → Bound → Closing → Empty`：窗口只携带后端生成且不可复用的
label，暂停/继续/停止命令从 registry 读取 exact generation token，不接收前端提交的 session ID 或
generation。同一 session ID 再次录制也会得到新 label，旧窗口和迟到 bind 都不能命中新会话；窗口
意外销毁只会交出一次清理 token，关闭失败进入终态并阻止创建替代控制窗。

## 第一阶段验收

- 单区域无音频视频正常停止后可播放，尺寸、时长和选区内容正确；
- 编码阻塞时进程内 RGBA 不超过固定槽位预算，丢帧计数与时间线一致；
- 强制终止后恢复全部已提交分段，损坏或缺失尾段不影响此前连续前缀；
- 暂停时间不进入输出时长，继续后时间戳保持单调；
- 选择取消、权限拒绝、磁盘不足、显示器变化和编码失败不会留下活动采集会话；
- 每个平台单独记录帧源、权限、编码器、容器和播放器证据。
