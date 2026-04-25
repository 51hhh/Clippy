# Logging Guidelines

> How logging is done in this project.

---

## Overview

Clippy uses the `log` crate facade with `env_logger` as the backend. Tauri's built-in log plugin may be used as an alternative. All log messages are written in Chinese (per project convention).

---

## Log Levels

| Level | When to use | Example |
|-------|------------|---------|
| `error!` | Unrecoverable failures that affect user-facing functionality | DB corruption, failed to open database |
| `warn!` | Recoverable issues, transient failures | Clipboard read failed (will retry), config parse fallback |
| `info!` | Key lifecycle events | App started, DB initialized, watcher started/stopped |
| `debug!` | Detailed operational info | New clip detected, FTS index updated, config loaded |
| `trace!` | Very verbose, polling loops | Each clipboard poll cycle, raw content hashes |

---

## What to Log

- Application startup and shutdown
- Database initialization and migration
- Clipboard watcher start/stop
- New clip detection (content type + byte size, not content itself)
- Configuration changes
- Error recovery actions

---

## What NOT to Log

- **Clipboard content** — may contain passwords, secrets, PII
- **Full image data** — log size/hash only
- **Content hashes at info level** — hashes can be reversed for short strings; use debug level

---

## Format

```rust
log::info!("存储引擎初始化完成，数据库路径: {}", db_path.display());
log::debug!("检测到新剪贴板内容，类型: {}, 大小: {} 字节", content_type, byte_size);
log::warn!("剪贴板读取失败，将在下次轮询重试: {}", err);
```
