# 平台能力调研与约束

## 当前代码事实

- `lib.rs` 无条件声明 Linux DBus/GSettings/Paste 模块，并在启动时把非 Wayland 路径作为 X11；这是 Windows/macOS 首个编译隔离问题。
- `Cargo.toml` 把 `xcap 0.9` 放在全局依赖。该版本在 Linux 引入 PipeWire 绑定，stock Ubuntu 22 的 0.3.48 开发头文件不足以覆盖当前绑定使用的结构字段。
- Linux 专用依赖已经有 target section，但 `clipboard_watcher`、`paste`、`shortcut_conflict`、设置命令和部分 Pin/截图代码仍直接引用 Linux 模块。
- `tauri.conf.json` 的 dev/build 命令包含 POSIX shell block 和重定向；Windows shell 不可移植。bundle/updater 也仍是 Linux-only。
- 图片保存使用 `$HOME/Pictures/Clippy`；OCR 安装使用 `pkexec apt-get`；安装类型默认返回 deb，均需平台化。
- 标注画布、PNG iTXt 工程、SQLite、翻译和大部分 React UI 是平台无关能力，应避免引入不必要分叉。

## 功能矩阵

| 功能 | Linux X11 | Ubuntu 22 GNOME Wayland | 新版 GNOME/KDE/wlroots Wayland | Windows 10/11 | macOS |
|---|---|---|---|---|---|
| 文本/HTML/图片剪贴板 | arboard/X11 | arboard XWayland 或 data-control；不保证所有 compositor | 优先 data-control，保留回退 | arboard/native | arboard/native |
| 自动粘贴 | 恢复 X11 窗口 + Ctrl+V | RemoteDesktop Portal，会话授权可能不可恢复 | Portal，可探测 restore 支持 | HWND + SendInput；UIPI 时 copy-only | 激活目标应用 + CGEvent；需 Accessibility |
| 全局快捷键 | Tauri global shortcut | GNOME GSettings/DBus | GlobalShortcuts Portal 或桌面后端 | Tauri global shortcut | Tauri global shortcut |
| 区域截图 | X11 capture | Mutter/扩展/Portal | GNOME/KDE Portal 或 wlroots | xcap/WGC | CoreGraphics/xcap/ScreenCaptureKit |
| 全局窗口枚举 | X11 EWMH | 需 GNOME Shell 扩展 | compositor 私有协议；标准 Wayland 不完整 | EnumWindows/xcap | CGWindowList |
| 绝对窗口定位 | 支持 | compositor 控制 | compositor 控制 | 支持 | 支持 |
| Pin 始终置顶 | 支持 | Mutter 无通用 layer-shell | wlroots/KWin 可增强 | 原生 topmost | NSWindow level |
| 混合 DPI | XRandR/logical-physical 换算 | 每输出 scale + compositor | 每输出 scale + compositor | per-monitor DPI | backingScaleFactor |
| OCR | 系统 Tesseract/sidecar | 同左 | 同左 | sidecar/安装检测 | sidecar/安装检测 |
| iTXt 编辑工程 | 完整 | 完整 | 完整 | 完整 | 完整 |

## 官方资料与版本事实

- Tauri 支持平台覆盖配置，公共配置可与 `tauri.linux.conf.json`、`tauri.windows.conf.json`、`tauri.macos.conf.json` 合并：https://v2.tauri.app/reference/config/
- Tauri AppImage 应在最老支持发行版构建，避免无意抬高 glibc 基线：https://v2.tauri.app/distribute/appimage/
- Tauri global-shortcut 插件覆盖 Linux、Windows、macOS：https://v2.tauri.app/plugin/global-shortcut/
- xdg-desktop-portal GlobalShortcuts 在 1.16 引入；Ubuntu 22 的 portal 为 1.14 系列，因此 Jammy 不可只依赖该接口：https://github.com/flatpak/xdg-desktop-portal/blob/main/NEWS.md
- GlobalShortcuts 的 `BindShortcuts` 每个 session 只能调用一次，返回值允许是请求集合的任意子集；实现必须核对实际返回项，配置变化时重建 session：https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html
- `preferred_trigger` 使用 XDG Shortcuts 语法：`CTRL` / `ALT` / `SHIFT` / `LOGO` 与去掉 `XKB_KEY_` 前缀的 keysym 通过 `+` 连接：https://specifications.freedesktop.org/shortcuts/latest/
- xdg-shell 不给普通客户端任意设置顶层窗口全局坐标的通用协议：https://wayland.app/protocols/xdg-shell
- RemoteDesktop Portal 的输入注入由用户授权决定：https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html
- Windows `SendInput` 受 UIPI 限制，不能保证从普通完整性进程向高完整性进程注入：https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput
- Windows 前台窗口切换本身有限制：https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow
- Windows Graphics Capture 提供系统安全选择器，并要求检测 `IsSupported`：https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture
- macOS 屏幕捕获和输入控制分别受 Screen Recording 与 Accessibility 权限约束：https://developer.apple.com/documentation/screencapturekit/ ，https://support.apple.com/guide/mac-help/mh43185/mac
- macOS 外部分发需要 Developer ID、Hardened Runtime 与 notarization：https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution

## Ubuntu 22 依赖决策

推荐长期路径：

1. 将 `xcap 0.9` 限制到 Windows/macOS。
2. Linux X11 用现有 `x11rb` 或 Jammy-compatible X11-only 实现捕获和窗口探测。
3. Linux Wayland 保留 Mutter、GNOME Shell、Portal 和 wlroots 后端，但直接 PipeWire 快速路径不得成为基础构建的强制依赖。
4. 以 Ubuntu 22 原生 runner/container 构建 release，再向 Ubuntu 24 前向验证。

备选验证路径：Linux 临时固定 `xcap = 0.4.1`。该版本没有当前 PipeWire 依赖，可用于确认现有 API 迁移成本，但旧 0.x 依赖和维护风险使其不适合作为未经验证的最终架构。

## 不可绕过边界

- Wayland compositor 不提供的全局窗口几何、绝对坐标和置顶协议，客户端无法靠 Tauri 绕过。
- macOS 用户拒绝 Screen Recording/Accessibility 权限后，应用不能静默捕获或注入输入。
- Windows 普通权限进程不能保证向管理员进程或安全桌面注入输入。
- DRM/受保护内容可以返回黑帧或拒绝捕获。

这些边界必须进入 `PlatformCapabilities` 和 UI fallback，而不是被当成需要无限重试的偶发错误。
