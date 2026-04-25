# Research: Shortcut Recording in Tauri v2

- **Query**: How tauri-plugin-global-shortcut v2 handles registration, best practices for shortcut recording UI, conflict detection, Linux-specific issues
- **Scope**: mixed (internal codebase + Tauri v2 plugin documentation)
- **Date**: 2026-04-25

## Current Implementation State

### Files Involved

| File Path | Line(s) | Description |
|---|---|---|
| `src-tauri/src/lib.rs` | 64-78 | `register_shortcut()` -- registers global shortcut at startup |
| `src-tauri/src/lib.rs` | 85 | Plugin initialization: `tauri_plugin_global_shortcut::Builder::new().build()` |
| `src-tauri/src/commands.rs` | 111-148 | `update_shortcut` -- unregisters all, registers new shortcut |
| `src-tauri/src/commands.rs` | 150-160 | `check_shortcut_conflict` -- checks if shortcut is already registered |
| `src/js/settings.js` | 66-156 | Full shortcut recording UI (keydown capture, conversion, conflict check) |
| `src-tauri/Cargo.toml` | 17 | `tauri-plugin-global-shortcut = "2"` |
| `src-tauri/capabilities/default.json` | 13 | `"global-shortcut:default"` permission |

### Current Shortcut Flow

1. **Startup**: `lib.rs:127` calls `register_shortcut(app, &app_config.global_shortcut)` which calls `app.global_shortcut().on_shortcut(shortcut, callback)`.
2. **Recording**: `settings.js` captures `keydown` events, converts them to Tauri shortcut format (e.g., `"CmdOrCtrl+Shift+V"`), and displays in input field.
3. **Conflict check**: After recording, calls `check_shortcut_conflict` IPC which checks `app_handle.global_shortcut().is_registered(shortcut)`.
4. **Save**: On save, calls `update_shortcut` IPC which: unregisters all shortcuts -> registers new shortcut -> saves to config.

## Findings

### 1. tauri-plugin-global-shortcut v2 API

The plugin provides these key methods via the `GlobalShortcutExt` trait:

```rust
use tauri_plugin_global_shortcut::GlobalShortcutExt;

// Register a single shortcut with callback
app.global_shortcut().on_shortcut("CmdOrCtrl+K", move |app, shortcut, event| {
    // event has `state` field: Pressed or Released
    if event.state == ShortcutState::Pressed {
        // handle
    }
})?;

// Register multiple shortcuts
app.global_shortcut().on_shortcuts(["CmdOrCtrl+K", "CmdOrCtrl+J"], handler)?;

// Check if registered
let is_reg: bool = app.global_shortcut().is_registered("CmdOrCtrl+K");

// Unregister specific shortcut
app.global_shortcut().unregister("CmdOrCtrl+K")?;

// Unregister all
app.global_shortcut().unregister_all()?;
```

**Shortcut format string**: Modifiers are `CmdOrCtrl`, `Ctrl`, `Alt`, `Shift`, `Super`/`Meta`. Keys are standard names: `A`-`Z`, `0`-`9`, `Space`, `Tab`, `Enter`, `Escape`, `F1`-`F24`, `ArrowUp`, etc. Joined with `+`.

**ShortcutState**: The callback receives events for both `Pressed` and `Released`. Current code does not filter by state (both trigger the toggle). This could cause double-toggling on some platforms. A guard like `if event.state == ShortcutState::Pressed` is recommended.

### 2. Shortcut Recording Best Practices

#### Should we unregister all shortcuts during recording?

**Yes, temporarily unregister during recording.** Rationale:
- If the user tries to record the currently-active global shortcut (e.g., `Super+V`), the global shortcut handler will fire and toggle the main window instead of being captured by the keydown listener.
- The browser-level `keydown` event in the webview may not receive keys that are intercepted by the global shortcut system at the OS level.

**Current code does NOT unregister during recording**, which means:
- If the user presses the current shortcut (`Super+V`) while recording, the main window toggles instead of recording the key combo.
- This is a bug in the current implementation.

**Recommended fix**: 
1. Add a `pause_shortcuts` / `resume_shortcuts` IPC command pair.
2. In `startRecording()`, call `invoke("pause_shortcuts")` to unregister all shortcuts.
3. In `stopRecording()`, call `invoke("resume_shortcuts")` to re-register the saved shortcut.
4. Or: the `update_shortcut` command already unregisters all first. A simpler approach is to call `invoke("unregister_all_shortcuts")` on record start and `invoke("register_shortcut", { shortcut: savedConfig.global_shortcut })` on record cancel/stop.

#### Key-to-Shortcut Conversion

The current `keyEventToShortcut()` function in `settings.js:110-129` maps:
- `e.ctrlKey` -> `"CmdOrCtrl"` (cross-platform: Ctrl on Linux/Windows, Cmd on macOS)
- `e.altKey` -> `"Alt"`
- `e.shiftKey` -> `"Shift"`
- `e.metaKey` -> `"Super"` (this is the Windows/Super key on Linux)

**Potential issue with `e.key` values**:
- Single-character keys are uppercased (`.toUpperCase()`), which is correct.
- Space is mapped to `"Space"`, correct.
- However, some special keys may not map correctly. For example, `e.key` returns `"Backspace"`, `"Delete"`, `"Tab"`, `"Home"`, `"End"`, `"PageUp"`, `"PageDown"` -- these need to match the Tauri shortcut parser's expected names exactly.
- Function keys (`F1`-`F24`) from `e.key` return `"F1"` etc., which matches Tauri's format.

