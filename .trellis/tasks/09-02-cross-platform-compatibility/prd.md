# Clippy 跨平台兼容与 Ubuntu 22 基线

## Goal

在不削弱现有 Linux 截图、剪贴板、Pin、OCR、翻译和可编辑 PNG 工作流的前提下，建立明确的平台能力层，使 Clippy 能在 Ubuntu 22 X11/Wayland、较新 Linux 桌面、Windows 10/11 和 macOS 上编译、打包、运行，并在系统安全模型不允许完整能力时给出可解释、可测试的降级行为。

## Supported Baselines

- Linux：Ubuntu 22.04 为构建和运行最低基线；重点覆盖 GNOME 42 X11/Wayland，并验证 Ubuntu 24.04 GNOME Wayland、KDE Wayland 和一个 wlroots compositor。
- Windows：Windows 10 22H2 和 Windows 11，首发 x86_64，随后评估 ARM64。
- macOS：Intel 与 Apple Silicon；最低版本由最终截图实现决定，若使用 ScreenCaptureKit 则不得声称支持低于该 API 可用版本。
- 不以交叉编译代替原生平台构建；安装包在对应系统 runner 上生成。

## Requirements

### 1. 平台抽象与能力发现

- 建立统一 `platform` 模块；编译期只选择当前操作系统模块，Linux 专用 crate 和源码不得泄漏到 Windows/macOS。
- Linux 内部运行时区分 X11、原生 Wayland、XWayland、桌面环境、Portal 接口可用性及版本。
- Windows 运行时检测截图 API、前台窗口和输入注入结果；macOS 运行时检测屏幕录制和辅助功能权限。
- 后端通过 typed IPC 暴露结构化 `PlatformInfo` / `PlatformCapabilities`，前端按能力显示功能和原因，不通过 user-agent 或环境变量自行猜测。
- 每项能力至少表达 `available`、`permission_required`、`degraded` 或 `unsupported`，并提供稳定 reason code。

### 2. 剪贴板与自动粘贴

- 文本、HTML 和图片读写在三平台保持一致；Linux Wayland 优先使用原生 data-control 能力，失败时允许 XWayland 或 copy-only。
- 自动粘贴后端按平台实现：X11 输入注入、Wayland RemoteDesktop Portal、Windows 前台窗口恢复 + `SendInput`、macOS 应用恢复 + `CGEvent`。
- Windows 高完整性目标、macOS 未授权、Wayland Portal 被拒绝或接口不存在时，必须保留复制成功并返回明确降级状态，不得无限重试或反复弹授权框。
- Portal restore token 和其他敏感凭据必须使用私有存储；不支持恢复的 Ubuntu 22 Portal 允许会话级授权。

### 3. 全局快捷键

- Windows/macOS/X11 使用 Tauri global-shortcut 能力；macOS 默认展示 Command 语义。
- Ubuntu 22 GNOME Wayland 保留 GSettings/DBus 路径；支持 GlobalShortcuts Portal 的新桌面可在运行时使用 Portal。
- 快捷键注册、暂停、恢复、冲突检测和错误返回通过统一接口完成。
- 现有用户快捷键配置必须兼容；新增默认值使用 `CmdOrCtrl` 等跨平台 accelerator。

### 4. 截图与窗口命中

- 保持区域截图、窗口命中、多显示器、混合 DPI、编辑、Copy/Save/Pin/Translate 工作流。
- Ubuntu 22 构建不得强制依赖比系统 PipeWire 0.3.48 更新的开发头文件。
- Linux X11 使用 Jammy 可编译的显示器/窗口/像素捕获实现；Wayland 依次使用可用的 GNOME、Portal、wlroots 或 copy-safe fallback。
- Windows 首版可使用 xcap 捕获并用 Win32 完成 HWND 枚举、窗口几何和前台窗口；后续可切换 Windows Graphics Capture。
- macOS 使用 CoreGraphics/xcap 或 ScreenCaptureKit，并正确处理屏幕录制权限。
- DRM、安全桌面和 compositor 不暴露窗口几何时允许返回不可捕获或仅区域选择，不伪造窗口信息。

### 5. 窗口、覆盖层与 Pin

- 主窗口、截图覆盖层、工具栏和 Pin 统一使用 logical/physical 坐标契约，并在混合 DPI 场景测试。
- Windows 使用原生 topmost/layered window 能力；macOS 使用窗口 level、Spaces/全屏集合行为；Linux X11 保持现有控制。
- Wayland 由 compositor 控制绝对位置时，返回 capability 降级；不得循环调用被忽略的定位 API。
- GNOME Mutter 无通用 layer-shell 时不承诺 Pin 永久置顶；UI/诊断信息必须说明限制。
- 可编辑 PNG 的原图、操作层、合成预览和 iTXt 工程数据保持平台无关，三平台读写结果一致。

