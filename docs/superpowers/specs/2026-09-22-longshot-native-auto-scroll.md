# PX-LS-NATIVE-AUTO-01 — Windows / macOS 长截图自动滚动

日期：2026-09-22

关联需求：`PX-LS-2D-01`、`PX-LS-AUTO-01`

## Goal

在已经稳定的二维长截图会话上，为 Windows 与 macOS 增加受控的上下左右自动滚动。每一步都必须
命中冻结选区下的同一个原生窗口，检测用户接管鼠标，随后沿用现有重捕获、重叠估计和原子提交；
不能因为输入 API 返回成功就跳过图像质量门。

## Requirements

1. Windows 与 macOS 只在真实原生后端可用时声明四方向自动滚动。macOS 未授予辅助功能权限时
   返回 `permission_required`，不渲染可执行按钮；Wayland 继续返回
   `wayland_remote_desktop_required`，不得借 XWayland 或普通合成键伪装支持。
2. 滚动点只能由后端从冻结帧的物理裁剪区反算，前端不能提交桌面坐标。控制窗隐藏并完成 settle
   后才锁定该点下的目标窗口；锁包含原生窗口 ID 与进程 ID，后续每一步前后都必须一致。
3. Windows 使用 `GetPhysicalCursorPos` / `SetPhysicalCursorPos` 处理含负坐标的虚拟桌面，使用
   `WindowFromPhysicalPoint` + `GetAncestor(GA_ROOT)` 锁定顶层窗口。截图层的逻辑显示器原点必须按该屏
   `scale_x/scale_y` 恢复为物理虚拟桌面坐标，再叠加物理裁剪中心；不能把 macOS/X11 的逻辑点
   换算直接用于 Windows。滚动前检查目标进程完整性级别，避免 `SendInput` 被 UIPI 静默拒绝；
   不得使用 Enigo 当前只按主显示器映射的绝对移动实现。
4. macOS 使用 Core Graphics 的全局显示坐标读取和移动指针，使用一次窗口列表快照按 z-order
   选择包含滚动点的普通窗口。窗口枚举与输入都不得依赖 AppKit 主线程；实际滚动前再次检查
   Accessibility 权限。
5. 每一步按“保存原指针 → 移到滚动点 → settle → 锁定/复核窗口 → 注入一格滚动 → settle →
   复核指针与窗口 → 重捕获 → 再复核指针”的顺序执行。任一检查失败都不提交新帧。
6. 用户在步骤中移动鼠标时返回 `longshot_auto_user_interrupted`，并保留用户的新位置；其它失败与
   正常完成恢复步骤开始前的指针位置。这样停止自动流程不会与用户争抢鼠标。
7. 上下左右仍使用同一 `LongshotAutoDirection` wire 值和同一二维拼接质量门。目标窗口变化、权限
   撤销、输入失败、动态内容无可靠重叠与页面到底均暂停自动循环，保留已提交结果供手动恢复或导出。
8. TypeScript IPC 联合类型、控制窗提示、中英文文案、路线文档和 CHANGELOG 必须与后端能力一致。

## Research Basis

- Microsoft `WindowFromPhysicalPoint` 明确以物理点返回命中窗口；再取 `GA_ROOT` 可把子控件归一到
  稳定的顶层窗口：
  <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-windowfromphysicalpoint>
- Microsoft `GetPhysicalCursorPos` / `SetPhysicalCursorPos` 明确读写物理桌面坐标，避免 DPI 感知上下文
  改变普通 cursor API 的解释：
  <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getphysicalcursorpos>、
  <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setphysicalcursorpos>
- Microsoft 明确说明 `SendInput` 受 UIPI 限制，并且返回值和 `GetLastError` 都不会指出失败是否由
  UIPI 引起，因此必须在注入前比较进程完整性级别：
  <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput>
- Apple 的 Quartz Window Services 可返回当前用户会话的窗口 ID、边界与管理信息：
  <https://developer.apple.com/documentation/coregraphics/cgwindowlistcopywindowinfo(_:_:)>。
- Apple `AXIsProcessTrustedWithOptions` 的提示是异步的，调用返回值仍只是当前信任状态；能力响应必须
  以无弹窗预检为准：
  <https://developer.apple.com/documentation/applicationservices/1459186-axisprocesstrustedwithoptions>

## Acceptance Criteria

- [x] 纯策略测试覆盖 X11、Wayland、Windows、macOS 已授权/未授权与其它平台能力矩阵。
- [x] 目标锁测试覆盖首步锁定、同窗口复用、窗口 ID 或 PID 变化、空目标和 Clippy 自身窗口拒绝。
- [ ] Windows/macOS 原生构建通过 check、clippy 与 tests；默认 Linux 完整门禁保持通过。
- [x] 控制窗在 macOS 权限缺失时只显示准确说明，不显示可执行自动滚动按钮；授权后四方向保持可选。
- [ ] Windows 双屏负坐标/混合 DPI 与 macOS Retina/外接屏真机验证指针命中、用户中断、目标切换、
  上下左右滚动、页面到底和动态内容失败保留。
- [ ] X11 既有自动滚动回归通过；Wayland 能力仍关闭并明确记录 RemoteDesktop/libei 后续工作。

## Verification

- 2026-09-22 Linux x86_64：`./scripts/ci-local.sh` 为 25 通过、0 失败、2 跳过；Rust 为
  1104 通过、14 ignored，前端为 73 个文件、1270 项通过，X11 录屏与剪贴板隔离回归、三项
  DOM/Canvas/Layout smoke、Vite 构建均通过。
- Windows 交叉 `cargo check` 因当前 Linux 主机没有 Windows SDK 头文件，停在第三方 `ring`
  的 `assert.h`，没有进入 Clippy 条件分支，不能记为 Windows 构建证据。
- Windows/macOS 原生 check、clippy、tests 与多显示器真机 QA 仍待同一提交的远程 Native Check
  和人工验收；未完成项继续保留在 Acceptance Criteria。

## Out of Scope

- Wayland RemoteDesktop/libei 授权会话、Portal 恢复 token 与合成器专项实现。
- 触控板惯性、像素级平滑滚动、自动寻找滚动容器、跨窗口继续或绕过高完整性窗口。
- 用固定延时宣称所有页面稳定；内容是否可拼接仍由现有图像质量门决定。
- 以 Windows/macOS 本地编译替代真实多显示器、混合 DPI 与权限真机验收。