**Keys that Tauri's parser may not accept**:
- `"Backspace"` -- may need to be `"Backspace"` (should work in v2)
- `"Delete"` -- should work
- Punctuation keys: `e.key` might return `-`, `=`, `[`, `]`, etc. Tauri expects these as `"Minus"`, `"Equal"`, `"BracketLeft"`, `"BracketRight"` (matching web `KeyboardEvent.code` naming). The current code passes `e.key` raw which may fail for these.

**Improvement**: Consider using `e.code` instead of `e.key` for non-character keys, or add a mapping table for special keys.

### 3. Conflict Detection

#### App-level conflict detection (current approach)

`check_shortcut_conflict` in `commands.rs:152-160`:
```rust
pub fn check_shortcut_conflict(shortcut: String, app_handle: tauri::AppHandle) -> Result<bool, String> {
    Ok(app_handle.global_shortcut().is_registered(shortcut.as_str()))
}
```

This only checks whether the shortcut is registered **by this app**. It will always return `true` for the current shortcut and `false` for everything else (since only one shortcut is registered at a time).

**Limitation**: This does NOT detect conflicts with:
- Other applications' global shortcuts
- OS-level shortcuts (e.g., `Super+L` for lock screen, `Alt+F4` for close window)
- Desktop environment shortcuts (e.g., GNOME `Super+V` for clipboard, KDE shortcuts)

#### OS-level conflict detection

**There is no reliable cross-platform API for detecting OS-level shortcut conflicts.** Approaches:

1. **Try-register approach**: Attempt to register the shortcut. If it fails, another app/OS has claimed it. This is the most practical method. `on_shortcut()` returns `Err` if registration fails.

2. **Known blacklist**: Maintain a list of commonly-used OS shortcuts and warn the user:
   - `Alt+F4` (close window)
   - `Super+L` (lock screen)
   - `Ctrl+Alt+Delete` (system interrupt)
   - `Super+D` (show desktop)
   - `Super+Tab` (window switcher)

3. **Desktop-environment-specific queries**: On GNOME, shortcuts are stored in `gsettings` (dconf). On KDE, in `kglobalshortcutsrc`. This is complex and fragile.

**Recommended approach**: Use try-register (Pattern 1) combined with a small blacklist (Pattern 2). The try-register is already implicitly available since `on_shortcut` will error on conflict.

### 4. Linux-Specific Issues (X11 vs Wayland)

#### X11
- Global shortcuts work reliably via `XGrabKey`. The `tauri-plugin-global-shortcut` uses the system's global hotkey mechanism which works on X11.
- `Super` key (Meta) is well-supported.
- Potential issue: some window managers (i3, sway) may grab keys before the app can register them.

#### Wayland
- **Global shortcuts are restricted by design in Wayland.** The Wayland protocol intentionally prevents applications from capturing global keyboard input for security reasons.
- `tauri-plugin-global-shortcut` relies on platform-specific backends. On Wayland, it may use:
  - The `org.freedesktop.portal.GlobalShortcuts` portal (available in newer GNOME/KDE)
  - `wlr-foreign-toplevel-management` on wlroots-based compositors
  - XWayland fallback (if the app runs under XWayland)
- **Known issues on Wayland**:
  - Shortcut registration may silently fail if the portal is not available.
  - The `Super` key is often reserved by the compositor (GNOME Shell, KDE Plasma).
  - Some key combinations may not be capturable.
  
#### Practical impact for Clippy:
- Default shortcut `Super+V` is problematic on GNOME Wayland because GNOME 42+ uses `Super+V` for its own clipboard manager notification.
- Consider providing alternative defaults or detecting the desktop environment.
- The `XDG_SESSION_TYPE` environment variable can distinguish X11 vs Wayland (`"x11"` or `"wayland"`).

#### webkeyboard event capture in Tauri webview on Wayland:
- The `keydown` event in the webview works normally for shortcut recording because it captures input going to the focused webview, not global input.
- The issue is only with **registering** the shortcut globally after recording.

### 5. Event State Filtering (ShortcutState)

The current shortcut handler does not filter by `ShortcutState`:

```rust
// Current code (lib.rs:67)
app.global_shortcut()
    .on_shortcut(shortcut, move |_app, _shortcut, _event| {
        // fires on BOTH Pressed and Released
```

In Tauri v2, the callback fires for both `Pressed` and `Released` states. Without filtering, the main window could toggle twice (show on press, hide on release). This depends on whether the OS sends both events -- on Linux X11, typically only `Pressed` is sent for global shortcuts. But for robustness:

```rust
use tauri_plugin_global_shortcut::ShortcutState;

app.global_shortcut()
    .on_shortcut(shortcut, move |_app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
            // toggle window
        }
    })?;
```

## Related Specs

| File | Description |
|---|---|
| `.trellis/spec/backend/error-handling.md` | How to handle shortcut registration errors |
| `.trellis/spec/guides/cross-layer-thinking-guide.md` | IPC command patterns |

## Caveats / Not Found

- The exact behavior of `tauri-plugin-global-shortcut` v2 on Wayland depends on the compositor and portal availability. No definitive compatibility matrix was found.
- `is_registered()` only checks app-internal registration, not OS-wide conflicts. True OS-level conflict detection is not available through the plugin API.
- The current `keyEventToShortcut` function may produce invalid shortcut strings for certain special keys (punctuation, numpad keys). This needs a mapping table or validation against Tauri's parser.
- During shortcut recording, the current implementation does NOT unregister existing shortcuts, which means the active global shortcut will intercept instead of being recorded.
