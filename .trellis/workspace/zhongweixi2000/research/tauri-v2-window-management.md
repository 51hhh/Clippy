# Research: Tauri v2 Window Management APIs

- **Query**: Tauri v2 skip_taskbar, CloseRequested, WindowBuilder popup options, cross-window events
- **Scope**: internal (local Tauri v2.10.3 crate source) + external
- **Date**: 2026-04-25

## 1. `skip_taskbar` Support

### Config-Level (`tauri.conf.json`)

From `tauri-utils-2.8.3/src/config.rs` line 1778:

```rust
/// If `true`, hides the window icon from the taskbar on Windows and Linux.
#[serde(default, alias = "skip-taskbar")]
pub skip_taskbar: bool,
```

**Usage in `tauri.conf.json`:**
```json
{
  "app": {
    "windows": [{
      "skipTaskbar": true
    }]
  }
}
```

### Builder-Level (Rust API)

From `tauri-2.10.3/src/webview/webview_window.rs` lines 688-697:

```rust
/// Sets whether or not the window icon should be hidden from the taskbar.
///
/// ## Platform-specific
///
/// - **macOS**: Unsupported.
#[must_use]
pub fn skip_taskbar(mut self, skip: bool) -> Self {
    self.window_builder = self.window_builder.skip_taskbar(skip);
    self
}
```

### Runtime-Level (Dynamic Toggle)

From `tauri-2.10.3/src/webview/webview_window.rs` lines 2114-2115:

```rust
pub fn set_skip_taskbar(&self, skip: bool) -> crate::Result<()> {
    self.window.set_skip_taskbar(skip)
}
```

### Platform Support

From `tauri-runtime-wry-2.10.1/src/lib.rs` lines 1210-1225:

- **Windows + Linux** (X11/Wayland): Supported via tao's `with_skip_taskbar()`
- **macOS / iOS / Android**: No-op (unsupported)

On Linux, tao uses `_NET_WM_STATE_SKIP_TASKBAR` (X11) or equivalent Wayland protocol hints.

### Current Clippy State

`tauri.conf.json` does NOT include `skipTaskbar`. Adding it is a one-line change:

```json
{
  "app": {
    "windows": [{
      "title": "Clippy",
      "width": 380,
      "height": 500,
      "resizable": false,
      "decorations": false,
      "alwaysOnTop": true,
      "visible": false,
      "center": true,
      "skipTaskbar": true
    }]
  }
}
```

---

## 2. `CloseRequested` Event Handling

### WindowEvent Enum

From `tauri-2.10.3/src/app.rs` lines 94-147:

```rust
/// Api exposed on the `CloseRequested` event.
#[derive(Debug, Clone)]
pub struct CloseRequestApi(Sender<bool>);

impl CloseRequestApi {
    /// Prevents the window from being closed.
    pub fn prevent_close(&self) {
        self.0.send(true).unwrap();
    }
}

pub enum WindowEvent {
    Resized(PhysicalSize<u32>),
    Moved(PhysicalPosition<i32>),
    CloseRequested {
        /// An API to modify the behavior of the close requested event.
        api: CloseRequestApi,
    },
    Destroyed,
    Focused(bool),
    ScaleFactorChanged { scale_factor: f64, new_inner_size: PhysicalSize<u32> },
    DragDrop(DragDropEvent),
    ThemeChanged(Theme),
}
```

### Builder `.on_window_event()` Method

From `tauri-2.10.3/src/app.rs` lines 1895-1917:

```rust
/// Registers a window event handler for all windows.
///
/// # Examples
/// ```
/// tauri::Builder::default()
///   .on_window_event(|window, event| match event {
///     tauri::WindowEvent::Focused(focused) => {
///       // hide window whenever it loses focus
///       if !focused {
///         window.hide().unwrap();
///       }
///     }
///     _ => {}
///   });
/// ```
pub fn on_window_event<F: Fn(&Window<R>, &WindowEvent) + Send + Sync + 'static>(
    mut self,
    handler: F,
) -> Self {
    self.window_event_listeners.push(Box::new(handler));
    self
}
```

### Per-Window Event Handler

From `tauri-2.10.3/src/window/mod.rs` lines 1101-1107:

```rust
/// Registers a window event listener.
pub fn on_window_event<F: Fn(&WindowEvent) + Send + 'static>(&self, f: F) {
    self.window.dispatcher
        .on_window_event(move |event| f(&event.clone().into()));
}
```

### Pattern: Hide on Close Instead of Quit

```rust
tauri::Builder::default()
    .on_window_event(|window, event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            // Prevent the window from actually closing
            api.prevent_close();
            // Hide it instead
            window.hide().unwrap();
        }
    })
```

### Pattern: Auto-Hide on Focus Loss (like CopyQ's close_on_unfocus)

```rust
tauri::Builder::default()
    .on_window_event(|window, event| {
        match event {
            tauri::WindowEvent::Focused(false) => {
                // Only auto-hide the main popup, not the settings window
                if window.label() == "main" {
                    window.hide().unwrap();
                }
            }
            tauri::WindowEvent::CloseRequested { api, .. } => {
                if window.label() == "main" {
                    api.prevent_close();
                    window.hide().unwrap();
                }
            }
            _ => {}
        }
    })
