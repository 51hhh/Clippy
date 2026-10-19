# PX-REC-MACOS-SCK-01 — macOS ScreenCaptureKit 录屏原型

日期：2026-09-21

关联路线：`PX-REC-01` / `PX-REC-WINDOWS-QA-01`

## Goal

在不抬高默认 macOS 11 发布基线的前提下，为 macOS 12.3 及以上的显式 QA 构建加入
ScreenCaptureKit 区域帧源，并用 `SCContentFilter(display:excludingWindows:)` 精确排除 Clippy
录屏控制窗。通过后，现有可信选区、VP9/WebM 分段、暂停/继续/停止、恢复和结果库可以在 macOS
进入可安装、可真机验收的非默认原型。

## Requirements

1. 新增独立的 macOS ScreenCaptureKit feature。它包含既有 VP9 原型，但不进入 default features、
   正式 release workflow 或普通 macOS 11 安装包。只有 macOS Native、显式 feature 和受支持系统版本
   同时成立时，产品入口才可见；其他构建继续隐藏入口并拒绝开始命令。
2. 默认应用的 `minimumSystemVersion` 保持 `11.0`。ScreenCaptureKit QA 包必须明确声明 macOS 12.3
   或以上基线，并包含 `NSScreenCaptureUsageDescription`；低版本不能因强链接新框架而破坏默认应用
   启动。
3. 控制窗先以隐藏状态由 Rust 创建。后端从该 Tauri 窗口取得可信 `NSWindow.windowNumber`，并把它
   作为只在本次启动流程内有效的原生排除目标交给帧源。WebView 不能提交、覆盖或复用 window ID；
   目标缺失、越界、过期或与 `SCShareableContent` 不唯一匹配时必须回滚录屏。
4. 帧源必须在 `SCShareableContent` 中唯一匹配冻结选区的 CoreGraphics display ID 和控制窗
   `CGWindowID`，再构造 `SCContentFilter(display:excludingWindows:)`。不得用
   `NSWindow.SharingType`、content protection、逐帧隐藏窗口或排除整个 Clippy 应用替代精确窗口过滤。
5. `SCStreamConfiguration` 只捕获可信选区：`sourceRect` 使用显示器逻辑点坐标，输出宽高等于冻结
   backing pixels，像素格式固定 BGRA，光标开启，帧率固定为后端策略，队列深度保持有界。显示器
   ID、逻辑尺寸、point-to-pixel scale 或输出尺寸变化必须在首帧前或当前帧上明确失败，禁止拉伸和
   先分配整块 6K/8K RGBA 后裁切。
6. `SCStream`、delegate、dispatch queue 和 CoreVideo 对象只在采集 worker 内创建、使用并析构。
   输出回调不得无限阻塞系统队列；开始、暂停、继续、停止和 Drop 都要有界等待并关闭原生 stream，
   丢弃暂停前缓存，不能留下 WindowServer 捕获或后台线程。系统权限拒绝与 stream 终止必须转换为
   稳定的录屏启动/运行错误，并沿既有 lifecycle 回滚控制窗与 Recording gate。
7. macOS Intel 与 Apple Silicon Native QA 包启用该 feature；Intel 继续使用锁定的 libvpx 源构建。
   QA 元数据必须记录实际 feature 和最低系统版本。结构化真机合同覆盖权限首次授权/拒绝、区域与
   Retina 像素、移动光标、控制窗排除、暂停/继续/停止、播放/缩略图/导出和异常分段恢复。
8. 代码、供应链脚本、架构文档与用户可见 CHANGELOG 同步记录该能力仍是非默认原型。原生 runner
   编译、安装包生成和真机录制属于三层独立证据，任何一层不能替代另一层。

## Acceptance Criteria

- [x] 策略测试证明 macOS 入口只在专用 feature、Native 会话和受支持版本组合下开放。
- [x] 生命周期测试证明源连接只能消费本次 Rust 创建控制窗返回的原生排除目标；缺失和迟到目标不会
  开始捕获，并完整回滚会话。
- [x] ScreenCaptureKit 适配器唯一匹配 display/window，固定选区像素合同，并在 callback、权限、
  start/stop、几何和流终止错误上有可回归的失败路径。
- [x] 默认 macOS 11 构建不链接专用 feature，正式 release workflow 不启用它；专用 QA 包明确使用
  macOS 12.3+ 与屏幕录制用途说明。
- [x] Linux/Windows 行为和门控保持不变，完整本地门禁及同一 SHA 的 Ubuntu、Windows、macOS 原生
  check/clippy/test 通过。
- [x] 同一 SHA 的 macOS Intel/Apple Silicon QA 包成功生成；真机记录完成前不把 macOS 录屏记为
  发布可用。

## Out of Scope

- 不把录屏或 ScreenCaptureKit feature 设为默认，不修改正式发布包的 macOS 11 最低版本。
- 不加入系统音频、麦克风、摄像头、GPU 编码、剪辑或转码。
- 不开放 Wayland 入口，不在本阶段实现托盘/快捷键无控制窗后备。
- 不用 GitHub runner、合成帧或编译成功代替 macOS 12.3+ 真机上的 TCC、光标、控制窗排除、混合
  DPI、旋转屏、4K/6K、长时间资源与系统播放器验收。

## Primary API Sources

- [Apple `SCContentFilter`](https://developer.apple.com/documentation/screencapturekit/sccontentfilter)
- [Apple ScreenCaptureKit sample](https://developer.apple.com/documentation/screencapturekit/capturing-screen-content-in-macos)
- [Apple `SCStreamConfiguration.sourceRect`](https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/sourcerect)
- [Apple `SCStreamConfiguration.queueDepth`](https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/queuedepth)
- [`objc2-screen-capture-kit` 0.3.2](https://docs.rs/objc2-screen-capture-kit/0.3.2/objc2_screen_capture_kit/)

## Verification

- 2026-09-21：`./scripts/ci-local.sh` 在 Linux x86_64 完整执行，25 项通过、0 项失败、2 项按配置
  跳过；Rust 单元测试为 1039 通过、14 忽略，前端为 73 个文件 / 1245 项通过，私有 X11、DOM、
  Canvas、布局 smoke 与 Vite 生产构建通过。
- 2026-09-21：用最小 macOS 编译夹具和仓库锁定的 objc2 依赖对
  `platform/macos/screencapturekit.rs` 执行 `aarch64-apple-darwin`、`-D warnings` 离线检查并通过。
  Linux 无 Apple SDK；整个 Tauri crate 已由下述远程 macOS runner 判定。
- 2026-09-21：同一 SHA `66a5d1d88b828fa821b2074f5055b43203ad1a41` 的
  [CI Check 35564816736](https://github.com/51hhh/Clippy/actions/runs/35564816736) 七个 job 全部通过，
  覆盖 Ubuntu、Windows 与 macOS 的 check/clippy/test；`verify-native-ci.mjs` 对该 40 位 SHA 判定
  `PASS`。
- 2026-09-21：同一 SHA 的
  [Native QA 35567132842](https://github.com/51hhh/Clippy/actions/runs/35567132842) 五个 job 全部通过，
  并生成 macOS Intel 与 Apple Silicon QA 包；同时生成 Linux x64、Windows x64、录屏模板与
  Ubuntu 24 AppImage runtime smoke 证据。
- macOS 12.3+ Intel/Apple Silicon 真机上的 TCC、Retina/旋转屏、控制窗排除、暂停恢复、长时间资源
  和系统播放器记录仍为 `not_run`。安装包生成不能替代真机录制，本能力仍不记为发布可用。
