# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目简介

Clippy 是跨平台轻量剪贴板管理器，基于 Tauri v2 + Rust（后端）+ vanilla HTML/CSS/JS（前端）。详细设计见 `docs/superpowers/specs/2026-04-24-clippy-clipboard-manager-design.md`。

当前阶段为 **MVP**：实现剪贴板监听、SQLite 存储（含 FTS5 全文搜索）、悬浮面板、搜索。系统托盘、开机自启、设置面板留到后续迭代。

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
Tauri IPC
前端 (src/)  ←────────→  Rust 后端 (src-tauri/src/)
index.html                  main.rs              — Tauri 入口，注册命令和插件
styles.css                  clipboard_watcher.rs — 独立线程轮询剪贴板（arboard）
main.js                     storage.rs           — SQLite + FTS5 存储引擎
                            config.rs            — JSON 配置读写
                            commands.rs          — Tauri IPC 命令定义
                            models.rs            — 数据结构（ClipItem 等）
```

**数据流**：ClipboardWatcher 线程轮询剪贴板 → 内容哈希去重 → 写入 SQLite → 通过 `app.emit("clip-added")` 通知前端刷新。

**前端无框架**：纯 HTML/CSS/JS，通过 `window.__TAURI__` 调用后端 IPC 命令，监听 Tauri 事件更新 UI。

## 关键设计约束

- 剪贴板监听采用轮询（~500ms），不使用系统通知机制
- 内容去重基于 SHA-256 哈希（`content_hash` 字段 UNIQUE 约束）
- 收藏条目不受历史上限清理影响
- SQLite 数据库位于 Tauri app data 目录，支持切换为内存模式
- v1 前端仅英文 UI

## 语言约定

- 代码注释、commit message、文档使用**中文**
- 前端 UI 文本使用**英文**
