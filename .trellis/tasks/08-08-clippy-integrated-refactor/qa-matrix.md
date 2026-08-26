# 综合重构 QA 矩阵

更新日期：2026-08-13

## 自动化证据

| 范围 | 命令/证据 | 结果 |
|---|---|---|
| Rust 格式 | `cargo fmt -- --check` | 通过 |
| Rust 编译 | `cargo check --all-targets` | 通过 |
| Rust 测试 | `cargo test --all` | 100 passed；包含截图动作错误/竞态清理、Portal token 状态机与翻译 provider 回环测试 |
| Rust lint | `cargo clippy --all-targets -- -D warnings` | 通过 |
| 本地敏感文件权限 | Rust Unix 回归测试 | `config.json`、`clips.db`、`-wal`、`-shm`、Portal token 均为 `0600`；旧配置/数据库宽松权限可修复 |
| 前端类型 | `npx tsc --noEmit` | 通过 |
| 前端测试 | `npx vitest run` | 25 files / 408 passed |
| 前端构建 | `npx vite build` | 通过，5 个窗口入口均生成 |
| X11/DOM smoke | `./scripts/smoke-dom.sh` | 1 file / 7 passed（Xvfb） |
| Release X11 startup | 最终 AppImage 解包后的 `AppRun` + `dbus-run-session` + `xvfb-run`，临时 HOME/XDG，12 秒超时 | 进程持续运行至预期超时，无提前崩溃；无完整桌面环境产生的 PipeWire/EGL/user-systemd 警告不等同视觉验收 |
| 截图动作生命周期 | Rust 单元测试 | crop/action 失败会精确结束本代会话、关闭覆盖层并恢复源窗口；并发取消/双动作无法在失去会话所有权后继续产生副作用 |
| 翻译 provider 回环集成 | `cargo test translation::service::tests`（本地临时 TCP mock） | 9 passed，覆盖 Libre/OpenAI 路径、请求体和认证头 |
| npm 依赖安全 | `npm audit --json` | 0 vulnerabilities |
| deb | `cargo tauri build --bundles deb,appimage --no-sign --ci` + `dpkg-deb --info/--contents` | 5,195,008 bytes，版本/依赖/desktop/bin 正确 |
| AppImage | `scripts/finalize-appimage.sh` + `scripts/smoke-appimage-x11.sh` + 最终文件独立 SquashFS 解包/`ldd` 检查 | 85,117,432 bytes，x86-64 ELF，依赖无缺失，镜像内 `.DirIcon -> Clippy.png`，X11 smoke 通过，本地未签名 |

产物校验：

- deb SHA-256: `9ebb8e26cd15210ce0ddf0eb8c386b113d3532c1d8a7db4804298f41eb2f3f48`
- AppImage SHA-256: `026e4cc2b2c40de1467cee7abef318d1e1b54f0c465224d02aa832c78128fd43`
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
