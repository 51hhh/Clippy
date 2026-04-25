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

## Storage Modes

- **Persistent (default)**: DB file at Tauri `app_data_dir()` / `clippy.db`
- **Memory**: `:memory:` — data cleared on exit, useful for testing

---

## Naming Conventions

- Table names: `snake_case` plural (`clips`, `clips_fts`)
- Column names: `snake_case` (`content_type`, `created_at`, `is_favorite`)
- Index names: `idx_{table}_{column}` (`idx_clips_created_at`)

---

## History Limit Cleanup

On each insert, if non-favorite count exceeds `max_history`, delete oldest non-favorite entries:

```rust
// Pseudocode
let excess = non_favorite_count - max_history;
if excess > 0 {
    // Delete oldest non-favorites, keeping all favorites
    conn.execute(
        "DELETE FROM clips WHERE id IN (SELECT id FROM clips WHERE is_favorite = 0 ORDER BY created_at ASC LIMIT ?1)",
        params![excess],
    )?;
}
```

---

## Common Mistakes to Avoid

- **Never interpolate user input into SQL strings** — always use parameter binding
- **Always sync FTS on insert AND delete** — forgetting the FTS delete command causes stale search results
- **Use transactions** for multi-step operations (insert clip + update FTS + cleanup old entries)
- **Handle `UNIQUE constraint` on `content_hash`** — duplicate content should update `created_at` instead of failing
