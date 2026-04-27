# CI 自动构建 + 自动更新 + Changelog 版本管理

## Goal

为 Clippy 添加完整的版本发布和自动更新体系，参考 fxxkDJTU 项目的实现模式。包含 4 个子系统：
1. GitHub Actions CI 流水线（构建 + 发布）
2. Tauri Updater 自动更新（检查 + 下载 + 安装）
3. Settings 页面版本号显示 + 检查更新按钮
4. CHANGELOG.md 规范 + CI 自动提取发布说明

## What I already know

### 当前状态
- Clippy 版本号 `0.1.0`，定义在 `tauri.conf.json`（`Cargo.toml` 和 `package.json` 也需同步）
- 无 CI/CD 配置，无 CHANGELOG.md
- Settings 页面有：快捷键录制、主题选择、历史上限、语言选择
- 前端走 Vite 构建（`npx vite build` → `../dist`）
- 构建目标仅 Linux：deb + AppImage
- Settings 窗口由 Rust 后端按需创建，不在 tauri.conf.json 中预声明

### fxxkDJTU 参考实现
- 使用 `tauri-plugin-updater`（Rust）+ `@tauri-apps/plugin-updater`（前端 JS SDK）
- `tauri.conf.json` 配置 updater: pubkey + endpoints（指向 GitHub Release latest.json）
- CI 用 `tauri-apps/tauri-action` 构建，`includeUpdaterJson: true` 自动生成 latest.json
- Changelog 格式：`## v0.1.0` + emoji 分类（✨新功能 / 🐞修复 / 🔧工程化）
- CI release job 用 awk 从 Changelog.md 提取对应版本内容
- 前端 UpdateDialog：正常更新 / 下载中 / 手动回退 三态
- 启动自动检查 + Settings 手动检查
- deb 不支持自动更新，提供手动下载回退

### 关键差异
- fxxkDJTU 是 Vue + TS，Clippy 是 vanilla JS
- fxxkDJTU 更新弹窗在 App.vue（Vue 组件），Clippy 需要 vanilla JS 实现
- Clippy 使用独立 Settings 窗口（Tauri 窗口），而非 SPA 路由页面

## Confirmed Decisions

- GitHub 仓库：`51hhh/Clippy`
- 更新弹窗：主窗口 modal（覆盖在剪贴板列表之上）
- 构建平台：仅 Linux（deb + AppImage）
- 使用 GitHub Releases 作为更新分发渠道
- 需要生成 updater 签名密钥对（用户在 CI secrets 中配置）

## Requirements (evolving)

### R1: CHANGELOG.md
- 项目根目录创建 `CHANGELOG.md`
- 格式：`## v{version}` 标题 + emoji 分类段落
- 初始版本 v0.1.0 记录已有功能

### R2: CI 流水线
- `release.yml`：tag `v*.*.*` 触发 → 版本校验 → Linux 构建（deb + AppImage）→ 提取 Changelog → 发布 Release
- `build.yml`：PR/push 触发 → cargo check + clippy + test + 前端 vitest
- `tauri-apps/tauri-action` 构建，`includeUpdaterJson: true`
- Rust 缓存（`Swatinem/rust-cache`）

### R3: Tauri Updater 集成
- Rust: `tauri-plugin-updater` 依赖 + lib.rs 注册
- 前端: `@tauri-apps/plugin-updater` 依赖
- tauri.conf.json: updater 配置（pubkey + endpoint）
- capabilities/default.json: 添加 `updater:default`
- `bundle.createUpdaterArtifacts: true`

### R4: 前端自动更新 UI（主窗口 modal）
- `app.js` 启动时自动检查更新
- 更新 modal：版本号 + Changelog 文本 + 操作按钮（跳过此版本/稍后提醒/立即安装）
- 下载进度展示（进度条 + 字节数）
- deb 回退：引导到 Release 页面手动下载
- "跳过此版本" localStorage 记忆
- modal HTML 结构在 index.html 中定义，JS 控制显隐
- 更新检查 API 封装在 api.js 中

### R5: Settings 页面版本号
- 底部"关于"区域显示 `v{version}`
- 检查更新按钮（手动触发）
- 更新状态反馈（已是最新 / 检查失败）

## Acceptance Criteria (evolving)

- [ ] `CHANGELOG.md` 存在且格式正确
- [ ] `cargo tauri build` 成功
- [ ] Settings 页面显示版本号
- [ ] 检查更新按钮可调用（开发环境下可 mock 或显示"已是最新"）
- [ ] CI workflow 文件语法正确（`act` 或 GitHub 可验证）
- [ ] 更新弹窗可手动触发展示

## Definition of Done

- Tests added/updated (unit/integration where appropriate)
- Lint / typecheck / CI green
- Docs/notes updated if behavior changes
- i18n 覆盖所有新增 UI 文本

## Out of Scope (explicit)

- macOS / Windows 构建支持（后续扩展）
- 自动 bump 版本号脚本（手动更新三文件）
- 增量更新 / delta patch

## Technical Notes

- 前端只有 `api.js` 允许直接访问 `window.__TAURI__`
- 更新检查可在 `api.js` 中封装 updater SDK 调用
- Changelog 渲染需注意 XSS：Clippy 禁止 innerHTML，需考虑安全的 Markdown 渲染方案（或纯文本展示）
- Settings 窗口是独立 Tauri 窗口，不共享主窗口 JS 上下文
- fxxkDJTU 的更新检查逻辑在 App.vue 根组件，对应 Clippy 应在 `app.js`（主窗口入口）中实现自动检查
