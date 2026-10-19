# UI Shell 与主题一致性

Requirement ID：`docs/superpowers/specs/2026-10-19-ui-shell-theme-unification.md`

## Goal

让主要应用窗口复用同一套可访问、可主题化的窗口壳层，并使录屏库与贴图工作区真实呈现全部六套主题。

## Requirements

- 提供由 CSS 变量驱动的公共 Window Shell / Titlebar 原语，Vanilla 与 React 页面都能复用，不把 Tauri API 越过 `api.ts` facade 暴露给业务代码。
- 设置窗口改为无原生装饰的可调整大小窗口；顶栏支持拖动、双击切换最大化、最小化、最大化/还原和关闭。
- 设置窗口关闭按钮必须走既有 `close_settings` 恢复快捷键再销毁的流程，失败时保留窗口与输入内容。
- Launcher、Viewer、录屏库和贴图工作区复用公共壳层/标题栏样式，同时保留各自布局和业务交互。
- 录屏库和贴图工作区从 `themes.css` 消费语义变量；light、dark、nord、solarized-light、rose、midnight 六套主题都不得退回硬编码浅色。
- 为窗口壳层交互、API facade 和主题接线增加自动化回归测试，并记录 Linux 本地验证与尚未执行的平台边界。

## Acceptance Criteria

- [x] 设置窗口由 Tauri 以 `decorations(false)` 创建，页面不存在重复的内容区一级标题。
- [x] 设置顶栏的拖动、双击最大化、最小化、最大化/还原和关闭均有自动化合同；关闭仍调用 `close_settings`。
- [x] Launcher、Viewer、录屏库和贴图工作区使用公共 `window-shell.css` 原语，重复的关闭按钮与标题栏基础样式不再各自维护。
- [x] 录屏库和贴图工作区导入 `themes.css`，其样式不再用仅匹配 `data-theme="dark"` 的分支模拟主题。
- [x] 六个主题 ID 有自动化静态合同，录屏库和贴图工作区的前景、背景、边框、控件及状态色均由主题变量或这些变量的 `color-mix()` 派生。
- [x] `cd src && npx tsc --noEmit`、相关 Vitest 和 `./scripts/ci-local.sh` 全部通过；任何跳过项单独记录，不计为通过。
- [ ] Linux 实机验证设置窗口拖动、缩放、最小化、最大化、关闭以及六主题切换；Windows/macOS 未执行时明确保留为未完成。

## Out of Scope

- 不改变截图覆盖层、长截图引导层、贴图画布和录屏控制浮层的高对比工具风格。
- 不在本需求中重构 D-Bus/zbus runtime、WebKit 生命周期或主窗口懒创建策略。
- 不改变设置项语义、窗口尺寸、快捷键恢复协议和数据持久化格式。
- 不以 CSS 拆包结果宣称运行内存下降；资源优化必须在独立需求和分支中测量。

## Decision

**Context**：当前已有六主题 token，但只有 main/settings/launcher/viewer 完整消费；录屏库与贴图工作区各自维护硬编码 light/dark 样式。设置页仍使用原生标题栏，而 Viewer、Launcher 和两个库窗口已经使用四种互不兼容的自定义标题栏。

**Decision**：新增框架无关的 `styles/window-shell.css`，把窗口壳层、标题栏、标题副本和窗口控制按钮定义为稳定 CSS 原语。Vanilla 设置页使用小型 `js/window-chrome.js` 绑定原生窗口动作，React 窗口只复用类名和现有 service adapter，不引入跨框架运行时组件。录屏库与贴图工作区直接消费六主题语义 token。

**Consequences**：顶栏视觉和可访问状态集中维护，主题能覆盖主要文档窗口；各页面仍可通过 CSS 变量保留不同高度和内容布局。无边框设置窗需要三平台真实桌面 QA，尤其是拖动、最大化与系统窗口行为。

## Validation

- `npx vitest run`：81 个文件通过、2 个文件跳过；1363 项通过、36 项跳过。Canvas 的 jsdom
  未实现提示不计失败，跳过项也不计通过。
- `npx tsc --noEmit`、`npm run build`、`cargo fmt -- --check`、`cargo check` 与
  `cargo build --release --bin clippy-app` 已通过。完整 `./scripts/ci-local.sh` 为 25 通过、0 失败、
  2 跳过；非宿主交叉 lint 与 AppImage 可视 smoke 未启用，不计为通过。
- 隔离 Xvfb/D-Bus 中启动真实 release 二进制，通过真实托盘打开设置页；窗口树确认内尺寸
  720×560，截图确认只有自绘顶栏。逐项选择六个主题后，配置文件与 WebView 均呈现对应主题；
  关闭控件销毁窗口并退出该页的 WebKit WebProcess。
- Xvfb 没有窗口管理器，不能验证拖动、缩放、最小化或最大化的真实桌面行为；Linux 真实桌面、
  Windows 与 macOS 的窗口行为继续保留为未完成。
