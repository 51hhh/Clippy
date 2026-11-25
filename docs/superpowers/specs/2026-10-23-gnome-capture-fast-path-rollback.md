# GNOME 截图 v5 快速路径回退

需求 ID：`GNOME-CAPTURE-FAST-PATH-02`

## Goal

以按键后的快速响应为最高优先级，撤回 GNOME Shell 协议 v6 的整舞台 PNG 预冻结流程，恢复
v5 的 gsd-media-keys → D-Bus → Mutter PipeWire 路径。

## Requirements

1. 内嵌 GNOME Shell 扩展、元数据和 Rust 协议上限统一为 v5。
2. Capture 键由 gsd-media-keys 持有；扩展不注册 accelerator，也不导出或消费
   `SetCaptureShortcut` / `TakeShortcutScreenshot`。
3. Wayland 冻结帧首先尝试 Mutter PipeWire，不在应用收到请求前同步编码整舞台 PNG。
4. 当前会话若仍加载已知 v6，应用启动时调用其兼容接口释放 grab、删除
   `capture-shortcut`，再写回 v5 扩展并恢复 gsettings；完整脚本切换仍需重新登录。
5. 高于已知 v6 的未知运行协议不得被覆盖或抢占快捷键。
6. 窗口几何、逐屏/整屏截图兜底和贴图摆位能力继续保留。
7. 日历、状态/托盘菜单和 Activities 概览可能在外部命令启动时收起；当前版本明确接受该边界。

## Acceptance Criteria

- 扩展合同测试确认协议与 metadata 均为 v5，接口 XML 不包含两个 v6 快捷键方法。
- 后端顺序以 Mutter PipeWire 为首，不存在 pending Shell 快捷键 PNG 消费入口。
- 三个 GNOME 快捷键均写入 gsettings，Capture 不再被置空交给 Shell。
- 模拟未知高版本时保持其 Capture 所有权，不覆盖扩展文件。
- 真机从已加载 v6 的会话启动新版后，Capture 只触发一次且日志首先命中 Mutter PipeWire；
  锁屏解锁后仍可触发。
- 真机记录从按键到覆盖层可见的端到端耗时；目标回到现有普通桌面约 0.5–0.8 秒量级，最终数字
  以重建安装包的人工测量为准，未测不得写成通过。

## Out of Scope

- 在本次回退中继续保证 Shell 日历、菜单或概览像素可见。
- 实现冻结 Shell actor / texture lease 后再由 PipeWire 捕获的下一代低延迟方案。
- 改变 KDE、wlroots、Windows 或 macOS 的截图快捷键实现。
