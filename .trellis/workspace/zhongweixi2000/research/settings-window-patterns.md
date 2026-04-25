# Research: Settings Window Patterns in Tauri v2

- **Query**: Settings page as a normal window in Tauri v2 - window behavior, cross-window communication, event patterns
- **Scope**: mixed (internal codebase + Tauri v2 documentation)
- **Date**: 2026-04-25

## Current Implementation State

### Existing Settings Window Creation

The settings window is created dynamically in two places:

| File Path | Line(s) | Description |
|---|---|---|
| `src-tauri/src/lib.rs` | 36-51 | Tray menu "Settings" item creates/focuses settings window |
| `src-tauri/src/commands.rs` | 163-182 | `show_settings` IPC command creates/focuses settings window |

Both use identical logic:
```rust
tauri::WebviewWindowBuilder::new(
    &app_handle,
    "settings",
    tauri::WebviewUrl::App("settings.html".into()),
)
.title("Clippy Settings")
.inner_size(500.0, 400.0)
.center()
.resizable(false)
.build()
```

### Main Window vs Settings Window Configuration

| Property | Main Window (tauri.conf.json) | Settings Window (programmatic) |
|---|---|---|
| `decorations` | `false` (popup-style) | `true` (default, has title bar) |
| `resizable` | `false` | `false` |
| `alwaysOnTop` | `true` | `false` (default) |
| `visible` | `false` (hidden until toggled) | `true` (shown immediately) |
| `center` | `true` | `true` |
| `width` x `height` | 380 x 500 | 500 x 400 |

This pattern is correct: the main window is a lightweight popup (no decorations, always on top, starts hidden) while the settings window is a standard dialog (decorated, normal z-order, shown immediately).

### Settings Window Capabilities

File: `src-tauri/capabilities/default.json`
- Both `"main"` and `"settings"` are listed under `"windows"`, so they share the same permission set.
- Current permissions: `core:default`, window show/hide/focus/close, and `global-shortcut:default`.

## Findings: Settings Window Best Practices for Tauri v2

### 1. Window Behavior Patterns

**Settings as a standard decorated window** (current approach is correct):
- Settings windows should have `decorations: true` (title bar with close button) so users can close via the OS chrome.
- `resizable: false` is appropriate for a fixed-layout settings form.
- `alwaysOnTop: false` prevents the settings from obscuring other app windows.
- Should be a **singleton** -- check if already open before creating a new instance (already implemented via `get_webview_window("settings")`).

**Window close behavior considerations**:
- Tauri v2 `WebviewWindow::close()` destroys the window. The next "open settings" call must re-create it via `WebviewWindowBuilder`.
- Alternative: use `window.hide()` instead of `close()` to keep state. However, this keeps the window in memory. For a settings page that loads fresh from config each time, destroy-and-recreate is simpler.
- Current implementation: the "Cancel" button calls `getCurrentWindow().close()` from JS (`settings.js:196`), which destroys the window. This is correct since `DOMContentLoaded` reloads config from backend each time.

**Modal vs non-modal**:
- Tauri v2 does not natively support modal windows. The settings window is non-modal, which is fine for a utility app.
- If blocking the main window is desired, it must be implemented manually (e.g., overlay + disable interaction in the main window while settings is open).

### 2. Cross-Window Communication Patterns

There are three main patterns for settings-to-main-window communication in Tauri v2:

#### Pattern A: Backend Event Bus (app.emit / app.emit_to)

```rust
// Rust side: broadcast to ALL windows
app_handle.emit("config-changed", &new_config)?;

// Rust side: emit to a specific window
app_handle.emit_to("main", "config-changed", &new_config)?;
```

```javascript
// JS side (main window): listen for config changes
const { listen } = window.__TAURI__.event;
const unlisten = await listen("config-changed", (event) => {
  const newConfig = event.payload;
  applyTheme(newConfig.theme);
});
```

**When to use**: When config changes go through the backend (via IPC command). The backend command handler can emit after persisting the config. This is the most robust pattern for Clippy because `update_config` already saves to disk -- adding an emit there broadcasts the change.

#### Pattern B: emit_to from JS (window-to-window)

Tauri v2 allows JS in one window to emit events to another window:
```javascript
// From settings window JS:
const { emit } = window.__TAURI__.event;
await emit("config-changed", { theme: "dark" });
// This broadcasts to all listeners in all windows
```

**When to use**: When the change doesn't require backend processing first. Less common since settings typically need backend persistence.

#### Pattern C: Re-read Config on Focus

```javascript
// main window: re-read config when gaining focus
window.addEventListener("focus", async () => {
  const config = await invoke("get_config");
  applyTheme(config.theme);
});
```

**When to use**: Simple fallback. Already partially implemented (`app.js:92-95` refreshes the clip list on focus). However, this misses real-time updates.

#### Recommended Approach for Clippy

Combine Pattern A (event bus) with Pattern C (focus reload) for robustness:

1. In `update_config` command (commands.rs), after saving config, emit `"config-changed"` event with the new config.
2. In `main` window's `app.js`, listen for `"config-changed"` and apply theme changes immediately.
3. Keep the existing focus-reload as a fallback.

### 3. Theme Synchronization Across Windows

**Real-time preview in settings** (already implemented):
- `settings.js:61-63`: theme select `change` event immediately calls `applyTheme()` on the settings window.
- This is local to the settings window only.

**Propagating theme to main window on save**:
- Currently missing. When the user saves settings with a new theme, the main window does not update until the next window focus event (and even then, `_onWindowFocus` only calls `clipboardList.refresh()`, not `theme.init()`).
- Fix: emit `"config-changed"` from the backend on save, and listen in the main window to call `applyTheme()`.

**Real-time preview in main window** (optional, more complex):
- Emit a `"theme-preview"` event on each theme dropdown change (before save).
- Main window listens and applies temporarily.
- On cancel/close, emit `"theme-preview-cancel"` so main window reverts.
- This adds complexity; save-then-apply is simpler and sufficient for MVP.

### 4. Window Lifecycle Events

Tauri v2 provides window lifecycle events that can be listened to:

```javascript
// Listen for the settings window being destroyed
const { listen } = window.__TAURI__.event;
await listen("tauri://close-requested", (event) => {
  // cleanup before close
});
```

For the settings window, the `close-requested` event can be used to prompt "unsaved changes" warnings if desired.

## Related Specs

| File | Description |
|---|---|
| `.trellis/spec/frontend/state-management.md` | Frontend state patterns |
| `.trellis/spec/frontend/component-guidelines.md` | Component structure guidelines |
| `.trellis/spec/guides/cross-layer-thinking-guide.md` | Cross-layer data flow patterns |

## Caveats / Not Found

- Tauri v2 has no built-in "modal dialog" API. If modal behavior is needed, it must be implemented with custom CSS overlays and IPC coordination.
- The `emit_to` function targets a window by label. If the main window is hidden (not destroyed), events still reach it and will be processed when the window is shown.
- Window creation is async in the Rust side but the settings window appears synchronously to the user. No race conditions observed in the current implementation.
- The `core:event:default` permission may need to be added to capabilities if cross-window emit from JS doesn't work. Currently `core:default` may cover this, but it should be verified.
