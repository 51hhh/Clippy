# 自动粘贴授权调研

## 当前实现

- `select_clip` 写入剪贴板、隐藏主窗口、等待 100ms，再调用 enigo X11/XTest 模拟 `Ctrl+V`。
- enigo 仅启用 `x11rb` feature；Wayland 会话中存在 `DISPLAY` 时实际连接的是 XWayland，不能可靠控制原生 Wayland 应用。
- 当前仓库没有 RemoteDesktop Portal 会话，因此现有代码本身不会产生 RemoteDesktop 授权弹窗。
- 当前代码没有记录和显式恢复唤起 Clippy 前的活动窗口。

## XDG Portal v2

- RemoteDesktop `SelectDevices` 支持 `persist_mode`：0 不保存、1 应用存活期间保存、2 直到显式撤销。
- `restore_token` 是一次性 token；恢复后必须保存 `Start` 返回的新 token。
- 恢复失败、设备变化或权限撤销时，Portal 会忽略 token 并正常询问用户。
- ashpd 0.13.12 已提供 `SelectDevicesOptions::set_persist_mode`、`set_restore_token` 和 `SelectedDevices::restore_token`。
- 当前项目需为 ashpd 增加 `remote_desktop` feature。

## RustDesk 对照

- RustDesk 常驻服务存在时主要走 uinput 输入与 ScreenCast restore token。
- 非服务模式才使用 RemoteDesktop Portal 注入输入。
- RustDesk 当前源码对 RemoteDesktop Portal 持久 token 仍保留 TODO；“始终授权”体验不能直接等同于普通应用 Portal 恢复。

## 推荐设计

- `PasteBackend` trait：`paste()`、`status()`、`request_permission()`、`shutdown()`。
- `X11PasteBackend`：记录 `_NET_ACTIVE_WINDOW`，隐藏主窗口后请求 WM 激活原窗口，确认焦点，再用 XTest 发送按键。
- `WaylandPortalPasteBackend`：首次显式启用时创建会话；应用存活期间复用；启动时最多恢复一次。
- `CopyOnlyPasteBackend`：始终可用，不注入按键。
- token 使用 Secret Service/keyring 或独立 0600 文件保存；不得进入普通 AppConfig。
- 每个按键序列使用释放保护，确保错误路径释放 Control 键。

## 兼容性边界

- Portal interface v2 是能力条件，不代表所有后端都一定静默恢复。
- X11 无需 Portal 授权；若真实 X11 仍弹窗，应确认弹窗所属进程和运行二进制版本。
- 最终必须在 GNOME Wayland、GNOME Xorg 和至少一个 KDE 环境进行手工验证。

