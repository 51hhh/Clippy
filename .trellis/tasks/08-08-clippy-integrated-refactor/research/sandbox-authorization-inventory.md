# 沙箱授权与无人值守清单

## 已完成

- `git pull --ff-only`：已更新 Flashot 到 `23f16b5` (`v0.7.1`)。
- `git pull --ff-only`：已更新 Translator 到 `a8ac6cc` (`v0.3.2`)。
- `curl`：已读取 XDG Desktop Portal 官方规范和 RustDesk 官方源码。
- `gdbus call`：已只读确认当前 RemoteDesktop Portal v2、设备位图 7、ScreenCast Portal v5。
- `npm ci` / `npm install` / `npm audit`：已获外部执行权限，更新 DOMPurify 与开发工具链并移除未使用的 sharp；完整依赖审计为 0。
- `xvfb-run`：已获 GUI 外部权限，DOM 入口 smoke 6 项通过。
- `cargo tauri build`：已获打包权限，deb/AppImage 成功生成并检查；本地无 updater 私钥，签名留给 release CI secret。

## 已授权但按需执行

以下命令会在后续实现或验证阶段访问网络、桌面会话或 GUI，应在进入无人值守前取得授权：

1. `npm ci` / `npm install`：仅按 lockfile 恢复或更新明确依赖。
2. `cargo tauri build`：构建 Linux deb/AppImage。
3. `xvfb-run`：在隔离 X11 服务器运行 smoke test。

`cargo tauri dev` 会连接真实桌面会话；当前没有可自动确认 Portal 弹窗的授权，因此不在无人值守阶段调用。

## 不在无人值守阶段触发

- Portal 首次 RemoteDesktop 授权弹窗：必须由用户明确确认，不能自动代答。
- 安装 root/uinput 服务、修改用户组、Polkit 规则或系统配置：当前 PRD 明确不采用。
- 打开外部浏览器或桌面设置页面：只作为最终人工 QA 指引。

## 已有无需重复授权的规则

- `git pull`
- `curl`
- `npm test`
- `git add`
- `git commit`

## 执行约束

- 后续自动实现不得调用会阻塞等待桌面弹窗的 Portal Start 流程。
- Portal 功能使用 mock/trait 测试；真实首次授权留在最终人工矩阵。
- 新依赖下载应集中在授权阶段完成，之后尽量使用 `--offline` 验证可复现性。
