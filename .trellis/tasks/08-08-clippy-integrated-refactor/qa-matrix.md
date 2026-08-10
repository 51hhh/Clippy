# 综合重构 QA 矩阵

更新日期：2026-08-10

## 自动化证据

| 范围 | 命令/证据 | 结果 |
|---|---|---|
| Rust 格式 | `cargo fmt -- --check` | 通过 |
| Rust 编译 | `cargo check --all-targets` | 通过 |
| Rust 测试 | `cargo test` | 68 passed |
| Rust lint | `cargo clippy --all-targets -- -D warnings` | 通过 |
| 前端类型 | `npx tsc --noEmit` | 通过 |
| 前端测试 | `npx vitest run` | 22 files / 363 passed |
| 前端构建 | `npx vite build` | 通过，5 个窗口入口均生成 |
| X11/DOM smoke | `./scripts/smoke-dom.sh`（外部 Xvfb 权限） | 6 passed |
| npm 依赖安全 | `npm audit --json` | 0 vulnerabilities |
| deb | `cargo tauri build` + `dpkg-deb --info/--contents` | 5.0 MiB，版本/依赖/desktop/bin 正确 |
| AppImage | `cargo tauri build --bundles appimage --no-sign --ci` + `file` 检查 | 82 MiB，x86-64 ELF，未签名 |

产物校验：

- deb SHA-256: `c75d9e1edcc0e0895f157fc4e9578912cb46f2d54d29d1490d46fcfa4023a8a1`
- AppImage SHA-256: `3eb0ef5497941996be9775ea38f84a7761bc7ed53ca626d5ae9da4b79f07ea8f`
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
