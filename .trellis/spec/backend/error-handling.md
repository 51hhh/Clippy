# Error Handling

> How errors are handled in this project.

---

## Overview

Clippy uses `thiserror` for defining domain error types and `anyhow` for internal error propagation. Tauri IPC commands return `Result<T, String>` to the frontend — errors are converted to user-readable strings at the IPC boundary.

---

## Error Types

### Domain errors in `storage.rs` / `clipboard_watcher.rs`

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("数据库操作失败: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("内容哈希冲突: {0}")]
    DuplicateContent(String),
    #[error("配置读取失败: {0}")]
    Config(String),
}
```

---

## Error Propagation

- Within a module: use `?` operator with `thiserror` types
- Across module boundaries: use `.map_err(|e| e.to_string())` at the Tauri command level

```rust
// commands.rs — IPC boundary converts to String
#[tauri::command]
fn get_clips(state: State<AppState>, ...) -> Result<Vec<ClipItem>, String> {
    let storage = state.storage.lock().unwrap();
    storage.query_clips(...).map_err(|e| e.to_string())
}
```

---

## Error Handling in Background Threads

The clipboard watcher runs on a separate thread. Errors must not panic — log and continue:

```rust
// clipboard_watcher.rs
loop {
    match clipboard.get_text() {
        Ok(text) => { /* process */ },
        Err(e) => {
            log::warn!("剪贴板读取失败: {}", e);
            // Continue polling — transient errors are expected
        }
    }
    std::thread::sleep(poll_interval);
}
```

---

## Forbidden Patterns

- **Never `unwrap()` on user-facing or I/O operations** — use `?` or explicit error handling
- **`unwrap()` is acceptable only** for `Mutex::lock()` (panic = poisoned = unrecoverable) and compile-time guarantees
- **Never expose internal error details to the frontend** — map to user-friendly messages at the command boundary
- **Never silently swallow errors** — at minimum, log them
