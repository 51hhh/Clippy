# 修复开机自启动幽灵进程导致正常启动失败

## 现象

v0.1.6 引入开机自启动后:开机时 systemd-user 拉起一个 clippy 进程,但该进程
- 状态栏不显示托盘图标
- 不响应全局快捷键
- 但已占用 D-Bus 上 `com.clippy.app.SingleInstance` / `com.clippy.app` / `com.clippy.app.Shortcuts` 三个 well-known name

用户再次手动启动时,正常进程被 single-instance 检测拦截,转发 args 给幽灵进程失败,
报"无法连接到 local"。

## journalctl 实证根因

1. **autostart 路径污染**:`~/.config/autostart/Clippy.desktop` 的 `Exec=` 写入了
   `/home/rick/desktop/Clippy/src-tauri/target/debug/clippy-app`(开发期间点过 toggle
   留下的脏数据)。`tauri-plugin-autostart` 用 `current_exe()` 直接拍下当时的二进制路径。
2. **GNOME 双 desktop entry 触发并发启动**:postinst.sh 把 `com.clippy.app.desktop`
   软链到 `Clippy.desktop`,GNOME 仍按 desktop 文件路径各启一份 → 第二份 zbus 抢
   `com.clippy.app.Shortcuts` 失败,journal 中频繁出现:
   ```
   [ERROR clippy_lib] D-Bus 服务启动失败: name already taken on the bus
   ```
3. **失败不退出**:`lib.rs:204-208` `tauri::async_runtime::spawn` 里 D-Bus 服务启动
   失败仅 `log::error!`,进程继续驻留,造成幽灵实例。

## 修复方案

### 1. D-Bus 抢名失败必须 fatal(根因止血)

`gsettings_shortcuts.rs::start_dbus_service` 失败时不能驻留 —— 它的失败 = 已有实例
在跑,当前进程继续活着只会变成幽灵。改为通过 channel 把首次启动结果回传给 `lib.rs`,
失败立即 `app.exit(1)` 让 GNOME / single-instance 自动清理。

### 2. 防止 dev 二进制污染 autostart

在前端自启 toggle handler 中,检测 `current_exe` 路径,若包含 `target/debug` 或
`target/release/clippy-app`(非安装路径),禁用 toggle 并提示用户"请使用安装版本"。
后端新增 `is_installed_binary` IPC 命令辅助检测。

启动时若发现旧 autostart 文件指向不存在或非当前安装路径的二进制,自动调用
`disableAutostart` 清理。

### 3. GNOME 双图标问题(已知,本次顺手修)

postinst.sh 创建的 `com.clippy.app.desktop` 软链改为**重命名安装的 desktop 文件**:
直接安装一份 `com.clippy.app.desktop`(去掉 `Clippy.desktop`),通过 `desktopTemplate`
改文件名,避免 GNOME 同时识别两个 entry。

### 4. setup 阶段守护

在 `lib.rs` setup 起手处,检查 `current_exe` 是否合法的安装路径(非 debug、非
target/release dev 产物);若开机自启动起来的进程检测到自身路径异常,自动注销
autostart 并退出。

## 不做的事

- 不改 `tauri_plugin_single_instance` 的 identifier(`com.clippy.app` GTK app id 是
  另一回事,不需要联动)。
- 不引入 `X-GNOME-Autostart-Delay`(治标不治本,根因不是时序而是路径污染 + 失败不退出)。

## 验收

- [ ] 设置页 toggle 自启,重启系统后 `~/.config/autostart/Clippy.desktop` 的 Exec
      指向 `/usr/bin/clippy-app`(deb 安装路径),不会写入 dev 路径。
- [ ] 开机后 systemd-user 拉起的 clippy 进程必须托盘可见、快捷键可用 —— 否则它必须
      自杀,而不是驻留为幽灵。
- [ ] D-Bus name 抢占失败时进程立即退出,journal 中不再出现"name already taken"
      之后还能看到该进程其他日志的情况。
- [ ] GNOME 桌面环境下不再并发启动两份 Clippy(`Started app-gnome-Clippy-*` 和
      `Started app-gnome-com.clippy.app-*` 不再同时出现)。
- [ ] dev 模式(`cargo tauri dev`)下尝试开启自启,前端给出明确禁用提示。
