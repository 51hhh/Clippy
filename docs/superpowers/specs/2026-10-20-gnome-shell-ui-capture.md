# GNOME Shell 系统界面截图修复

需求 ID：`GNOME-SHELL-UI-CAPTURE-01`

## Goal

在 GNOME Wayland 中使用 Clippy 截图快捷键时，能够保留按键瞬间可见的日历、状态菜单、
应用托盘菜单和 Activities/应用概览，而不是只截到它们背后的普通应用窗口。

## Requirements

1. 截图快捷键在 GNOME Shell 的 `NORMAL`、`OVERVIEW` 与 `POPUP` action mode 可用。
2. `OVERVIEW`/`POPUP` 中先由 Shell 冻结当前 stage，再通知 Clippy 创建覆盖层。
3. 普通桌面继续使用 Mutter PipeWire 首帧，不为系统界面修复承担整屏 PNG 编码开销。
4. 扩展不可用、过期或调用失败时保留原 gsd-media-keys + D-Bus 降级路径。
5. Shell 扩展与 gsd-media-keys 不得同时占用 Capture 快捷键。
6. 预冻结截图必须位于私有 runtime 目录、一次消费，并在应用不可用时限时清理。
7. 登录、锁屏、解锁和系统认证 action mode 不得触发该快捷键；解锁回到
   普通会话后必须自动恢复。

## Acceptance Criteria

- 打开顶栏日历后按 Capture 快捷键，Clippy 覆盖层底图仍包含日历。
- 打开应用托盘菜单后按 Capture 快捷键，底图仍包含菜单。
- 在 Activities 与应用网格中按 Capture 快捷键，底图仍包含当前概览。
- 双屏混合缩放下，预冻结舞台可按现有整舞台几何规则切为每屏冻结帧。
- 普通应用窗口截图日志仍显示 PipeWire 路径。
- 扩展 v5 或未安装时，Capture 快捷键仍由 gsd-media-keys 触发。
- 暂停、恢复和修改快捷键不会形成双触发；冲突时恢复旧绑定。
- 锁屏并解锁后快捷键自动恢复；卸载扩展后原 gsettings 绑定立即恢复。
- 扩展停用、应用未运行或十秒内未消费时，不遗留预冻结截图。

## Out of Scope

- 把 Shell actor 暴露为窗口速选候选；本次只保证像素可见，系统菜单仍需手动框选。
- 捕获锁屏、解锁、登录或系统认证对话框。
- 改变 KDE、wlroots、Windows 或 macOS 的截图快捷键实现。
- 为系统界面预冻结路径实现新的无压缩跨进程纹理传输；普通路径性能保持由 PipeWire 保证。
