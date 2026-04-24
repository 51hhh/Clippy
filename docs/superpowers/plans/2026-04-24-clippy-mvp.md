# Clippy Clipboard Manager MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a cross-platform clipboard manager with clipboard monitoring, SQLite+FTS5 storage, floating panel UI, search, and global shortcut.

**Architecture:** Tauri v2 app with Rust backend (clipboard polling via arboard, SQLite via rusqlite, serde for config) and vanilla HTML/CSS/JS frontend (no framework, no bundler). Frontend communicates with backend exclusively through Tauri IPC commands and events.

**Tech Stack:** Rust, Tauri v2, rusqlite (bundled + FTS5), arboard, sha2, serde, thiserror, tauri-plugin-global-shortcut, vanilla HTML/CSS/JS (ES modules)

---

## File Structure

```
clippy/
├── src/                              # 前端（Tauri frontendDist）
│   ├── index.html                    # 单页面 HTML
│   ├── styles/
│   │   ├── base.css                  # 重置 + 基础样式
│   │   ├── themes.css                # CSS 变量（light/dark/ocean/forest）
│   │   └── components.css            # 组件样式
│   ├── js/
│   │   ├── app.js                    # 入口：初始化 + 事件绑定
│   │   ├── api.js                    # 封装所有 Tauri IPC（唯一 __TAURI__ 耦合）
│   │   ├── clipboard-list.js         # 列表渲染 + 无限滚动
│   │   ├── search.js                 # 搜索框 + debounce
│   │   └── theme.js                  # 主题切换
│   └── assets/
│       └── icon.png
├── src-tauri/
│   ├── src/
│   │   ├── main.rs                   # 二进制入口
│   │   ├── lib.rs                    # Tauri Builder 注册
│   │   ├── models.rs                 # ClipItem, AppConfig, ContentType
│   │   ├── storage.rs                # SQLite + FTS5 引擎
│   │   ├── clipboard_watcher.rs      # 剪贴板轮询线程
│   │   ├── config.rs                 # JSON 配置读写
│   │   └── commands.rs               # Tauri IPC 命令
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── build.rs
│   └── capabilities/
│       └── default.json
```

---

## Task 1: Tauri v2 项目脚手架

**Files:**
- Create: `src-tauri/Cargo.toml`
- Create: `src-tauri/tauri.conf.json`
- Create: `src-tauri/build.rs`
- Create: `src-tauri/capabilities/default.json`
- Create: `src-tauri/src/main.rs`
- Create: `src-tauri/src/lib.rs`
- Create: `src/index.html`

- [ ] **Step 1: 创建 Cargo.toml**

```toml
[package]
name = "clippy-app"
version = "0.1.0"
description = "跨平台轻量剪贴板管理器"
authors = ["zhongweixi2000"]
edition = "2021"

[lib]
name = "clippy_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-global-shortcut = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
rusqlite = { version = "0.31", features = ["bundled"] }
arboard = "3"
sha2 = "0.10"
thiserror = "2"
log = "0.4"
env_logger = "0.11"

[profile.release]
strip = true
lto = "thin"
codegen-units = 1
```

- [ ] **Step 2: 创建 build.rs**

```rust
fn main() {
    tauri_build::build()
}
```

- [ ] **Step 3: 创建 tauri.conf.json**

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Clippy",
  "version": "0.1.0",
  "identifier": "com.clippy.app",
  "build": {
    "frontendDist": "../src"
  },
  "app": {
    "windows": [
      {
        "title": "Clippy",
        "width": 380,
        "height": 500,
        "resizable": false,
        "decorations": false,
        "alwaysOnTop": true,
        "visible": false,
        "center": true
      }
    ],
    "security": {
      "csp": "default-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self'"
    }
  },
  "bundle": {
    "active": true,
    "targets": ["deb", "appimage"],
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/icon.ico",
      "icons/icon.icns"
    ]
  }
}
```

- [ ] **Step 4: 创建 capabilities/default.json**

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Clippy 主窗口权限",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "global-shortcut:default"
  ]
}
```

- [ ] **Step 5: 创建 main.rs**

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    clippy_lib::run()
}
```

- [ ] **Step 6: 创建 lib.rs（最小版本）**

```rust
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::init())
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}
```

- [ ] **Step 7: 创建 src/index.html（最小版本）**

```html
<!DOCTYPE html>
<html lang="en" data-theme="light">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Clippy</title>
</head>
<body>
  <h1>Clippy</h1>
</body>
</html>
```

- [ ] **Step 8: 验证编译**

```bash
cd src-tauri && cargo check
```

Expected: 编译通过，无错误。

- [ ] **Step 9: 提交**

```bash
git add src-tauri/ src/index.html
git commit -m "feat: Tauri v2 项目脚手架初始化"
```

---

## Task 2: 数据模型（models.rs）

**Files:**
- Create: `src-tauri/src/models.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 创建 models.rs**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ContentType {
    Text,
    Html,
    Image,
}

impl ContentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContentType::Text => "text",
            ContentType::Html => "html",
            ContentType::Image => "image",
        }
    }
}

impl std::fmt::Display for ContentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ContentType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "text" => Ok(ContentType::Text),
            "html" => Ok(ContentType::Html),
            "image" => Ok(ContentType::Image),
            other => Err(format!("未知内容类型: {}", other)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipItem {
    pub id: i64,
    pub content_type: ContentType,
    pub text_content: Option<String>,
    pub html_content: Option<String>,
    pub image_data: Option<Vec<u8>>,
    pub content_hash: String,
    pub is_favorite: bool,
    pub created_at: i64,
    pub byte_size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub max_history: u32,
    pub storage_mode: String,
    pub global_shortcut: String,
    pub theme: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            max_history: 100,
            storage_mode: "persistent".to_string(),
            global_shortcut: "CmdOrCtrl+Shift+V".to_string(),
            theme: "light".to_string(),
        }
    }
}
```

- [ ] **Step 2: 在 lib.rs 中注册模块**

在 `lib.rs` 顶部添加：

```rust
mod models;
```

- [ ] **Step 3: 验证编译**

```bash
cd src-tauri && cargo check
```

- [ ] **Step 4: 提交**

```bash
git add src-tauri/src/models.rs src-tauri/src/lib.rs
git commit -m "feat: 添加数据模型 ClipItem 和 AppConfig"
```

---

## Task 3: SQLite 存储引擎（storage.rs）

**Files:**
- Create: `src-tauri/src/storage.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 创建 storage.rs — 结构体和初始化**

