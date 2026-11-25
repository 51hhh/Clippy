# GNOME Shell 系统界面截图修复

需求 ID：`GNOME-SHELL-UI-CAPTURE-01`

状态：**已于 2026-10-23 撤回，未完成。** v6 在复杂 4K 桌面把整个 stage 编成 PNG 后才通知
应用，实测仅这一步就需约 2.7–3.8 秒，不符合截图工具的响应要求。本文保留为已撤回设计记录，
不代表当前实现；现行取舍见
[`GNOME-CAPTURE-FAST-PATH-02`](2026-10-23-gnome-capture-fast-path-rollback.md)。

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
8. 应用必须在 D-Bus 应答前原子认领截图模式；拒绝的请求立即清理预冻结图，后继动作不得
   消费十秒窗口内的旧画面。
9. 卸载运行中的扩展必须得到 Shell 明确停用应答；停用失败时保持原安装状态，不得同时恢复
   gsettings 绑定形成双重占用。
10. 扩展状态、安装与卸载中的 D-Bus、gsettings 和进程等待不得阻塞 Tauri UI 主线程。
11. 应用运行期间从系统扩展页单独停用或重新启用扩展时，Capture 键所有权必须自动在
    gsd-media-keys 与 Shell 之间切换；锁屏导致的临时停用不得被误判为用户停用。
12. `GetVersion` 返回高于当前应用支持上限的协议时仍须视为“有运行实例”；应用不得调用未知
    协议能力、覆盖新版扩展文件，或重新启用可能与新版扩展冲突的 gsettings Capture 绑定。

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
- 截图/录屏模式已占用时，Shell 收到拒绝应答且预冻结图立即删除；随后从动作启动器截图不会
  读到被拒绝请求的画面。
- `DisableExtension` 返回 false、错误或停用后仍应答时，卸载命令失败且不删除扩展文件、
  不恢复 gsettings Capture 绑定。
- 设置页安装、卸载和状态检查在 blocking worker 执行，外部命令超时不冻结窗口事件循环。
- 应用保持运行时，在系统扩展页停用扩展后 gsettings Capture 绑定自动恢复；重新启用后 Shell
  自动接管且 gsettings 绑定重新清空，全程无需重启 Clippy。
- 锁屏期间扩展实例被临时销毁时不恢复 gsettings；解锁后的新实例通知应用重新确认 Shell 所有权。
- 模拟高于内嵌上限的运行协议时，状态仍显示 active、卸载仍要求明确停用应答，快捷键进入
  Preserve 模式且启动自检不覆盖扩展文件。

## Out of Scope

- 把 Shell actor 暴露为窗口速选候选；本次只保证像素可见，系统菜单仍需手动框选。
- 捕获锁屏、解锁、登录或系统认证对话框。
- 改变 KDE、wlroots、Windows 或 macOS 的截图快捷键实现。
- 为系统界面预冻结路径实现新的无压缩跨进程纹理传输；普通路径性能保持由 PipeWire 保证。
