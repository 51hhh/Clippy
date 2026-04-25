# Directory Structure

> How backend code is organized in this project.

---

## Overview

Clippy backend is a Tauri v2 + Rust application. All Rust source lives under `src-tauri/src/`. The entry point follows Tauri v2 convention: `main.rs` calls `lib.rs::run()`, which builds and runs the Tauri app.

---

## Directory Layout

```
src-tauri/
├── src/
│   ├── main.rs              # Binary entry: calls lib::run()
│   ├── lib.rs               # Tauri Builder setup: plugins, .manage(), .invoke_handler()
│   ├── clipboard_watcher.rs # Independent thread polling clipboard via arboard
│   ├── storage.rs           # SQLite + FTS5 engine (rusqlite)
│   ├── config.rs            # JSON config read/write (serde_json)
│   ├── commands.rs          # All #[tauri::command] IPC handlers
│   └── models.rs            # Shared data structs (ClipItem, AppConfig, etc.)
├── Cargo.toml
├── tauri.conf.json
├── capabilities/
│   └── default.json         # Tauri v2 capability permissions
└── icons/
```

---

## Module Organization

- **One module per concern** — each `.rs` file owns one domain (clipboard, storage, config, IPC commands, models).
- **No nested module directories** for MVP — flat `src/` structure is sufficient at this scale.
- **`models.rs` is the shared type hub** — all data structs used across modules are defined here with `#[derive(Serialize, Deserialize, Clone, Debug)]`.
- **`commands.rs` is the IPC boundary** — all `#[tauri::command]` functions live here. Commands delegate to `storage.rs` / `clipboard_watcher.rs` / `config.rs` for actual logic.
- **`lib.rs` is the wiring layer** — registers plugins, manages state (`AppState`), and binds commands.

---

## Naming Conventions

- File names: `snake_case.rs`
- Structs/Enums: `PascalCase` (e.g., `ClipItem`, `StorageEngine`, `AppConfig`)
- Functions: `snake_case`
- Constants: `UPPER_SNAKE_CASE`
- Tauri command names: `snake_case` (matches Rust function names, auto-converted to camelCase on JS side)

---

## Key Patterns

### AppState via Tauri `.manage()`

```rust
// lib.rs
pub struct AppState {
    pub storage: Arc<Mutex<StorageEngine>>,
    pub config: Arc<Mutex<AppConfig>>,
}

// commands.rs — access via State<>
#[tauri::command]
fn get_clips(state: State<AppState>, ...) -> Result<Vec<ClipItem>, String> {
    let storage = state.storage.lock().unwrap();
    storage.get_clips(...)
}
```

### main.rs is minimal

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    clippy_lib::run()
}
```
