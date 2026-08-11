# 综合重构 QA 矩阵

更新日期：2026-08-11

## 自动化证据

| 范围 | 命令/证据 | 结果 |
|---|---|---|
| Rust 格式 | `cargo fmt -- --check` | 通过 |
| Rust 编译 | `cargo check --all-targets` | 通过 |
| Rust 测试 | `cargo test --lib -- --skip libre_provider_completes_a_loopback_http_request --skip openai_provider_sends_bearer_auth_over_loopback_http` | 73 passed；2 项 localhost mock 因当前沙箱禁止监听端口而过滤，见独立回环证据 |
| Rust lint | `cargo clippy --all-targets -- -D warnings` | 通过 |
| 本地敏感文件权限 | Rust Unix 回归测试 | `config.json`、`clips.db`、`-wal`、`-shm`、Portal token 均为 `0600`；旧配置/数据库宽松权限可修复 |
| 前端类型 | `npx tsc --noEmit` | 通过 |
| 前端测试 | `npx vitest run` | 22 files / 363 passed |
| 前端构建 | `npx vite build` | 通过，5 个窗口入口均生成 |
| X11/DOM smoke | `./scripts/smoke-dom.sh`（外部 Xvfb 权限） | 6 passed |
| Release X11 startup | release binary + `dbus-run-session` + `xvfb-run`，临时 HOME，12 秒超时 | watcher、SQLite/config、X11 快捷键初始化；无提前崩溃（不等同视觉验收） |
| 翻译 provider 回环集成 | `cargo test translation::service::tests`（本地临时 TCP mock） | 9 passed，覆盖 Libre/OpenAI 路径、请求体和认证头 |
| npm 依赖安全 | `npm audit --json` | 0 vulnerabilities |
| deb | `cargo tauri build` + `dpkg-deb --info/--contents` | 5.0 MiB，版本/依赖/desktop/bin 正确 |
| AppImage | `cargo tauri build --bundles appimage --no-sign --ci` + `file` 检查 | 82 MiB，x86-64 ELF，未签名 |

产物校验：

- deb SHA-256: `3c54f457471dc329ec3a88666992dc2dfb522dc59af297f455507216c756aa3c`
- AppImage SHA-256: `045d7dd3ea92253ef65bb6ac97c5272b945b876e27887e2f34fe7539f55c1e37`
- 本地未配置 `TAURI_SIGNING_PRIVATE_KEY`，所以 updater 签名未生成；release workflow 已从 GitHub Actions secret 注入签名密钥。

## 真实桌面人工矩阵

下列项目不能由无交互沙箱代替。当前结果明确标为“待真实桌面验证”，不能由单元测试或 Xvfb 推断为通过。

| 场景 | 验收点 | 当前结果 |
|---|---|---|
| GNOME X11 | 原窗口恢复、Ctrl+V、无 Portal 弹窗、主页面非黑屏 | 待真实桌面验证 |
| GNOME Wayland | 首次授权、同进程会话复用、Copy-only fallback | 待真实桌面验证 |
| GNOME Wayland 重启 | restore token 滚动更新、静默恢复或后端提示 | 待真实桌面验证 |
| Portal 撤权 | 仅一次失败、设置页显式重试、剪贴板仍保留 | 待真实桌面验证 |
| KDE Wayland | Portal 后端兼容、覆盖层、Pin 置顶 | 待真实桌面验证 |
| deb 实装 | 主窗口渲染、托盘、截图、Pin、设置 | 未修改系统，待实装验证 |
| AppImage 实机 | 主窗口渲染、托盘、截图、Pin、更新器 | 待真实桌面验证 |
| Secret Service | API key 保存/查询/删除且配置文件无明文 | 待真实服务验证 |
| LibreTranslate-compatible | 实际请求、超时、复制结果 | 需要可用测试端点 |
| OpenAI-compatible | 实际请求、模型/key、结构化错误 | 需要可用测试端点与 key |

## 人工操作边界

- Portal 首次确认和撤权必须由用户操作，自动化不得代答。
- 不安装 root/uinput 常驻服务，不修改用户组、Polkit 或系统安全策略。
- deb 实装会修改系统状态，因此未在无人值守沙箱内执行。