### 6. 画布、文件和系统目录

- 标注画布、撤销重做、模糊、马赛克、文字、滤镜、原图坐标和工程恢复不包含平台分支。
- 保存目录使用 Tauri 平台目录 API，不硬编码 `$HOME/Pictures` 或 Unix 路径分隔符。
- PNG/iTXt 项目导入继续执行大小、校验和、坐标和 schema 验证；外部文件一律视为不可信输入。
- Windows 对私有文件使用应用私有目录并评估 ACL；Unix 保持 0600/0700。

### 7. OCR、翻译、密钥与可选集成

- OCR 后端先检测可用性；安装入口按平台提供，禁止在 Windows/macOS 调用 `pkexec apt-get`。
- 评估签名 Tesseract sidecar；若暂不捆绑，设置页展示平台对应安装说明并隐藏不可用安装按钮。
- keyring 在三平台使用原生凭据存储；失败时不得自动降级为明文。
- tmux 监控仅在支持平台展示；Windows 必须编译排除 Linux inotify/nix 实现。
- SQLite、翻译 HTTP、搜索、收藏、托盘、自动启动、单实例和更新器保持可用，并为每个平台生成正确 updater artifact。

### 8. 构建、打包与发布

- 公共 Tauri 配置只保存跨平台字段；Linux、Windows、macOS 使用平台覆盖配置。
- dev/build 命令不依赖 POSIX shell 语法。
- CI 至少包含 Ubuntu 22、Windows 和 macOS 的 fmt/typecheck/test/build 或可行的 compile gate。
- Linux 在 Ubuntu 22 runner 构建 deb/AppImage；Windows 生成 MSI/NSIS；macOS 生成签名后的 app/dmg。
- macOS 正式发布执行 Developer ID 签名、Hardened Runtime 和 notarization；测试用自签名证书不得作为发布验收依据。
- updater manifest 包含每个已发布平台与架构，安装类型判断不得把非 AppImage 平台统称为 deb。

### 9. 诊断与测试

- 诊断输出记录操作系统、会话类型、桌面环境、Portal 能力、选择的后端及降级原因，但不得记录剪贴板内容、token 或密钥。
- 平台选择逻辑、能力序列化、目录选择、快捷键规范化和 fallback 决策具有单元测试。
- 自动化测试不能替代实机测试；必须维护 X11/Wayland、Windows DPI/UIPI、macOS TCC/Spaces 的人工矩阵。
- 每个平台验证文本/HTML/图片剪贴板、自动粘贴、快捷键、截图、窗口选择、Pin、画布、可编辑 PNG、OCR、密钥和更新器。

## Acceptance Criteria

- [x] Linux 本机现有功能通过完整测试，且平台抽象不改变已验证的 X11/Wayland 选择结果。
- [x] 在干净 Ubuntu 22.04 环境中无需第三方 PipeWire PPA 即可完成 release 构建。
- [ ] Ubuntu 22 GNOME 42 X11 与 Wayland 的快捷键、截图、复制和可用的粘贴路径有实机记录。
- [x] Windows 10/11 和 macOS 原生 runner 能完成 Rust compile、前端 typecheck/test/build 和 Tauri bundle。
- [ ] Windows 普通目标自动粘贴成功；高完整性目标稳定降级为 copy-only 并显示 reason code。
- [ ] macOS 屏幕录制/辅助功能权限的未决定、拒绝、已授权和撤销状态均有可恢复行为。
- [ ] Wayland 缺少窗口几何或绝对定位能力时，区域截图仍可用且 UI 不声称支持窗口命中或固定置顶。
- [ ] 三平台保存路径来自系统目录 API；可编辑 PNG 在三平台可打开、继续编辑并导出相同合成结果。
- [x] OCR 安装入口不会在错误平台执行 Linux 命令；keyring 不降级为明文。
- [ ] 平台 Tauri 配置、安装包和 updater manifest 与实际目标匹配。
- [x] `cargo fmt --check`、`cargo check/test/clippy`、前端 test/typecheck/build、quick CI 和完整 CI 全部通过；无法在本机执行的平台项准确记录在 QA 矩阵。

## Out of Scope

- 不通过 root 服务、uinput 守护进程、关闭 SIP/TCC、提权 Clippy 或 UIAccess 绕过操作系统安全模型。
- 不承诺捕获 DRM/受保护视频、安全桌面或其他系统明确禁止的内容。
- 不承诺所有 Wayland compositor 都支持全局窗口枚举、绝对定位或始终置顶。
- 不以 Wine、虚拟 API mock 或仅交叉编译结果代替 Windows/macOS 原生运行验证。
- 本任务不重写平台无关的编辑器和可编辑 PNG 数据模型，除非跨平台验证发现真实缺陷。
