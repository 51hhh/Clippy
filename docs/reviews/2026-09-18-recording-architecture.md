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
  manager.rs       唯一会话、状态机、取消与资源预算
  manifest.rs      原子 journal、分段哈希、启动恢复
  timeline.rs      单调时间戳、帧丢弃策略、暂停区间
  frame.rs         有界 RGBA 帧合同与固定物理 crop
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
  segment-000000.partial
  segment-000000.<container>
  segment-000001.<container>
```

`manifest.json` 使用版本化 schema，至少记录 session ID、单调时间基、选区/像素尺寸、帧率目标、
编码器身份、容器、创建时间、状态、丢帧数和已提交 segment。每个 segment 记录序号、起始/持续
时间、帧数、字节数与 SHA-256。

提交顺序固定为：完成并 `sync_all` 临时分段 → 原子重命名最终分段 → 用私有临时文件原子替换
manifest。manifest 只能引用已经完成的最终文件。进程中断后忽略 `.partial`，按连续序号、预算、
文件大小和 SHA 验证已提交前缀；坏尾段只截断恢复点，不使此前分段失效。启动扫描限制会话数、
manifest 大小、分段数和总字节，拒绝符号链接、路径分隔符与未知 schema。

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

## 平台顺序

1. 先实现与平台无关的 manifest、时间线、有界队列和合成帧 fixture；
2. Linux X11 用长寿命帧源验证区域 crop、背压和控制窗排除；
3. Windows 使用 WGC/DXGI，macOS 使用系统屏幕录制权限与长寿命帧源；
4. Wayland 使用用户授权的 ScreenCast Portal 会话和其返回的 PipeWire remote FD/stream node，
   不复用一次性 Screenshot Portal，也不借 XWayland 截取原生窗口；
5. 三平台视频闭环完成后再引入单一音轨，先分别建立系统音频/麦克风能力矩阵，再做 A/V 同步。

Linux X11 已加入持久 x11rb 连接的根窗口区域帧源：每次只请求选区物理矩形，依据服务器 visual mask
和字节序转为紧凑 RGBA，并在进入 Clippy 边界时写入单调时间戳。硬件光标通过 XFixes
`GetCursorImage` 取得；其预乘 ARGB 像素按 hotspot 与选区求交后合成，异常尺寸或数据长度会使当前帧
明确失败。它没有采用 xcap 公开的已缩放 `Monitor::x/y`，也没有整屏捕获后裁切。冻结会话到 RandR
物理矩形的可信 handoff、采集 worker、控制窗排除和 X11 真机性能证据仍未接入，因此还不能从 UI
开始录屏。

## 第一阶段验收

- 单区域无音频视频正常停止后可播放，尺寸、时长和选区内容正确；
- 编码阻塞时进程内 RGBA 不超过固定槽位预算，丢帧计数与时间线一致；
- 强制终止后恢复全部已提交分段，损坏或缺失尾段不影响此前连续前缀；
- 暂停时间不进入输出时长，继续后时间戳保持单调；
- 选择取消、权限拒绝、磁盘不足、显示器变化和编码失败不会留下活动采集会话；
- 每个平台单独记录帧源、权限、编码器、容器和播放器证据。
