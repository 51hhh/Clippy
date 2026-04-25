# Quality Guidelines

> Code quality standards for backend development.

---

## Overview

Clippy backend follows standard Rust quality practices enforced by `cargo fmt`, `cargo clippy`, and `cargo test`.

---

## Required Checks (must pass before commit)

```bash
cd src-tauri && cargo fmt          # Format all Rust code
cd src-tauri && cargo clippy -- -D warnings   # Zero warnings policy
cd src-tauri && cargo test         # All tests pass
```

---

## Forbidden Patterns

| Pattern | Why | Alternative |
|---------|-----|-------------|
| `unwrap()` on I/O or user input | Panics crash the app | Use `?` or `match` |
| String interpolation in SQL | SQL injection | Use `params![]` binding |
| `println!()` for logging | Not captured in release | Use `log::` macros |
| `clone()` where a reference suffices | Unnecessary allocation | Pass `&` references |
| `pub` on everything | Breaks encapsulation | Default private, pub only at module boundary |
| `unsafe` blocks | Not needed for this project | Find a safe alternative |

---

## Required Patterns

| Pattern | Where |
|---------|-------|
| `#[derive(Serialize, Deserialize, Clone, Debug)]` | All IPC-facing structs in `models.rs` |
| `Arc<Mutex<T>>` for shared state | State accessed from multiple threads (storage, config) |
| Transactions for multi-step DB ops | Insert clip + FTS sync + cleanup |
| `thiserror` for domain errors | Each module's error type |

---

## Testing Requirements

- **Unit tests** for `storage.rs` — test CRUD, FTS search, dedup, history cleanup using `:memory:` SQLite
- **Unit tests** for `config.rs` — test default values, load/save, invalid input handling
- **Tests live in the same file** using `#[cfg(test)] mod tests { ... }`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_query() {
        let storage = StorageEngine::new_in_memory().unwrap();
        // ...
    }
}
```

---

## Code Review Checklist

- [ ] No `unwrap()` on fallible operations
- [ ] SQL uses parameter binding
- [ ] FTS index stays in sync with clips table
- [ ] Thread-safe state access (Arc<Mutex<>>)
- [ ] Error types use `thiserror`
- [ ] Clippy passes with zero warnings
- [ ] Comments in Chinese, UI strings in English