```rust
use crate::models::{ClipItem, ContentType};
use rusqlite::{params, Connection, Result as SqlResult};
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("数据库操作失败: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("内容类型解析失败: {0}")]
    InvalidContentType(String),
}

pub struct StorageEngine {
    conn: Connection,
}

impl StorageEngine {
    pub fn new(db_path: &Path) -> Result<Self, StorageError> {
        let conn = Connection::open(db_path)?;
        let engine = Self { conn };
        engine.init_tables()?;
        Ok(engine)
    }

    pub fn new_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        let engine = Self { conn };
        engine.init_tables()?;
        Ok(engine)
    }

    fn init_tables(&self) -> Result<(), StorageError> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS clips (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                content_type TEXT NOT NULL,
                text_content TEXT,
                html_content TEXT,
                image_data   BLOB,
                content_hash TEXT NOT NULL UNIQUE,
                is_favorite  INTEGER DEFAULT 0,
                created_at   INTEGER NOT NULL,
                byte_size    INTEGER NOT NULL
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS clips_fts USING fts5(
                text_content,
                content='clips',
                content_rowid='id'
            );

            CREATE INDEX IF NOT EXISTS idx_clips_created_at ON clips(created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_clips_favorite ON clips(is_favorite, created_at DESC);"
        )?;
        Ok(())
    }
}
```

- [ ] **Step 2: 添加插入方法**

在 `StorageEngine` impl 块中追加：

```rust
    pub fn insert_clip(
        &self,
        content_type: &ContentType,
        text_content: Option<&str>,
        html_content: Option<&str>,
        image_data: Option<&[u8]>,
        content_hash: &str,
        byte_size: i64,
    ) -> Result<ClipItem, StorageError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // 如果哈希已存在，更新 created_at 使其排到最前
        let existing = self.conn.query_row(
            "SELECT id FROM clips WHERE content_hash = ?1",
            params![content_hash],
            |row| row.get::<_, i64>(0),
        );

        if let Ok(existing_id) = existing {
            self.conn.execute(
                "UPDATE clips SET created_at = ?1 WHERE id = ?2",
                params![now, existing_id],
            )?;
            return self.get_clip_by_id(existing_id);
        }

        self.conn.execute(
            "INSERT INTO clips (content_type, text_content, html_content, image_data, content_hash, created_at, byte_size)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                content_type.as_str(),
                text_content,
                html_content,
                image_data,
                content_hash,
                now,
                byte_size,
            ],
        )?;

        let clip_id = self.conn.last_insert_rowid();

        // 同步 FTS 索引
        if let Some(text) = text_content {
            self.conn.execute(
                "INSERT INTO clips_fts(rowid, text_content) VALUES (?1, ?2)",
                params![clip_id, text],
            )?;
        }

        self.get_clip_by_id(clip_id)
    }

    fn get_clip_by_id(&self, id: i64) -> Result<ClipItem, StorageError> {
        let clip = self.conn.query_row(
            "SELECT id, content_type, text_content, html_content, image_data, content_hash, is_favorite, created_at, byte_size
             FROM clips WHERE id = ?1",
            params![id],
            |row| {
                Ok(ClipItem {
                    id: row.get(0)?,
                    content_type: row.get::<_, String>(1)?
                        .parse()
                        .unwrap_or(ContentType::Text),
                    text_content: row.get(2)?,
                    html_content: row.get(3)?,
                    image_data: row.get(4)?,
                    content_hash: row.get(5)?,
                    is_favorite: row.get::<_, i32>(6)? != 0,
                    created_at: row.get(7)?,
                    byte_size: row.get(8)?,
                })
            },
        )?;
        Ok(clip)
    }
```

- [ ] **Step 3: 添加查询和搜索方法**

```rust
    pub fn get_clips(
        &self,
        query: Option<&str>,
        favorites_only: bool,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<ClipItem>, StorageError> {
        let mut clips = Vec::new();

        if let Some(q) = query {
            if !q.trim().is_empty() {
                // FTS 全文搜索
                let mut stmt = self.conn.prepare(
                    "SELECT c.id, c.content_type, c.text_content, c.html_content, c.image_data,
                            c.content_hash, c.is_favorite, c.created_at, c.byte_size
                     FROM clips c
                     JOIN clips_fts f ON c.id = f.rowid
                     WHERE clips_fts MATCH ?1
                     ORDER BY c.created_at DESC
                     LIMIT ?2 OFFSET ?3"
                )?;

                let rows = stmt.query_map(params![q, limit, offset], |row| {
                    Ok(ClipItem {
                        id: row.get(0)?,
                        content_type: row.get::<_, String>(1)?
                            .parse()
                            .unwrap_or(ContentType::Text),
                        text_content: row.get(2)?,
                        html_content: row.get(3)?,
                        image_data: row.get(4)?,
                        content_hash: row.get(5)?,
                        is_favorite: row.get::<_, i32>(6)? != 0,
                        created_at: row.get(7)?,
                        byte_size: row.get(8)?,
                    })
                })?;

                for row in rows {
                    clips.push(row?);
                }
                return Ok(clips);
            }
        }

        // 普通查询（无搜索词）
        let sql = if favorites_only {
            "SELECT id, content_type, text_content, html_content, image_data,
                    content_hash, is_favorite, created_at, byte_size
             FROM clips WHERE is_favorite = 1
             ORDER BY created_at DESC LIMIT ?1 OFFSET ?2"
        } else {
            "SELECT id, content_type, text_content, html_content, image_data,
                    content_hash, is_favorite, created_at, byte_size
             FROM clips ORDER BY created_at DESC LIMIT ?1 OFFSET ?2"
        };

        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params![limit, offset], |row| {
            Ok(ClipItem {
                id: row.get(0)?,
                content_type: row.get::<_, String>(1)?
                    .parse()
                    .unwrap_or(ContentType::Text),
                text_content: row.get(2)?,
                html_content: row.get(3)?,
                image_data: row.get(4)?,
                content_hash: row.get(5)?,
                is_favorite: row.get::<_, i32>(6)? != 0,
                created_at: row.get(7)?,
                byte_size: row.get(8)?,
            })
        })?;

        for row in rows {
            clips.push(row?);
        }
        Ok(clips)
    }
```

- [ ] **Step 4: 添加删除、收藏、清空、历史清理方法**

