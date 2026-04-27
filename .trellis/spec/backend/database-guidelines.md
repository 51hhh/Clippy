# Database Guidelines

> Database patterns and conventions for this project.

---

## Overview

Clippy uses **SQLite** via `rusqlite` (with `bundled` feature) for local-only storage. **FTS5** virtual table provides full-text search. No ORM — raw SQL via rusqlite's parameter binding.

---

## Schema

```sql
CREATE TABLE clips (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    content_type TEXT NOT NULL,          -- 'text' | 'html' | 'image'
    text_content TEXT,                   -- plain text (for text/html types)
    html_content TEXT,                   -- HTML source (for html type)
    image_data   BLOB,                   -- PNG binary (for image type)
    content_hash TEXT NOT NULL UNIQUE,   -- SHA-256 for dedup
    is_favorite  INTEGER DEFAULT 0,
    created_at   INTEGER NOT NULL,       -- Unix timestamp
    byte_size    INTEGER NOT NULL
);

CREATE VIRTUAL TABLE clips_fts USING fts5(
    text_content,
    content='clips',
    content_rowid='id'
);

CREATE INDEX idx_clips_created_at ON clips(created_at DESC);
CREATE INDEX idx_clips_favorite ON clips(is_favorite, created_at DESC);
```

---

## Query Patterns

### Parameter Binding — Always use `?` placeholders

```rust
conn.execute(
    "INSERT INTO clips (content_type, text_content, content_hash, created_at, byte_size) VALUES (?1, ?2, ?3, ?4, ?5)",
    params![content_type, text_content, hash, timestamp, size],
)?;
```

### Prepared Statement 缓存 — 使用 `prepare_cached`

所有频繁执行的查询使用 `conn.prepare_cached()` 而非 `conn.prepare()`，利用 rusqlite 内建 LRU 缓存避免重复 SQL 编译。

```rust
let mut stmt = self.conn.prepare_cached(sql)?;
```

### UPSERT — 哈希去重用 ON CONFLICT

插入剪贴板条目使用 UPSERT 语法，避免先查后改的多次往返：

```rust
conn.execute(
    "INSERT INTO clips (...) VALUES (...)
     ON CONFLICT(content_hash) DO UPDATE SET created_at = excluded.created_at",
    params![...],
)?;
```

### FTS Search

```rust
conn.prepare(
    "SELECT c.* FROM clips c JOIN clips_fts f ON c.id = f.rowid WHERE clips_fts MATCH ?1 ORDER BY c.created_at DESC LIMIT ?2 OFFSET ?3"
)?;
```

### FTS Sync — Keep FTS index in sync on insert/delete

```rust
// After INSERT into clips:
conn.execute(
    "INSERT INTO clips_fts(rowid, text_content) VALUES (?1, ?2)",
    params![clip_id, text_content],
)?;

// Before DELETE from clips:
conn.execute(
    "INSERT INTO clips_fts(clips_fts, rowid, text_content) VALUES ('delete', ?1, ?2)",
    params![clip_id, text_content],
)?;
```

---

## SQLite PRAGMA 配置

初始化时设置以下 PRAGMA：

```sql
PRAGMA journal_mode = WAL;      -- 读写并发，减少写阻塞
PRAGMA cache_size = 128;        -- 512KB cache，控制内存
PRAGMA temp_store = MEMORY;     -- 临时表/索引在内存
```

---

## Storage Modes

- **Persistent (default)**: DB file at Tauri `app_data_dir()` / `clippy.db`
- **Memory**: `:memory:` — data cleared on exit, useful for testing

---

## Naming Conventions

- Table names: `snake_case` plural (`clips`, `clips_fts`)
- Column names: `snake_case` (`content_type`, `created_at`, `is_favorite`)
- Index names: `idx_{table}_{column}` (`idx_clips_created_at`)

---

## History Limit Cleanup — 批量删除

On each insert, if non-favorite count exceeds `max_history`, batch-delete oldest non-favorite entries:

```rust
// 1. 查出超额 id 列表
// 2. 逐条从 FTS 删除（FTS5 不支持批量 delete 命令）
// 3. 用 IN 子句一次删除主表
let sql = format!("DELETE FROM clips WHERE id IN ({})", placeholders);
conn.execute(&sql, rusqlite::params_from_iter(params))?;
```

---

## Common Mistakes to Avoid

- **Never interpolate user input into SQL strings** — always use parameter binding
- **Always sync FTS on insert AND delete** — forgetting the FTS delete command causes stale search results
- **Use transactions** for multi-step operations (insert clip + update FTS + cleanup old entries)
- **Handle `UNIQUE constraint` on `content_hash`** — use UPSERT (ON CONFLICT) 而非 try-catch 分支
- **使用 `prepare_cached`** — 避免 `prepare()` 重复编译同一 SQL
- **批量删除用 IN 子句** — 避免循环逐条删除的 N+1 模式
