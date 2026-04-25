# Backend Development Guidelines

> Best practices for backend development in this project.

---

## Overview

Clippy backend is a Tauri v2 + Rust application using SQLite (rusqlite + FTS5), arboard for clipboard access, and serde for serialization. Code lives in `src-tauri/src/` with a flat module structure.

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Directory Structure](./directory-structure.md) | Flat `src/` layout, one module per concern, AppState pattern | Filled |
| [Database Guidelines](./database-guidelines.md) | SQLite + FTS5 schema, parameter binding, sync patterns | Filled |
| [Error Handling](./error-handling.md) | thiserror + anyhow, String at IPC boundary, no unwrap on I/O | Filled |
| [Quality Guidelines](./quality-guidelines.md) | cargo fmt/clippy/test, forbidden patterns, testing requirements | Filled |
| [Logging Guidelines](./logging-guidelines.md) | log crate, Chinese messages, no PII in logs | Filled |

---

## Quick Reference

- **Language**: Rust (2021 edition)
- **Framework**: Tauri v2
- **Key crates**: rusqlite (bundled), arboard, sha2, serde, thiserror, anyhow
- **Entry**: `main.rs` → `lib.rs::run()` → Tauri Builder
- **State**: `AppState` managed via `tauri::Builder::manage()`, accessed via `State<AppState>` in commands
- **Comments**: Chinese
- **Formatting**: `cargo fmt` + `cargo clippy -- -D warnings`