```rust
    pub fn delete_clip(&self, id: i64) -> Result<(), StorageError> {
        // 先删 FTS
        let text: Option<String> = self.conn.query_row(
            "SELECT text_content FROM clips WHERE id = ?1",
            params![id],
            |row| row.get(0),
        ).ok().flatten();

        if let Some(text) = text {
            self.conn.execute(
                "INSERT INTO clips_fts(clips_fts, rowid, text_content) VALUES ('delete', ?1, ?2)",
                params![id, text],
            )?;
        }

        self.conn.execute("DELETE FROM clips WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn toggle_favorite(&self, id: i64) -> Result<bool, StorageError> {
        self.conn.execute(
            "UPDATE clips SET is_favorite = CASE WHEN is_favorite = 0 THEN 1 ELSE 0 END WHERE id = ?1",
            params![id],
        )?;
        let is_fav: bool = self.conn.query_row(
            "SELECT is_favorite FROM clips WHERE id = ?1",
            params![id],
            |row| Ok(row.get::<_, i32>(0)? != 0),
        )?;
        Ok(is_fav)
    }

    pub fn clear_history(&self) -> Result<(), StorageError> {
        self.conn.execute("DELETE FROM clips WHERE is_favorite = 0", [])?;
        self.conn.execute("INSERT INTO clips_fts(clips_fts) VALUES ('rebuild')", [])?;
        Ok(())
    }

    pub fn cleanup_old_entries(&self, max_history: u32) -> Result<Vec<i64>, StorageError> {
        if max_history == 0 {
            return Ok(vec![]);
        }
        let count: u32 = self.conn.query_row(
            "SELECT COUNT(*) FROM clips WHERE is_favorite = 0",
            [],
            |row| row.get(0),
        )?;

        if count <= max_history {
            return Ok(vec![]);
        }

        let excess = count - max_history;
        let mut stmt = self.conn.prepare(
            "SELECT id FROM clips WHERE is_favorite = 0 ORDER BY created_at ASC LIMIT ?1"
        )?;
        let ids: Vec<i64> = stmt.query_map(params![excess], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        for id in &ids {
            self.delete_clip(*id)?;
        }

        Ok(ids)
    }
```

- [ ] **Step 5: 添加单元测试**

在 `storage.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_engine() -> StorageEngine {
        StorageEngine::new_in_memory().unwrap()
    }

    #[test]
    fn test_insert_and_query() {
        let engine = make_engine();
        let clip = engine.insert_clip(
            &ContentType::Text, Some("hello world"), None, None, "hash1", 11,
        ).unwrap();
        assert_eq!(clip.text_content.as_deref(), Some("hello world"));

        let clips = engine.get_clips(None, false, 0, 20).unwrap();
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].id, clip.id);
    }

    #[test]
    fn test_dedup_updates_timestamp() {
        let engine = make_engine();
        let clip1 = engine.insert_clip(
            &ContentType::Text, Some("dup"), None, None, "same_hash", 3,
        ).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(1));
        let clip2 = engine.insert_clip(
            &ContentType::Text, Some("dup"), None, None, "same_hash", 3,
        ).unwrap();
        assert_eq!(clip1.id, clip2.id);
        assert!(clip2.created_at >= clip1.created_at);

        let all = engine.get_clips(None, false, 0, 20).unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn test_fts_search() {
        let engine = make_engine();
        engine.insert_clip(&ContentType::Text, Some("rust language"), None, None, "h1", 13).unwrap();
        engine.insert_clip(&ContentType::Text, Some("python script"), None, None, "h2", 13).unwrap();

        let results = engine.get_clips(Some("rust"), false, 0, 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text_content.as_deref(), Some("rust language"));
    }

    #[test]
    fn test_delete_clip() {
        let engine = make_engine();
        let clip = engine.insert_clip(&ContentType::Text, Some("to delete"), None, None, "hd", 9).unwrap();
        engine.delete_clip(clip.id).unwrap();
        let all = engine.get_clips(None, false, 0, 20).unwrap();
        assert!(all.is_empty());
    }

    #[test]
    fn test_toggle_favorite() {
        let engine = make_engine();
        let clip = engine.insert_clip(&ContentType::Text, Some("fav"), None, None, "hf", 3).unwrap();
        assert!(!clip.is_favorite);

        let is_fav = engine.toggle_favorite(clip.id).unwrap();
        assert!(is_fav);

        let is_fav = engine.toggle_favorite(clip.id).unwrap();
        assert!(!is_fav);
    }

    #[test]
    fn test_cleanup_preserves_favorites() {
        let engine = make_engine();
        for i in 0..5 {
            engine.insert_clip(
                &ContentType::Text, Some(&format!("item {}", i)), None, None, &format!("h{}", i), 6,
            ).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // 收藏第一个
        let clips = engine.get_clips(None, false, 0, 20).unwrap();
        let oldest_id = clips.last().unwrap().id;
        engine.toggle_favorite(oldest_id).unwrap();

        let removed = engine.cleanup_old_entries(2).unwrap();
        assert_eq!(removed.len(), 2);
        assert!(!removed.contains(&oldest_id));

        let remaining = engine.get_clips(None, false, 0, 20).unwrap();
        assert_eq!(remaining.len(), 3); // 2 non-fav + 1 fav
    }

    #[test]
    fn test_clear_history_preserves_favorites() {
        let engine = make_engine();
        engine.insert_clip(&ContentType::Text, Some("keep"), None, None, "h1", 4).unwrap();
        let clip2 = engine.insert_clip(&ContentType::Text, Some("del"), None, None, "h2", 3).unwrap();
        engine.toggle_favorite(clip2.id).unwrap();

        engine.clear_history().unwrap();
        let all = engine.get_clips(None, false, 0, 20).unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].is_favorite);
    }
}
```

- [ ] **Step 6: 在 lib.rs 中注册模块**

```rust
mod storage;
```

- [ ] **Step 7: 运行测试**

```bash
cd src-tauri && cargo test
```

Expected: 全部通过。

- [ ] **Step 8: 提交**

```bash
git add src-tauri/src/storage.rs src-tauri/src/lib.rs
git commit -m "feat: 添加 SQLite + FTS5 存储引擎及单元测试"
```

---

## Task 4: 配置管理（config.rs）

**Files:**
- Create: `src-tauri/src/config.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 创建 config.rs**

```rust
use crate::models::AppConfig;
use std::fs;
use std::path::Path;

