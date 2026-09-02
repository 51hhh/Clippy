# 技术设计与实施计划

## 目标架构

```text
commands / app wiring / feature domains
                  │
                  ▼
        platform::PlatformServices
          │       │       │
          ▼       ▼       ▼
       linux    windows   macos
      X11/Wayland  Win32  AppKit/CoreGraphics
```

`platform` 负责系统事实和原生操作，剪贴板、截图、Pin、翻译等领域模块只依赖稳定能力接口，不在业务层散布 `cfg` 和环境变量判断。

## 核心数据契约

```rust
enum CapabilityState {
    Available,
    PermissionRequired,
    Degraded,
    Unsupported,
}

struct Capability {
    state: CapabilityState,
    reason: Option<CapabilityReason>,
}

struct PlatformCapabilities {
    clipboard_text: Capability,
    clipboard_image: Capability,
    auto_paste: Capability,
    global_shortcuts: Capability,
    screen_capture: Capability,
    window_pick: Capability,
    absolute_window_position: Capability,
    always_on_top: Capability,
    ocr: Capability,
}

struct PlatformInfo {
    operating_system: OperatingSystem,
    session: DesktopSession,
    desktop_environment: Option<String>,
    architecture: String,
    xwayland_available: bool,
    portal: PortalInfo, // 服务状态与四个接口各自的 available/version
    capabilities: PlatformCapabilities,
}
```

Reason code 必须是 serde 稳定枚举，文案由前端 i18n 决定。诊断信息可以携带后端名称，但不能把自由文本错误当成 UI 逻辑条件。
Portal 接口存在性必须读取各接口的 `version` 属性；不能用 portal 进程存在或软件包总版本推断。

## 分阶段实施

### Phase A：平台骨架和编译隔离

1. 新建 `src-tauri/src/platform/`：公共模型、session detection 和三个 OS 模块。
2. 把 DBus、GSettings、Linux Portal/X11 paste、tmux watcher、Linux shortcut conflict 等声明放入 Linux `cfg`。
3. 为 Windows/macOS 提供真实的能力探测和显式 copy-only/unsupported 后端，不用假成功 stub。
4. 新增 `get_platform_info` IPC 和 typed frontend wrapper。
5. 将启动、设置、快捷键和 Pin 的 `is_wayland()` 调用迁移到统一 platform API。
6. 在 Linux 本机运行全量回归，确保现有选择路径不变。

### Phase B：构建、目录与配置可移植性

1. 把 Vite 命令移入 npm scripts，Tauri 配置调用不含 shell block 的跨平台命令。
2. 拆分三个 `tauri.*.conf.json`。
3. 保存目录切到 Tauri path resolver；安装类型改为真实 OS/package enum。
4. OCR 设置按平台暴露 `available/installable/instructions`。
5. 默认快捷键使用 `CmdOrCtrl`，保留旧配置兼容。

### Phase C：Ubuntu 22 基线

1. 从 Linux 全局路径移除 xcap 0.9/PipeWire 0.9 强依赖。
2. 实现或接入 Jammy-compatible X11 截图、窗口枚举和显示器信息。
3. Wayland 后端按实际接口探测：Mutter → GNOME extension → Portal → wlroots → 明确失败。
4. 对 Ubuntu 22 portal 缺少 GlobalShortcuts/restore 的情况提供 GSettings 和会话级授权。
5. 增加 Ubuntu 22 release build gate，并审计 ELF 动态依赖。

### Phase D：Windows 10/11

1. 原生 HWND 目标捕获、前台恢复与窗口枚举。
2. `SendInput` 自动粘贴；失败/UIPI 场景 copy-only。
3. xcap 或 Windows Graphics Capture 截图和 per-monitor DPI 坐标换算。
4. topmost/layered Pin、透明覆盖层和多显示器 placement。
5. MSI/NSIS、WebView2 策略、updater 和 Tesseract sidecar/提示。
6. 私有文件使用仅当前用户可访问的受保护 DACL；配置和 Portal token 可原子覆盖并在替换后复验权限。

### Phase E：macOS

1. 前台应用记录/恢复、CGEvent 自动粘贴和 Accessibility onboarding。
2. CoreGraphics/xcap 或 ScreenCaptureKit 捕获以及 Screen Recording onboarding。
3. CGWindowList 窗口候选、Retina 坐标、NSWindow level/Spaces 行为。
4. Command 快捷键展示、系统目录、keychain。
5. Intel/Apple Silicon 构建、签名、公证和 updater。

### Phase F：验证和发布门禁

1. 纯逻辑单测覆盖能力选择、fallback、目录、快捷键和 IPC schema。
2. 三平台原生 CI 编译与测试。
3. 维护实机矩阵：多屏、混合 DPI、热插拔、权限拒绝/撤销、Windows elevated target、macOS Spaces/fullscreen。
4. 对每个安装包做安装、自动启动、更新、卸载和数据保留测试。

## 小步提交计划

每个提交只完成一个可验证目标，并遵循中文 Conventional Commit：

1. `docs(platform): 记录跨平台能力矩阵与实施契约`
2. `refactor(platform): 建立平台能力模型和会话检测`
3. `fix(build): 隔离 Linux 专用模块和依赖`
4. `fix(build): 改为跨平台前端构建命令`
5. `fix(storage): 使用系统图片目录保存文件`
6. `fix(shortcut): 按平台选择快捷键后端`
7. `fix(paste): 增加非 Linux 自动粘贴后端与降级`
8. `fix(capture): 建立 Ubuntu 22 兼容截图路径`
9. `feat(platform): 增加 Windows 窗口与截图后端`
10. `feat(platform): 增加 macOS 权限与截图后端`
11. `ci(platform): 增加三平台构建与测试矩阵`
12. `docs(platform): 记录实机验收结果和系统边界`

未经对应平台验证的提交不得使用“完成 Windows/macOS 支持”措辞。
