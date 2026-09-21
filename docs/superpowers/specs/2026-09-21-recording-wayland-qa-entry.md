# PX-REC-WAYLAND-QA-01 — Wayland Portal 录屏 QA 入口

日期：2026-09-21

关联路线：`PX-REC-01` / `PX-ROADMAP-2026-09`

## Goal

在不改变默认发布 feature 的前提下，为 Linux Wayland 专用 QA 构建开放现有
ScreenCast Portal + PipeWire 区域录屏链路。系统选源对话框必须绑定可信 Clippy 父窗口并可取消；
录制开始后不保留可被合成器录入的控制窗，改由原生托盘菜单提供暂停、继续和停止。

## Requirements

1. 新增 `recording-wayland-qa` feature。它包含 VP9 原型，但不进入 default features 或正式 release
   workflow。只有 Linux、原生 Wayland 会话和该 feature 同时成立时才开放入口；X11、Windows、
   macOS 与默认构建的既有策略保持不变。
2. 授权父窗口由 Rust 创建。后端只能从本次 `recording-control-*` Tauri 窗口的 Wayland surface 和
   display 导出 xdg-foreign 标识，并在 Portal 交互结束前保持导出对象与窗口存活。WebView/IPC 不得
   提交父窗口字符串、surface 指针、Portal token 或 PipeWire node。
3. 授权窗口显示明确的等待状态和 Cancel。取消、窗口意外销毁、Portal 拒绝、缺少 xdg-foreign、
   返回错误显示器或多 stream 时，必须关闭 Portal session、回收控制窗、释放 Recording/capture
   gate，且不留下录屏文件或后台采集线程；迟到响应不能启动后续会话。
4. Portal 只请求一个 Monitor source、Embedded cursor 与 `PersistMode::DoNot`。返回 stream 必须继续
   通过冻结显示器的逻辑位置/尺寸复核；单显示器且 Portal 不返回几何时才允许既有兼容分支。PipeWire
   格式、像素预算和可信无缩放 crop 合同保持不变。
5. Portal 授权成功且 stream 身份核对完成后，必须先隐藏授权窗口，再打开 PipeWire remote、启动
   采集 worker。Wayland 不创建或显示录屏浮动控制窗，也不依赖绝对定位、content protection 或
   合成器排除窗口能力。
6. 专用 QA 构建在原生托盘提供 Pause、Resume、Stop。菜单事件直接从后端当前活动 lifecycle 取得
   exact generation token；前端不能提交 token。空闲、启动、录制、暂停和结束状态必须限制菜单
   可用性，迟到事件不得控制随后建立的会话。
7. Linux Native QA 包使用 `recording-wayland-qa`，`QA-BUILD.txt` 记录实际 feature。Wayland 五个
   profile 覆盖授权/取消、单/多显示器、混合缩放、区域像素、光标、托盘暂停/继续/停止、结果库与
   强杀恢复；X11 profile 仍执行现有控制窗在选区外合同。
8. 架构文档、Native QA 文档、结构化模板和 CHANGELOG 同步记录该能力仍为无音频、非默认原型。
   GitHub runner 编译和安装包生成不能替代 GNOME、KDE、wlroots 真机 Portal/PipeWire 验收。

## Acceptance Criteria

- [x] 策略与回归测试证明 Wayland 入口只由专用 feature 开放，default/release 不携带该 feature。
- [x] 父窗口和授权取消合同有单元测试；缺失/错误 target、取消竞态和迟到响应均安全失败。
- [x] 授权窗口在 PipeWire 启动前隐藏，录制期间只保留托盘控制；全屏选区不依赖窗口可放置空间。
- [x] 托盘控制只操作后端当前活动 token，并覆盖空闲、录制、暂停、停止和迟到事件状态。
- [x] Wayland 五个结构化 QA profile 使用可执行录屏合同，X11/Windows/macOS 合同不回退。
- [x] `cargo fmt`、Rust check/clippy/test、前端测试/类型检查/构建和仓库静态门禁通过。
- [x] 同一 SHA 的 Ubuntu/Windows/macOS 原生 CI、Linux QA 包与 Ubuntu 24 AppImage runtime smoke
  成功。
- [ ] GNOME、KDE、wlroots 原生 Wayland 真机记录完成前，不把该入口记为默认或发布可用。

## Out of Scope

- 不启用默认录屏入口，不修改正式 release feature。
- 不加入系统音频、麦克风、摄像头、GPU 编码、剪辑或转码。
- 不保存 Portal restore token，不绕过每次由用户确认的系统选源流程。
- 不承诺合成器能排除任意 Clippy 窗口；录制阶段以没有可见控制窗为合同。
- 不用 XWayland 根窗口、Screenshot Portal 轮询或逐帧隐藏窗口替代 ScreenCast/PipeWire。

## Verification

- `./scripts/ci-local.sh`：25 项通过、0 失败；非宿主交叉 lint 与 AppImage smoke 2 项按脚本配置
  跳过。默认 Rust 主测试 1048 项通过、14 项忽略；X11 私有剪贴板协议与大图传输通过；前端 73 个
  文件、1246 项测试以及 DOM、Canvas、布局 smoke 和 Vite 构建通过。
- `cargo clippy --all-targets --features recording-wayland-qa -- -D warnings`：通过。
- `cargo test --features recording-wayland-qa`：Rust 主测试 1074 项通过、0 失败、15 项忽略；4 项需
  私有 Xvfb 的集成测试保持忽略，并已由默认完整门禁的隔离 X11 步骤执行通过。
- 授权取消、错误 target、授权页迟到 ready、空启动会话回滚和迟到原生菜单 generation 均有回归
  测试；专用 feature 的 Portal parent 依赖不进入 default/release feature。
- 2026-09-21：同一 SHA `7b8f1b9a8480fc651faae1af2b6e84c41c51fd34` 的
  [CI Check 35574613771](https://github.com/51hhh/Clippy/actions/runs/35574613771) 七个 job 全部通过，
  覆盖 Ubuntu 主检查、Windows/macOS 原生 check/clippy/test，以及 Ubuntu、Windows、macOS ARM64
  和 macOS Intel 的录屏编码原型。
- 2026-09-21：同一 SHA 的
  [Native QA 35577757619](https://github.com/51hhh/Clippy/actions/runs/35577757619) 五个 job 全部通过，
  生成 Linux Wayland、Windows x64、macOS Intel/Apple Silicon QA 包，并通过 Ubuntu 24 AppImage
  runtime smoke。
- GNOME、KDE、wlroots 真机上的 Portal 授权、选区像素、光标、托盘控制与强杀恢复 profile 仍为
  `not_run`。安装包生成不能替代真机录制，本能力保持非默认 QA 原型。