pub fn load_config(config_path: &Path) -> AppConfig {
    if config_path.exists() {
        match fs::read_to_string(config_path) {
            Ok(content) => {
                serde_json::from_str(&content).unwrap_or_else(|e| {
                    log::warn!("配置文件解析失败，使用默认配置: {}", e);
                    AppConfig::default()
                })
            }
            Err(e) => {
                log::warn!("配置文件读取失败，使用默认配置: {}", e);
                AppConfig::default()
            }
        }
    } else {
        let config = AppConfig::default();
        save_config(config_path, &config);
        config
    }
}

pub fn save_config(config_path: &Path, config: &AppConfig) {
    if let Some(parent) = config_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(config) {
        Ok(json) => {
            if let Err(e) = fs::write(config_path, json) {
                log::error!("配置文件写入失败: {}", e);
            }
        }
        Err(e) => {
            log::error!("配置序列化失败: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.max_history, 100);
        assert_eq!(config.theme, "light");
        assert_eq!(config.storage_mode, "persistent");
    }

    #[test]
    fn test_load_missing_creates_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        let config = load_config(&path);
        assert_eq!(config.max_history, 100);
        assert!(path.exists());
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut config = AppConfig::default();
        config.theme = "dark".to_string();
        save_config(&path, &config);

        let loaded = load_config(&path);
        assert_eq!(loaded.theme, "dark");
    }
}
```

- [ ] **Step 2: 添加 tempfile dev-dependency 到 Cargo.toml**

在 `[dependencies]` 下面添加：

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: 在 lib.rs 中注册模块**

```rust
mod config;
```

- [ ] **Step 4: 运行测试**

```bash
cd src-tauri && cargo test
```

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/config.rs src-tauri/Cargo.toml src-tauri/src/lib.rs
git commit -m "feat: 添加 JSON 配置管理及单元测试"
```

---

## Task 5: 剪贴板监听器（clipboard_watcher.rs）

**Files:**
- Create: `src-tauri/src/clipboard_watcher.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 创建 clipboard_watcher.rs**

```rust
use crate::models::{AppConfig, ContentType};
use crate::storage::StorageEngine;
use arboard::Clipboard;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub struct ClipboardWatcher {
    running: Arc<Mutex<bool>>,
}

impl ClipboardWatcher {
    pub fn new() -> Self {
        Self {
            running: Arc::new(Mutex::new(false)),
        }
    }

    pub fn start(
        &self,
        app_handle: AppHandle,
        storage: Arc<Mutex<StorageEngine>>,
        config: Arc<Mutex<AppConfig>>,
    ) {
        let running = Arc::clone(&self.running);
        {
            let mut r = running.lock().unwrap();
            if *r {
                return;
            }
            *r = true;
        }

        thread::spawn(move || {
            let mut clipboard = match Clipboard::new() {
                Ok(c) => c,
                Err(e) => {
                    log::error!("剪贴板初始化失败: {}", e);
                    return;
                }
            };

            let mut last_hash = String::new();
            log::info!("剪贴板监听器已启动");

            loop {
                {
                    let r = running.lock().unwrap();
                    if !*r {
                        break;
                    }
                }

                // 尝试读取文本
                if let Ok(text) = clipboard.get_text() {
                    if !text.is_empty() {
                        let hash = compute_hash(text.as_bytes());
                        if hash != last_hash {
                            last_hash = hash.clone();
                            let byte_size = text.len() as i64;
                            let storage = storage.lock().unwrap();
                            match storage.insert_clip(
                                &ContentType::Text,
                                Some(&text),
                                None,
                                None,
                                &hash,
                                byte_size,
                            ) {
                                Ok(clip) => {
                                    let max_history = config.lock().unwrap().max_history;
                                    if let Ok(removed_ids) = storage.cleanup_old_entries(max_history) {
                                        for rid in removed_ids {
                                            let _ = app_handle.emit("clip-removed", rid);
                                        }
                                    }
                                    let _ = app_handle.emit("clip-added", &clip);
                                    log::debug!("新剪贴板内容，类型: text, 大小: {} 字节", byte_size);
                                }
                                Err(e) => {
                                    log::warn!("剪贴板内容保存失败: {}", e);
                                }
                            }
                        }
                    }
                }

                thread::sleep(Duration::from_millis(500));
            }

            log::info!("剪贴板监听器已停止");
        });
    }

    pub fn stop(&self) {
        let mut r = self.running.lock().unwrap();
        *r = false;
    }
}

fn compute_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}
```

- [ ] **Step 2: 在 lib.rs 中注册模块**

```rust
mod clipboard_watcher;
```

- [ ] **Step 3: 验证编译**

```bash
cd src-tauri && cargo check
```

- [ ] **Step 4: 提交**

```bash
git add src-tauri/src/clipboard_watcher.rs src-tauri/src/lib.rs
git commit -m "feat: 添加剪贴板监听器（arboard 轮询 + SHA-256 去重）"
```

---

## Task 6: IPC 命令（commands.rs）+ 完整 lib.rs 集成

**Files:**
- Create: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 创建 commands.rs**

```rust
use crate::clipboard_watcher::ClipboardWatcher;
use crate::config;
use crate::models::{AppConfig, ClipItem};
use crate::storage::StorageEngine;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::State;

pub struct AppState {
    pub storage: Arc<Mutex<StorageEngine>>,
    pub config: Arc<Mutex<AppConfig>>,
    pub config_path: PathBuf,
    pub watcher: ClipboardWatcher,
}

