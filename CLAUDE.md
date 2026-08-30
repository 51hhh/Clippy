# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目简介

Clippy 是跨平台轻量剪贴板管理器，基于 Tauri v2 + Rust（后端）+ vanilla HTML/CSS/JS（主前端）+ React/TS（截图编辑功能岛）。当前架构见 `docs/architecture.md`，历史设计见 `docs/superpowers/specs/2026-04-24-clippy-clipboard-manager-design.md`。

已完成功能：剪贴板监听、SQLite 存储（含 FTS5 全文搜索）、悬浮面板、搜索、系统托盘、X11/Wayland 分流自动粘贴、冻结截图/Pin/图片编辑、OCR、翻译和设置面板。

## 开发环境搭建（Ubuntu）

```bash
# 1. Tauri v2 的系统依赖
sudo apt update && sudo apt install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev \
  pkg-config

# 2. 本项目额外依赖
#   libpipewire-0.3-dev  — ashpd 的 screencast feature（libspa-sys 需要 pkg-config）
#   libgbm-dev 等        — libwayshot-xcap 的 Wayland 截图链接依赖（-lgbm/-lEGL）
#   xvfb                 — scripts/smoke-dom.sh 与 ci-local.sh 的无头 DOM smoke
sudo apt install -y \
  libpipewire-0.3-dev \
  libgbm-dev libegl1-mesa-dev libdrm-dev libwayland-dev libxcb1-dev \
  xvfb

# 3. Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"

# 4. Node.js（最低 20.19，推荐当前 LTS）
curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
sudo apt install -y nodejs

# 5. Tauri CLI
cargo install tauri-cli --version "^2"
```

## 常用命令

```bash
# 启动开发服务器（热重载前端 + Rust 后端）
cargo tauri dev
# 没装 cargo-tauri 时用 npx（两者都可以，构建前钩子对两种 cwd 都成立）
npx --yes @tauri-apps/cli@^2 dev

# 构建发布包（输出到 src-tauri/target/release/bundle/）
cargo tauri build

# 仅编译 Rust 后端（快速检查编译错误）
cd src-tauri && cargo check --all-targets

# 运行 Rust 单元测试
cd src-tauri && cargo test

# 运行单个测试
cd src-tauri && cargo test test_name

# 格式化 + lint
cd src-tauri && cargo fmt
cd src-tauri && cargo clippy --all-targets -- -D warnings

# 性能基线（criterion，门禁只编译不运行；数字与坑见 docs/bench-baseline.md）
cd src-tauri && cargo bench

# 完整本地门禁（含 DOM/Xvfb smoke 与前端生产构建）
./scripts/ci-local.sh
```

## 架构概览

```
前端 (src/)
├── index.html / settings.html         — 主窗口与设置
├── pin.html / capture*.html           — React 功能岛入口
├── js/
│   ├── api.ts / ipc-types.ts          — 类型化 IPC 边界与 serde 合同
│   ├── app.js / clipboard-list.js     — 主窗口路由与列表状态
│   ├── preview-panel.js / preview/    — 预览调度与分职责渲染器
│   ├── translation-*.js               — 翻译面板与设置
│   └── settings.js / theme.js         — 设置与主题
├── react/
│   ├── capture-overlay/               — 冻结画面选区与选区翻译
│   ├── capture/                       — 图片编辑器
│   └── pin/                           — 统一贴图窗口
└── styles/                            — base/components/settings/themes

Rust 后端 (src-tauri/src/)
├── lib.rs / main.rs                   — Tauri 初始化与入口
├── commands.rs / commands/            — AppState 与按功能 IPC 命令
├── clipboard_watcher.rs / storage.rs  — 剪贴板监听与 SQLite/FTS5
├── paste/ / window_controller.rs      — X11/Portal 粘贴与窗口几何
├── capture/ / screenshot.rs           — CaptureSession 与平台截图
├── pin/ / pin_window.rs               — PinManager 与窗口适配
├── translation/ / ocr.rs              — 翻译服务、密钥与本地 OCR
└── config.rs / models.rs              — 配置与共享模型
```

### 数据流

1. `ClipboardWatcher` 独立线程每 500ms 轮询系统剪贴板（arboard）
2. SHA-256 哈希去重 → 重复内容只更新 `created_at` 置顶
3. 写入 SQLite `clips` 表 + 同步 `clips_fts` FTS5 虚拟表
4. `app.emit("clip-added")` / `app.emit("clip-removed")` 通知前端
5. 前端 `api.ts` 监听事件 → `clipboard-list.js` 增量更新 DOM

### 反向写入（select_clip）

用户选中条目 → 写入系统剪贴板 → 通过 `skip_hash` 机制让 watcher 跳过此次变更，避免重复存储。

### 窗口管理

- **main** 窗口：无边框悬浮面板（380×500），`visible: false` 启动，全局快捷键切换显隐
- **settings** 窗口：按需创建（从托盘菜单或 IPC `show_settings` 命令），不在 `tauri.conf.json` 中预声明

### 共享状态

`AppState` 通过 `app.manage()` 注入，包含：
- `storage: Arc<Mutex<StorageEngine>>` — 剪贴板数据库
- `config: Arc<Mutex<AppConfig>>` — 运行时配置
- `config_path: PathBuf` — 配置文件路径
- `watcher: ClipboardWatcher` — 持有监听器生命周期

### 前端约定

- 主界面无框架，纯 HTML/CSS/JS + ES Module `<script type="module">`
- 截图编辑页是隔离的 React/TS 功能岛（`src/react/capture/`），不反向重写主界面
- 使用 Vite 作为开发服务器和构建工具（`src/vite.config.mjs`）
- **只有 `api.ts` 允许直接访问 Tauri IPC**，其他模块通过 typed wrapper 导出函数间接调用
- 所有用户内容通过 React 文本节点或 `textContent` 写入 DOM；富文本只允许 DOMPurify 严格清洗后的 `innerHTML`
- HTML 实体解码使用隔离 `DOMParser`，禁止 `Function`/`eval` 动态执行
- 翻译 API key 只写系统 Secret Service；Wayland restore token 使用单独 0600 文件，不进入 AppConfig

## 关键设计约束

- 剪贴板监听采用轮询（~500ms），不使用系统通知机制
- 内容去重基于 SHA-256 哈希（`content_hash` 字段 UNIQUE 约束）
- 收藏条目不受历史上限清理影响（`cleanup_old_entries` 跳过 `is_favorite = 1`）
- SQLite 数据库位于 Tauri app data 目录，`config.storage_mode` 可切换为 `"memory"`
- FTS 索引需手动同步：插入时同步写 `clips_fts`，删除时用 FTS5 `'delete'` 命令清理
- 构建目标仅 Linux（deb, appimage），`tauri.conf.json` 中无 macOS/Windows bundle 配置
- 前端通过 Vite 构建输出到 `dist/`，Tauri 从 `frontendDist: "../dist"` 加载静态文件

## 语言约定

- 代码注释、commit message、文档使用**中文**
- 前端 UI 文本使用**英文**
