# Linux 设置页 WebProcess 复用

Requirement ID：`LINUX-SETTINGS-WEBPROCESS-01`

## Goal

在不改变设置窗口按需创建/关闭语义的前提下，让 Linux 设置页与常驻主窗口复用同一个
WebKit WebProcess，降低打开设置页时的进程组 PSS 增量，并保留可回退的非 Linux 路径。

## Requirements

- 仅在 Linux 使用 Tauri `WebviewWindowBuilder::with_related_view`；Windows 与 macOS 继续走
  原建窗路径，不新增条件编译依赖。
- 主窗口存在时，从其平台 WebView 在主线程创建设置窗口；标题、尺寸、最小尺寸、可调整大小、
  无原生装饰和受保护关闭协议保持不变。
- 主窗口缺失时安全回退到独立 WebProcess，避免恢复/启动异常把设置入口变成永久不可用。
- 跨线程调度必须有界；调度失败或超时返回明确错误，不得在未知建窗结果后再创建第二个设置窗口。
- 用同一 release 构建、同一隔离 X11/D-Bus 流程分别记录设置页关闭、打开、再次关闭时的
  进程数与进程组 PSS；RSS/VmSize 仅作诊断，不作为收益结论。
- 用户可见变更、风险和未验证平台写入 `CHANGELOG.md` 与架构文档。

## Acceptance Criteria

- [x] Linux 主窗口存在时，设置页窗口树仍为 720×560，且打开前后不新增第二个
  `WebKitWebProcess`。
- [x] 设置页可加载、切换主题并通过受保护的关闭按钮销毁；关闭后主 WebProcess 继续存活。
- [x] 主窗口缺失回退、同步/异步调度成功、调度失败、超时和建窗错误有自动化合同。
- [x] 同一环境的设置页打开增量 PSS 低于未复用基线，并记录采样方法与数值；不根据单次 RSS
  推断跨设备绝对收益。
- [x] `./scripts/ci-local.sh` 为 0 失败；跳过项不计为通过。
- [x] Windows/macOS 条件编译代码与真实桌面行为未执行时明确保留为未完成。

## Remaining Validation

- [ ] Windows 与 macOS 同 SHA Native Check。
- [ ] Wayland、真实 Linux 窗口管理器与安装包桌面 QA。
- [ ] Windows/macOS 设置页真实创建、关闭和进程行为。

## Out of Scope

- 不销毁常驻主 WebView，不改变剪贴板监听、设置数据、快捷键恢复或窗口外观。
- 不让 Pin、截图、录屏、Viewer 或 Launcher 共享 WebProcess；这些窗口的隔离/崩溃边界另案评估。
- 不调整 WebKit MemoryPressureSettings、GPU 策略、malloc、SQLite、前端虚拟滚动或缓存预算。
- 不把 Xvfb 结果当作 Windows/macOS、Wayland 或真实窗口管理器验收。

## Decision

**Context**：隔离 release 基线中，设置页按需打开会增加一个独立 `WebKitWebProcess`；关闭后该
进程能完整退出，说明生命周期正确，但打开期间存在可观的进程级固定成本。当前 Tauri 2.10.3
已在 Linux 暴露 `with_related_view`，可让新 WebView 与主 WebView 使用同一 WebProcess。

**Decision**：只让设置页关联主 WebView。关联值不能跨线程发送，因此在 Tauri 的
`with_webview` 主线程回调内完成 builder 创建，并只把可发送的建窗结果传回调用方。主窗口缺失
直接走原 builder；已派发但未返回的请求不得回退重建，以免迟到结果产生重复窗口。

**Consequences**：打开设置页不再承担第二个 WebProcess 的固定成本，但主窗口与设置页在 Linux
共享崩溃边界和进程调度。设置页本身仍在独立 WebView/DOM/IPC caller 中，关闭后其页面资源仍释放。

验证数值、热缓存留存与环境边界见
`docs/reviews/2026-10-19-linux-settings-webprocess-sharing-review.md`。