#[tauri::command]
pub fn get_clips(
    state: State<AppState>,
    query: Option<String>,
    favorites_only: bool,
    offset: u32,
    limit: u32,
) -> Result<Vec<ClipItem>, String> {
    let storage = state.storage.lock().unwrap();
    storage
        .get_clips(query.as_deref(), favorites_only, offset, limit)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_clip(state: State<AppState>, id: i64) -> Result<(), String> {
    let storage = state.storage.lock().unwrap();
    storage.delete_clip(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn toggle_favorite(state: State<AppState>, id: i64) -> Result<bool, String> {
    let storage = state.storage.lock().unwrap();
    storage.toggle_favorite(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clear_history(state: State<AppState>) -> Result<(), String> {
    let storage = state.storage.lock().unwrap();
    storage.clear_history().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn select_clip(
    state: State<AppState>,
    app_handle: tauri::AppHandle,
    id: i64,
) -> Result<(), String> {
    let storage = state.storage.lock().unwrap();
    let clip = storage.get_clips(None, false, 0, 1).map_err(|e| e.to_string())?;

    // 从数据库获取指定 clip
    let all = storage.get_clips(None, false, 0, 9999).map_err(|e| e.to_string())?;
    let clip = all.iter().find(|c| c.id == id)
        .ok_or_else(|| "条目不存在".to_string())?;

    // 写入系统剪贴板
    if let Some(ref text) = clip.text_content {
        let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        clipboard.set_text(text).map_err(|e| e.to_string())?;
    }

    // 隐藏窗口
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.hide();
    }

    Ok(())
}

#[tauri::command]
pub fn get_config(state: State<AppState>) -> AppConfig {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
pub fn update_config(state: State<AppState>, new_config: AppConfig) -> Result<(), String> {
    let mut config = state.config.lock().unwrap();
    *config = new_config.clone();
    config::save_config(&state.config_path, &new_config);
    Ok(())
}
```

- [ ] **Step 2: 重写 lib.rs 完成全部集成**

```rust
mod clipboard_watcher;
mod commands;
mod config;
mod models;
mod storage;

use clipboard_watcher::ClipboardWatcher;
use commands::AppState;
use std::sync::{Arc, Mutex};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir().expect("无法获取 app data 目录");
            std::fs::create_dir_all(&app_data_dir).expect("无法创建 app data 目录");

            let config_path = app_data_dir.join("config.json");
            let app_config = config::load_config(&config_path);

            let db_path = if app_config.storage_mode == "memory" {
                None
            } else {
                Some(app_data_dir.join("clippy.db"))
            };

            let storage = if let Some(ref path) = db_path {
                storage::StorageEngine::new(path).expect("数据库初始化失败")
            } else {
                storage::StorageEngine::new_in_memory().expect("内存数据库初始化失败")
            };

            let storage = Arc::new(Mutex::new(storage));
            let config = Arc::new(Mutex::new(app_config));

            let watcher = ClipboardWatcher::new();
            watcher.start(
                app.handle().clone(),
                Arc::clone(&storage),
                Arc::clone(&config),
            );

            let state = AppState {
                storage,
                config,
                config_path,
                watcher,
            };

            app.manage(state);

            // 注册全局快捷键
            use tauri_plugin_global_shortcut::ShortcutState;
            app.global_shortcut().on_shortcut("CmdOrCtrl+Shift+V", move |app, _shortcut, event| {
                if event.state == ShortcutState::Pressed {
                    if let Some(window) = app.get_webview_window("main") {
                        if window.is_visible().unwrap_or(false) {
                            let _ = window.hide();
                        } else {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                }
            })?;

            log::info!("Clippy 启动完成");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_clips,
            commands::delete_clip,
            commands::toggle_favorite,
            commands::clear_history,
            commands::select_clip,
            commands::get_config,
            commands::update_config,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}
```

- [ ] **Step 3: 验证编译**

```bash
cd src-tauri && cargo check
```

- [ ] **Step 4: 运行已有测试**

```bash
cd src-tauri && cargo test
```

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat: 添加 IPC 命令层，集成全部后端模块 + 全局快捷键"
```

---

## Task 7: 前端 — CSS 主题和基础样式

**Files:**
- Create: `src/styles/base.css`
- Create: `src/styles/themes.css`
- Create: `src/styles/components.css`

- [ ] **Step 1: 创建 base.css**

```css
*, *::before, *::after {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

html, body {
  width: 100%;
  height: 100%;
  overflow: hidden;
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  font-size: 14px;
  line-height: 1.4;
  color: var(--text-primary);
  background: var(--bg-primary);
  user-select: none;
  -webkit-user-select: none;
}

::-webkit-scrollbar {
  width: 6px;
}

::-webkit-scrollbar-track {
  background: transparent;
}

::-webkit-scrollbar-thumb {
  background: var(--scrollbar-thumb);
  border-radius: 3px;
}

::-webkit-scrollbar-thumb:hover {
  background: var(--scrollbar-thumb-hover);
}
```

- [ ] **Step 2: 创建 themes.css**

```css
[data-theme="light"] {
  --bg-primary: #ffffff;
  --bg-secondary: #f5f5f5;
  --bg-hover: #e8e8e8;
  --bg-active: #d0d0d0;
  --text-primary: #1a1a1a;
  --text-secondary: #666666;
  --text-muted: #999999;
  --accent: #4a90d9;
  --accent-hover: #357abd;
  --border: #e0e0e0;
  --danger: #e74c3c;
  --danger-hover: #c0392b;
  --favorite: #f1c40f;
  --scrollbar-thumb: #c0c0c0;
  --scrollbar-thumb-hover: #a0a0a0;
  --overlay-bg: rgba(255, 255, 255, 0.85);
  --shadow: 0 2px 12px rgba(0, 0, 0, 0.15);
}

[data-theme="dark"] {
  --bg-primary: #1e1e1e;
  --bg-secondary: #2a2a2a;
  --bg-hover: #333333;
  --bg-active: #404040;
  --text-primary: #e0e0e0;
  --text-secondary: #a0a0a0;
  --text-muted: #707070;
  --accent: #5b9fe6;
  --accent-hover: #4a8ed4;
  --border: #3a3a3a;
  --danger: #e74c3c;
  --danger-hover: #c0392b;
  --favorite: #f1c40f;
  --scrollbar-thumb: #555555;
  --scrollbar-thumb-hover: #777777;
  --overlay-bg: rgba(30, 30, 30, 0.85);
  --shadow: 0 2px 12px rgba(0, 0, 0, 0.4);
}

[data-theme="ocean"] {
  --bg-primary: #0d1b2a;
  --bg-secondary: #1b2838;
  --bg-hover: #253545;
  --bg-active: #2f4255;
  --text-primary: #e0e1dd;
  --text-secondary: #8d99ae;
  --text-muted: #5c6b7a;
  --accent: #48cae4;
  --accent-hover: #00b4d8;
  --border: #2a3a4a;
  --danger: #ef476f;
  --danger-hover: #d63057;
  --favorite: #ffd166;
  --scrollbar-thumb: #3a4a5a;
  --scrollbar-thumb-hover: #4a5a6a;
  --overlay-bg: rgba(13, 27, 42, 0.85);
  --shadow: 0 2px 12px rgba(0, 0, 0, 0.5);
}

[data-theme="forest"] {
  --bg-primary: #1b2e1b;
  --bg-secondary: #253525;
  --bg-hover: #304030;
  --bg-active: #3b4b3b;
  --text-primary: #d8e2dc;
  --text-secondary: #95a88e;
  --text-muted: #6b7d65;
  --accent: #52b788;
  --accent-hover: #40916c;
  --border: #2a3e2a;
  --danger: #e76f51;
  --danger-hover: #d05a3e;
  --favorite: #e9c46a;
  --scrollbar-thumb: #3a4e3a;
  --scrollbar-thumb-hover: #4a5e4a;
  --overlay-bg: rgba(27, 46, 27, 0.85);
  --shadow: 0 2px 12px rgba(0, 0, 0, 0.5);
}
```

- [ ] **Step 3: 创建 components.css**

```css
/* 搜索框 */
.search-container {
  padding: 12px;
  border-bottom: 1px solid var(--border);
}

.search-input {
  width: 100%;
  padding: 8px 12px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--bg-secondary);
  color: var(--text-primary);
  font-size: 13px;
  outline: none;
  transition: border-color 0.2s;
}

.search-input:focus {
  border-color: var(--accent);
}

.search-input::placeholder {
  color: var(--text-muted);
}

/* 标签栏 */
.tabs {
  display: flex;
  padding: 0 12px;
  border-bottom: 1px solid var(--border);
}

.tab-btn {
  flex: 1;
  padding: 8px;
  border: none;
  background: none;
  color: var(--text-secondary);
  font-size: 13px;
  cursor: pointer;
  border-bottom: 2px solid transparent;
  transition: color 0.2s, border-color 0.2s;
}

.tab-btn:hover {
  color: var(--text-primary);
}

.tab-btn.active {
  color: var(--accent);
  border-bottom-color: var(--accent);
}

/* 剪贴板列表 */
.clip-list {
  flex: 1;
  overflow-y: auto;
  padding: 4px 0;
}

.clip-item {
  display: flex;
  align-items: flex-start;
  padding: 10px 12px;
  cursor: pointer;
  border-bottom: 1px solid var(--border);
  transition: background 0.15s;
  position: relative;
}

.clip-item:hover {
  background: var(--bg-hover);
}

.clip-item.selected {
  background: var(--bg-active);
}

.clip-content {
  flex: 1;
  min-width: 0;
  margin-right: 8px;
}

.clip-preview {
  font-size: 13px;
  color: var(--text-primary);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  word-break: break-all;
}

.clip-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 4px;
  font-size: 11px;
  color: var(--text-muted);
}

.clip-type-icon {
  font-size: 12px;
}

.clip-favorite {
  color: var(--favorite);
}

/* 多功能按钮 */
.clip-actions-btn {
  flex-shrink: 0;
  width: 28px;
  height: 28px;
  border: none;
  background: none;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: 4px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 16px;
  opacity: 0;
  transition: opacity 0.15s, background 0.15s;
}

.clip-item:hover .clip-actions-btn {
  opacity: 1;
}

.clip-actions-btn:hover {
  background: var(--bg-active);
  color: var(--text-primary);
}

/* 操作菜单覆盖层 */
.clip-item.action-active .clip-content {
  filter: blur(3px);
  opacity: 0.3;
}

.action-menu {
  position: absolute;
  inset: 0;
  display: none;
  align-items: center;
  justify-content: center;
  gap: 16px;
  background: var(--overlay-bg);
  backdrop-filter: blur(4px);
}

.clip-item.action-active .action-menu {
  display: flex;
}

.action-btn {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  padding: 8px;
  border: none;
  background: none;
  color: var(--text-secondary);
  cursor: pointer;
  border-radius: 6px;
  font-size: 11px;
  transition: background 0.15s, color 0.15s;
}

.action-btn:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.action-btn .action-icon {
  font-size: 18px;
}

.action-btn.danger:hover {
  color: var(--danger);
}

/* 图片缩略图 */
.clip-thumbnail {
  width: 48px;
  height: 48px;
  object-fit: cover;
  border-radius: 4px;
  border: 1px solid var(--border);
}

/* 空状态 */
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: var(--text-muted);
  font-size: 14px;
  gap: 8px;
}

.empty-state-icon {
  font-size: 32px;
  opacity: 0.5;
}

/* 焦点状态 */
.clip-item:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: -2px;
}
```

- [ ] **Step 4: 提交**

```bash
git add src/styles/
git commit -m "feat: 添加 CSS 基础样式和四套主题（light/dark/ocean/forest）"
```

---

## Task 8: 前端 — HTML 结构 + api.js

**Files:**
- Modify: `src/index.html`
- Create: `src/js/api.js`

- [ ] **Step 1: 重写 index.html**

```html
<!DOCTYPE html>
<html lang="en" data-theme="light">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Clippy</title>
  <link rel="stylesheet" href="styles/themes.css">
  <link rel="stylesheet" href="styles/base.css">
  <link rel="stylesheet" href="styles/components.css">
</head>
<body>
  <div class="search-container">
    <input type="text" class="search-input" id="search-input" placeholder="Search clipboard history..." autofocus>
  </div>

  <div class="tabs">
    <button class="tab-btn active" id="tab-all" data-tab="all">All</button>
    <button class="tab-btn" id="tab-favorites" data-tab="favorites">Favorites</button>
  </div>

  <div class="clip-list" id="clip-list">
    <div class="empty-state" id="empty-state">
      <span class="empty-state-icon">📋</span>
      <span>No clipboard history yet</span>
    </div>
  </div>

  <script type="module" src="js/app.js"></script>
</body>
</html>
```

- [ ] **Step 2: 创建 api.js**

```javascript
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

export async function getClips(query, favoritesOnly, offset, limit) {
  return invoke('get_clips', {
    query: query || null,
    favoritesOnly,
    offset,
    limit,
  });
}

export async function deleteClip(id) {
  return invoke('delete_clip', { id });
}

export async function toggleFavorite(id) {
  return invoke('toggle_favorite', { id });
}

export async function clearHistory() {
  return invoke('clear_history');
}

export async function selectClip(id) {
  return invoke('select_clip', { id });
}

export async function getConfig() {
  return invoke('get_config');
}

export async function updateConfig(config) {
  return invoke('update_config', { newConfig: config });
}

export function onClipAdded(callback) {
  return listen('clip-added', (event) => callback(event.payload));
}

export function onClipRemoved(callback) {
  return listen('clip-removed', (event) => callback(event.payload));
}
```

- [ ] **Step 3: 提交**

```bash
git add src/index.html src/js/api.js
git commit -m "feat: 添加 HTML 面板结构和 Tauri IPC 封装层"
```

---

## Task 9: 前端 — clipboard-list.js + search.js + theme.js + app.js

**Files:**
- Create: `src/js/clipboard-list.js`
- Create: `src/js/search.js`
- Create: `src/js/theme.js`
- Create: `src/js/app.js`

- [ ] **Step 1: 创建 clipboard-list.js**

```javascript
import { getClips, selectClip, deleteClip, toggleFavorite } from './api.js';

const PAGE_SIZE = 20;
let listContainer;
let emptyState;
let clips = [];
let offset = 0;
let currentQuery = null;
let favoritesOnly = false;
let selectedIndex = -1;
let loading = false;
let hasMore = true;

export function init(container, empty) {
  listContainer = container;
  emptyState = empty;
  listContainer.addEventListener('scroll', onScroll);
}

export async function refresh() {
  clips = [];
  offset = 0;
  hasMore = true;
  selectedIndex = -1;
  listContainer.innerHTML = '';
  await loadMore();
  updateEmptyState();
}

export function setQuery(query) {
  currentQuery = query;
  refresh();
}

export function setFavoritesOnly(fav) {
  favoritesOnly = fav;
  refresh();
}

export function prependClip(clip) {
  // 去重：如果已存在同 id，先移除旧的
  const existingIndex = clips.findIndex(c => c.id === clip.id);
  if (existingIndex !== -1) {
    clips.splice(existingIndex, 1);
    const existingEl = listContainer.querySelector(`[data-clip-id="${clip.id}"]`);
    if (existingEl) existingEl.remove();
  }
  clips.unshift(clip);
  const el = createClipElement(clip);
  if (listContainer.firstChild && listContainer.firstChild !== emptyState) {
    listContainer.insertBefore(el, listContainer.firstChild);
  } else {
    listContainer.appendChild(el);
  }
  updateEmptyState();
}

export function removeClip(clipId) {
  const index = clips.findIndex(c => c.id === clipId);
  if (index !== -1) {
    clips.splice(index, 1);
    const el = listContainer.querySelector(`[data-clip-id="${clipId}"]`);
    if (el) el.remove();
  }
  updateEmptyState();
}

export function moveSelection(direction) {
  const items = listContainer.querySelectorAll('.clip-item');
  if (items.length === 0) return;

  if (selectedIndex >= 0 && selectedIndex < items.length) {
    items[selectedIndex].classList.remove('selected');
  }

  selectedIndex += direction;
  if (selectedIndex < 0) selectedIndex = 0;
  if (selectedIndex >= items.length) selectedIndex = items.length - 1;

  items[selectedIndex].classList.add('selected');
  items[selectedIndex].scrollIntoView({ block: 'nearest' });
}

export async function confirmSelection() {
  if (selectedIndex >= 0 && selectedIndex < clips.length) {
    await selectClip(clips[selectedIndex].id);
  }
}

async function loadMore() {
  if (loading || !hasMore) return;
  loading = true;
  try {
    const newClips = await getClips(currentQuery, favoritesOnly, offset, PAGE_SIZE);
    if (newClips.length < PAGE_SIZE) {
      hasMore = false;
    }
    for (const clip of newClips) {
      clips.push(clip);
      listContainer.appendChild(createClipElement(clip));
    }
    offset += newClips.length;
  } catch (e) {
    console.error('加载剪贴板历史失败:', e);
  }
  loading = false;
}

function onScroll() {
  const { scrollTop, scrollHeight, clientHeight } = listContainer;
  if (scrollHeight - scrollTop - clientHeight < 100) {
    loadMore();
  }
}

function createClipElement(clip) {
  const el = document.createElement('div');
  el.className = 'clip-item';
  el.dataset.clipId = clip.id;
  el.tabIndex = 0;

  // 内容区
  const content = document.createElement('div');
  content.className = 'clip-content';

  const preview = document.createElement('div');
  preview.className = 'clip-preview';

  if (clip.content_type === 'image') {
    const img = document.createElement('img');
    img.className = 'clip-thumbnail';
    if (clip.image_data) {
      const bytes = new Uint8Array(clip.image_data);
      const blob = new Blob([bytes], { type: 'image/png' });
      img.src = URL.createObjectURL(blob);
    }
    img.alt = 'Image';
    preview.appendChild(img);
  } else {
    preview.textContent = clip.text_content || clip.html_content || '';
  }

  const meta = document.createElement('div');
  meta.className = 'clip-meta';

  const typeIcon = document.createElement('span');
  typeIcon.className = 'clip-type-icon';
  typeIcon.textContent = clip.content_type === 'text' ? '📋' :
                          clip.content_type === 'html' ? '📄' : '🖼️';

  const time = document.createElement('span');
  time.textContent = formatRelativeTime(clip.created_at);

  meta.appendChild(typeIcon);
  meta.appendChild(time);

  if (clip.is_favorite) {
    const fav = document.createElement('span');
    fav.className = 'clip-favorite';
    fav.textContent = '⭐';
    meta.appendChild(fav);
  }

  content.appendChild(preview);
  content.appendChild(meta);

  // 多功能按钮
  const actionsBtn = document.createElement('button');
  actionsBtn.className = 'clip-actions-btn';
  actionsBtn.textContent = '⋯';
  actionsBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    toggleActionMenu(el, clip);
  });

  // 操作菜单
  const actionMenu = createActionMenu(clip, el);

  el.appendChild(content);
  el.appendChild(actionsBtn);
  el.appendChild(actionMenu);

  // 点击内容区 → 选中并写入剪贴板
  content.addEventListener('click', () => {
    selectClip(clip.id);
  });

  return el;
}

function createActionMenu(clip, itemEl) {
  const menu = document.createElement('div');
  menu.className = 'action-menu';

  const actions = [
    { icon: '⭐', label: 'Favorite', handler: () => handleFavorite(clip, itemEl) },
    { icon: '🗑', label: 'Delete', danger: true, handler: () => handleDelete(clip) },
    { icon: '📋', label: 'Copy', handler: () => selectClip(clip.id) },
  ];

  for (const action of actions) {
    const btn = document.createElement('button');
    btn.className = 'action-btn' + (action.danger ? ' danger' : '');
    btn.innerHTML = `<span class="action-icon">${action.icon}</span>${action.label}`;
    btn.addEventListener('click', (e) => {
      e.stopPropagation();
      itemEl.classList.remove('action-active');
      action.handler();
    });
    menu.appendChild(btn);
  }

  return menu;
}

function toggleActionMenu(itemEl, clip) {
  // 关闭其他打开的菜单
  document.querySelectorAll('.clip-item.action-active').forEach(el => {
    if (el !== itemEl) el.classList.remove('action-active');
  });
  itemEl.classList.toggle('action-active');
}

async function handleFavorite(clip, itemEl) {
  try {
    const isFav = await toggleFavorite(clip.id);
    clip.is_favorite = isFav;
    const meta = itemEl.querySelector('.clip-meta');
    const existingFav = meta.querySelector('.clip-favorite');
    if (isFav && !existingFav) {
      const fav = document.createElement('span');
      fav.className = 'clip-favorite';
      fav.textContent = '⭐';
      meta.appendChild(fav);
    } else if (!isFav && existingFav) {
      existingFav.remove();
    }
  } catch (e) {
    console.error('切换收藏失败:', e);
  }
}

async function handleDelete(clip) {
  try {
    await deleteClip(clip.id);
    removeClip(clip.id);
  } catch (e) {
    console.error('删除失败:', e);
  }
}

function updateEmptyState() {
  if (emptyState) {
    emptyState.style.display = clips.length === 0 ? 'flex' : 'none';
  }
}

function formatRelativeTime(timestamp) {
  const now = Math.floor(Date.now() / 1000);
  const diff = now - timestamp;

  if (diff < 60) return 'just now';
  if (diff < 3600) return `${Math.floor(diff / 60)} min ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} hr ago`;
  if (diff < 172800) return 'yesterday';
  return `${Math.floor(diff / 86400)} days ago`;
}
```

- [ ] **Step 2: 创建 search.js**

```javascript
let searchInput;
let debounceTimer;
let onSearchCallback;

export function init(input, onSearch) {
  searchInput = input;
  onSearchCallback = onSearch;
  searchInput.addEventListener('input', onInput);
}

export function clear() {
  if (searchInput) {
    searchInput.value = '';
    if (onSearchCallback) onSearchCallback('');
  }
}

export function focus() {
  if (searchInput) searchInput.focus();
}

function onInput(event) {
  clearTimeout(debounceTimer);
  debounceTimer = setTimeout(() => {
    if (onSearchCallback) {
      onSearchCallback(event.target.value);
    }
  }, 200);
}
```

- [ ] **Step 3: 创建 theme.js**

```javascript
import { getConfig } from './api.js';

export async function init() {
  try {
    const config = await getConfig();
    applyTheme(config.theme);
  } catch (e) {
    applyTheme('light');
  }
}

export function applyTheme(theme) {
  document.documentElement.dataset.theme = theme;
}
```

- [ ] **Step 4: 创建 app.js**

```javascript
import * as clipboardList from './clipboard-list.js';
import * as search from './search.js';
import * as theme from './theme.js';
import { onClipAdded, onClipRemoved } from './api.js';

document.addEventListener('DOMContentLoaded', async () => {
  // 初始化主题
  await theme.init();

  // 初始化剪贴板列表
  const list = document.getElementById('clip-list');
  const empty = document.getElementById('empty-state');
  clipboardList.init(list, empty);
  await clipboardList.refresh();

  // 初始化搜索
  const searchInput = document.getElementById('search-input');
  search.init(searchInput, (query) => {
    clipboardList.setQuery(query);
  });

  // 标签切换
  const tabAll = document.getElementById('tab-all');
  const tabFavorites = document.getElementById('tab-favorites');

  tabAll.addEventListener('click', () => {
    tabAll.classList.add('active');
    tabFavorites.classList.remove('active');
    clipboardList.setFavoritesOnly(false);
  });

  tabFavorites.addEventListener('click', () => {
    tabFavorites.classList.add('active');
    tabAll.classList.remove('active');
    clipboardList.setFavoritesOnly(true);
  });

  // 监听后端事件
  onClipAdded((clip) => clipboardList.prependClip(clip));
  onClipRemoved((clipId) => clipboardList.removeClip(clipId));

  // 键盘导航
  document.addEventListener('keydown', async (e) => {
    // 关闭操作菜单
    if (e.key === 'Escape') {
      const activeMenu = document.querySelector('.clip-item.action-active');
      if (activeMenu) {
        activeMenu.classList.remove('action-active');
        return;
      }
      // Escape 隐藏窗口由后端处理
    }

    // 列表导航（搜索框不聚焦时，或按上下箭头时）
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      clipboardList.moveSelection(1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      clipboardList.moveSelection(-1);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      await clipboardList.confirmSelection();
    }
  });

  // 窗口获得焦点时刷新列表并聚焦搜索框
  window.addEventListener('focus', () => {
    clipboardList.refresh();
    search.focus();
  });
});
```

- [ ] **Step 5: 提交**

```bash
git add src/js/
git commit -m "feat: 添加前端交互逻辑（列表渲染、搜索、主题、键盘导航）"
```

---

## Task 10: 端到端验证 + 修复

**Files:** All files from previous tasks

- [ ] **Step 1: 运行后端测试**

```bash
cd src-tauri && cargo test
```

Expected: 全部通过。

- [ ] **Step 2: 编译检查**

```bash
cd src-tauri && cargo clippy -- -D warnings
```

Expected: 零警告。

- [ ] **Step 3: 格式化**

```bash
cd src-tauri && cargo fmt
```

- [ ] **Step 4: 启动开发服务器**

```bash
cargo tauri dev
```

Expected: 窗口启动，按 CmdOrCtrl+Shift+V 可以切换显示/隐藏。

- [ ] **Step 5: 手动测试**

1. 复制一段文本 → 面板中应出现新条目
2. 搜索框输入关键词 → 列表应过滤
3. 点击条目 → 文本写入剪贴板 + 面板隐藏
4. 点击 ⋯ → 操作菜单显示（Favorite/Delete/Copy）
5. 切换 All/Favorites 标签
6. ↑/↓ 键盘导航 + Enter 选中

- [ ] **Step 6: 修复发现的问题**

根据测试结果修复 bug。

- [ ] **Step 7: 最终提交**

```bash
git add -A
git commit -m "fix: 端到端测试修复"
```