```

---

## 3. WindowBuilder Options for Popup/Utility Window

### Available Options (from WebviewWindowBuilder)

Relevant options for a clipboard popup window:

| Method | Type | Purpose |
|---|---|---|
| `.skip_taskbar(bool)` | Builder | Hide from taskbar |
| `.decorations(bool)` | Builder | Remove window chrome |
| `.always_on_top(bool)` | Builder | Keep above other windows |
| `.visible(bool)` | Builder | Start hidden |
| `.resizable(bool)` | Builder | Prevent resize |
| `.focused(bool)` | Builder | Whether to grab focus on creation |
| `.center()` | Builder | Center on screen |
| `.inner_size(w, h)` | Builder | Set size |
| `.transparent(bool)` | Builder | Transparent background |
| `.shadow(bool)` | Builder | Window shadow |

### Runtime Methods

| Method | Purpose |
|---|---|
| `window.show()` | Make window visible |
| `window.hide()` | Hide window |
| `window.set_focus()` | Request focus |
| `window.set_skip_taskbar(bool)` | Toggle taskbar visibility at runtime |
| `window.set_always_on_top(bool)` | Toggle always-on-top at runtime |
| `window.is_visible()` | Check visibility state |
| `window.set_position(pos)` | Move window |
| `window.set_size(size)` | Resize window |
| `window.close()` | Emits CloseRequested, then destroys |

### Current Clippy Window Configuration vs Ideal

| Setting | Current | Recommended |
|---|---|---|
| `decorations` | `false` | `false` (correct) |
| `alwaysOnTop` | `true` | `true` (correct) |
| `visible` | `false` | `false` (correct) |
| `resizable` | `false` | `false` (correct) |
| `skipTaskbar` | not set | `true` (should add) |
| `center` | `true` | `true` (correct) |

---

## 4. Cross-Window Event Emission

### The `Emitter` Trait

From `tauri-2.10.3/src/lib.rs` lines 940-1030:

**Broadcast to all windows:**
```rust
use tauri::Emitter;

// From any AppHandle or Window:
app_handle.emit("config-changed", new_config)?;
```

**Target a specific window:**
```rust
use tauri::{Emitter, EventTarget};

// By label
app_handle.emit_to("main", "config-changed", new_config)?;

// By target type
app_handle.emit_to(EventTarget::webview_window("main"), "theme-changed", "dark")?;
```

**Filter-based emission:**
```rust
app_handle.emit_filter("config-changed", payload, |target| match target {
    EventTarget::WebviewWindow { label } => label == "main",
    _ => false,
})?;
```

### Frontend Listener (JavaScript)

```javascript
const { listen } = window.__TAURI__.event;

// Listen for events from Rust backend
await listen("config-changed", (event) => {
    const newConfig = event.payload;
    // Apply theme, etc.
});
```

### Current Clippy State: Settings -> Main Window Communication

**Gap identified:** `src/js/settings.js` calls `invoke("update_config", { newConfig })` but does NOT emit an event to the main window afterward. The main window only picks up config changes when the user refocuses it (via the `window.addEventListener("focus", ...)` handler in `app.js`).

**Pattern to fix this (Rust side, in `update_config` command):**

```rust
#[tauri::command]
pub fn update_config(
    new_config: AppConfig,
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    *config = new_config.clone();
    save_config(&state.config_path, &config);

    // Notify all windows about config change
    use tauri::Emitter;
    let _ = app_handle.emit("config-changed", &new_config);

    Ok(())
}
```

**Or, from JavaScript (settings window):**

```javascript
const { emit } = window.__TAURI__.event;

await invoke("update_config", { newConfig });
await emit("config-changed", newConfig);
```

---

## 5. Relevant Files in Current Clippy Codebase

| File | Relevance |
|---|---|
| `src-tauri/tauri.conf.json` | Window config - needs `skipTaskbar: true` |
| `src-tauri/src/lib.rs` | App setup - needs `.on_window_event()` for CloseRequested and Focused |
| `src-tauri/src/commands.rs` | IPC commands - `select_clip` already hides after paste; `update_config` needs event emission |
| `src/js/app.js` | Frontend - already has `window.addEventListener("focus", ...)` for refresh |
| `src/js/settings.js` | Settings - needs to emit `config-changed` event to main window |
| `src-tauri/Cargo.toml` | `tauri = "2"` with `tray-icon` feature |

---

## Caveats / Not Found

- **Wayland focus-loss reliability**: On some Wayland compositors, `Focused(false)` may fire inconsistently when the window is hidden by the compositor. A short delay (200-500ms) before acting on focus-loss is recommended.
- **`skip_taskbar` on Wayland**: tao's implementation depends on the Wayland compositor's support for `_NET_WM_STATE_SKIP_TASKBAR` via xdg-shell or compositor-specific protocols. GNOME and KDE generally support it; wlroots-based compositors may vary.
- **ThemeChanged event on Linux**: The Tauri docs note that `WindowEvent::ThemeChanged` is NOT supported on Linux. Theme changes must be propagated through custom events.
- **Multiple `on_window_event` handlers**: Tauri supports multiple listeners (they are pushed to a Vec). This means you can add separate handlers for different concerns without conflict.
