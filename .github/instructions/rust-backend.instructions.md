---
description: "Use when writing or modifying Rust backend code in src-tauri/src/. Covers Tauri v2 IPC commands, SQLite storage, clipboard watcher, and shared state patterns."
applyTo: "src-tauri/src/**/*.rs"
---
# Rust 后端约定

## 模块结构
扁平布局，每文件一个关注点。新增模块需在 `lib.rs` 中 `mod` 声明。

## IPC 命令
- `commands.rs` 集中定义所有 `#[tauri::command]`
- 通过 `AppState` 访问共享状态：`storage: Arc<Mutex<StorageEngine>>`, `config: Arc<Mutex<AppConfig>>`
- IPC 边界返回 `Result<T, String>`，内部用 `thiserror`

## 数据库
- rusqlite (bundled) + FTS5 虚拟表
- 插入时同步写 `clips` 和 `clips_fts`
- 删除时用 FTS5 `'delete'` 命令清理虚拟表
- 内容去重：SHA-256 哈希（`content_hash` UNIQUE 约束）

## 剪贴板监听
- `clipboard_watcher.rs`：独立线程 500ms 轮询（arboard）
- 反向写入通过 `skip_hash` 跳过，避免重复存储

## 事件通知
- `app.emit("clip-added")` / `app.emit("clip-removed")` / `app.emit("config-changed")`
- 前端通过 `api.js` 监听

## 全局快捷键
- X11: `tauri-plugin-global-shortcut`
- Wayland: `portal_shortcuts.rs` + ashpd (XDG Portal D-Bus)
- `lib.rs` 启动时检测 `XDG_SESSION_TYPE` 自动选择方案

## 详细架构
见 [CLAUDE.md](../../CLAUDE.md)
