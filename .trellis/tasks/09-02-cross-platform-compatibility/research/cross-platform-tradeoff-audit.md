# 跨平台兼容取舍审计

## 审计范围

- 对比 `621e36f289a386dcf84d26c77742f6b0f1eae31e..HEAD` 中的平台、打包、CI 与发布变更。
- 重点检查为兼容旧系统、通过构建或降低打包风险而关闭能力、降级后端或弱化验证的情况。
- 区分操作系统真实安全/协议边界与项目自行引入的能力回退。

## 确认的高风险回退

### Linux 正式包裁掉 Mutter PipeWire 快速路径

- `11d38dc` 将 `pipewire` 改为可选依赖，并以空默认 feature 关闭 `linux-pipewire`。
- `34ce947` 在构建与发布工作流中反向检查默认依赖图不得出现 `pipewire-rs`。
- 正式 Linux 产物因此从约 229ms 的 Mutter/PipeWire 原始帧路径退回 3–4 秒的 Shell PNG 路径。
- 这是构建环境约束错误地传播为运行时能力裁剪，必须恢复为默认且正式发布始终包含的能力。

## 发布门禁缺口

- Windows/macOS required check 主要覆盖 `cargo check`、Clippy、测试、打包与签名结构检查。
- macOS 没有启动 app/挂载 DMG 后的运行烟测，也不会自动验证 TCC、Pin Spaces 或截图链路。
- Windows 没有安装/启动后的原生粘贴与权限路径烟测。
- Linux AppImage 自动烟测目前只覆盖 Ubuntu 24 X11，不覆盖 Ubuntu 22/24 GNOME Wayland 截图延迟。
- `native-qa.yml` 生成的人工 QA 资产不是 release 的强依赖，不能阻止同一 SHA 上的运行时回归发布。

建议把同一提交 SHA 的原生启动证据和 Linux Wayland 截图性能证据纳入发版门禁；短期至少让默认、CI、release 使用同一套截图 feature，并对依赖图和后端优先级做正向断言。

## 不应误判为同级回退的事项

- 非 GNOME Wayland 的 always-on-top 限制来自 Wayland 协议边界；在没有目标桌面实证前不应强行宣称支持。
- AppImage 移除构建机的 `libwayland-*` 是为了避免旧 ABI 与宿主 Mesa/EGL 混载崩溃；应补 Wayland 运行验证，而不是重新捆绑这些库。
- Windows 高完整性窗口的 copy-only、macOS 屏幕录制/辅助功能权限属于系统安全边界。
- macOS Ad-Hoc 与 Windows 自签名是明确的分发信任限制，不会直接裁掉运行能力。
- OCR 依赖外部 Tesseract 是已公开的可用性边界，不是本次截图性能回退。

## 观察项

- Linux `xcap` 从 0.9 固定到 0.4.1 规避了其强制引入的新 PipeWire 绑定；现用 API 仍存在，但应补 Ubuntu 22/24 X11 运行与性能验证。
- 平台能力诊断目前把所有 Linux Wayland 截图都标记为 Portal permission required，与 GNOME Mutter/扩展实际路径不一致；这属于诊断准确性问题，不应再反向驱动功能裁剪。
