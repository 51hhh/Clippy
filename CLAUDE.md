# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目简介

Clippy 是跨平台轻量剪贴板管理器，基于 Tauri v2 + Rust（后端）+ vanilla HTML/CSS/JS（主前端）+ React/TS（截图编辑功能岛）。详细设计见 `docs/superpowers/specs/2026-04-24-clippy-clipboard-manager-design.md`。

已完成功能：剪贴板监听、SQLite 存储（含 FTS5 全文搜索）、悬浮面板、搜索、系统托盘、全局快捷键动态注册、设置面板（快捷键录制 + 主题切换 + 历史上限）。

## 开发环境搭建（Ubuntu）

```bash
# 1. Tauri v2 的系统依赖
sudo apt update && sudo apt install -y \
  libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev \
  pkg-config

# 2. Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"

# 3. Node.js（推荐 v20 LTS）
curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
sudo apt install -y nodejs

# 4. Tauri CLI
cargo install tauri-cli --version "^2"
```

## 常用命令

```bash
# 启动开发服务器（热重载前端 + Rust 后端）
cargo tauri dev

# 构建发布包（输出到 src-tauri/target/release/bundle/）
cargo tauri build

# 仅编译 Rust 后端（快速检查编译错误）
cd src-tauri && cargo check

# 运行 Rust 单元测试
cd src-tauri && cargo test

# 运行单个测试
cd src-tauri && cargo test test_name

# 格式化 + lint
cd src-tauri && cargo fmt
cd src-tauri && cargo clippy -- -D warnings
```

## 架构概览

```
前端 (src/)                            Rust 后端 (src-tauri/src/)
├── index.html                         ├── main.rs              — 入口（调用 lib::run）
├── settings.html                      ├── lib.rs               — Tauri 初始化：插件、托盘、快捷键、状态管理
├── pin.html                           ├── commands.rs          — Tauri IPC 命令
├── capture.html                       ├── screenshot.rs        — Linux 截图捕获与 PNG 编码
├── vite.config.mjs                    ├── clipboard_watcher.rs — 独立线程轮询剪贴板（arboard, 500ms）
├── js/                                ├── storage.rs           — SQLite + FTS5 存储引擎
│   ├── api.js  — IPC 封装层           ├── config.rs            — JSON 配置读写
│   ├── app.js  — 主窗口入口           ├── models.rs            — ClipItem, AppConfig, ContentType
│   ├── clipboard-list.js              ├── ocr.rs               — Tesseract OCR 集成
│   ├── search.js                      ├── gsettings_shortcuts.rs — Wayland 快捷键（D-Bus Portal）
│   ├── settings.js                    └── tray_icon.rs         — 系统托盘
│   ├── pin.js
│   └── theme.js
├── react/
│   └── capture/                       — React/TS 截图编辑功能岛
└── styles/
    ├── base.css
    ├── components.css
    ├── settings.css
    └── themes.css
```

### 数据流

1. `ClipboardWatcher` 独立线程每 500ms 轮询系统剪贴板（arboard）
2. SHA-256 哈希去重 → 重复内容只更新 `created_at` 置顶
3. 写入 SQLite `clips` 表 + 同步 `clips_fts` FTS5 虚拟表
4. `app.emit("clip-added")` / `app.emit("clip-removed")` 通知前端
5. 前端 `api.js` 监听事件 → `clipboard-list.js` 增量更新 DOM

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
- **只有 `api.js` 允许直接访问 `window.__TAURI__`**，其他模块通过 `api.js` 导出函数间接调用
- 所有用户内容通过 `textContent` 写入 DOM（防 XSS），不使用 `innerHTML`
- 设置页面（`settings.js`）是例外：独立于主窗口，直接调用 `invoke`

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
