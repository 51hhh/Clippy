# Research: Linux Clipboard Manager Popup/Window Behavior

- **Query**: How do CopyQ, Parcellite, GPaste, Diodon, Clipman handle popup windows, taskbar hiding, tray close behavior
- **Scope**: external (CopyQ source code analysis) + internal (current Clippy codebase)
- **Date**: 2026-04-25

## 1. Floating Popup Window That Auto-Hides

### CopyQ Approach (Qt/C++ — most sophisticated)

CopyQ uses **`QEvent::WindowDeactivate`** (focus-lost) with a **configurable delay timer** to auto-hide the popup. This is the most robust pattern found.

**Key mechanism** (`src/gui/mainwindow.cpp` lines 3008-3053):

```cpp
bool MainWindow::event(QEvent *event)
{
    QEvent::Type type = event->type();

    if (m_options.closeOnUnfocus) {
        if (type == QEvent::WindowDeactivate) {
            hideWindowOnUnfocus(AppConfig().option<Config::close_on_unfocus_delay_ms>());
        } else if (
            type == QEvent::Move ||
            type == QEvent::Resize ||
            type == QEvent::DragEnter ||
            type == QEvent::DragLeave ||
            type == QEvent::DragMove
        ) {
            hideWindowOnUnfocus(AppConfig().option<Config::close_on_unfocus_extra_delay_ms>());
        }
    }
    // ...
}
```

**Timer-based auto-hide** (`src/gui/mainwindow.cpp` lines 2723-2738):

```cpp
void MainWindow::hideWindowIfNotActive()
{
    if (isVisible() && !hasDialogOpen(this) && !isAnyApplicationWindowActive()) {
        COPYQ_LOG("Auto-hiding unfocused main window");
        hideWindow();
    }
}

void MainWindow::hideWindowOnUnfocus(int intervalMsec)
{
    const int currentDelay = m_timerHideWindowIfNotActive.remainingTime();
    if (currentDelay > intervalMsec)
        return;
    m_timerHideWindowIfNotActive.start(intervalMsec);
}
```

**Configuration** (`src/common/appconfig.h`):

| Config Key | Default | Description |
|---|---|---|
| `close_on_unfocus` | `true` | Enable auto-hide on focus loss |
| `close_on_unfocus_delay_ms` | `500` | Delay before hiding after deactivate |
| `close_on_unfocus_extra_delay_ms` | `2000` | Longer delay during move/resize/drag |

**Key design choices:**
- Uses `WindowDeactivate` event, NOT mouse capture or click-outside detection
- Delay prevents accidental dismissal (e.g., user right-clicks to open context menu)
- Extra delay for move/resize/drag operations (workaround for certain window managers)
- Checks `hasDialogOpen()` and `isAnyApplicationWindowActive()` before hiding (prevents hiding when a child dialog is focused)

### CopyQ: Wayland-Specific Handling for Tray Menu

On Wayland, CopyQ removes the `Qt::Popup` flag from the tray menu and uses `Qt::Window | Qt::FramelessWindowHint | Qt::WindowStaysOnTopHint` instead (`src/gui/traymenu.cpp` lines 78-87):

```cpp
if (isWayland) {
    setWindowFlag(Qt::Popup, false);
    setWindowFlag(Qt::Window, true);
    setWindowFlag(Qt::FramelessWindowHint, true);
    setWindowFlag(Qt::WindowStaysOnTopHint, true);
}
```

This is because Wayland does not support the traditional X11 popup grab mechanism.

### GPaste Approach (GNOME Shell Extension)

GPaste integrates as a GNOME Shell extension, so it uses GNOME's native popup/panel mechanism. The popup dismisses through the Shell's built-in focus management. Not directly comparable to standalone window apps like Clippy.

### Parcellite / Clipman Approach (GTK)

Parcellite and Clipman use a GTK popup menu (similar to a right-click context menu). GTK popups automatically dismiss when the user clicks outside because GTK menus use an implicit pointer grab. However, for full window-based UIs (not just menus), GTK apps also rely on `focus-out-event` signals.

### Summary: Popup Dismiss Patterns

| Tool | Dismiss Mechanism | Notes |
|---|---|---|
| CopyQ | `WindowDeactivate` event + timer | Most robust, handles child dialogs |
| GPaste | GNOME Shell native | Not applicable to standalone apps |
| Parcellite | GTK menu popup grab | Only works for menu-style popups |
| Clipman | GTK menu popup grab | Same as Parcellite |

**Recommendation for Clippy (Tauri):** The Tauri `WindowEvent::Focused(false)` event is the direct equivalent of Qt's `WindowDeactivate`. A timer delay (300-500ms) before hiding is recommended to avoid false triggers.

---

## 2. Hiding After Paste Action

### CopyQ: Activate -> Copy -> Hide -> Focus Previous -> Paste

CopyQ's `activateCurrentItem()` (`src/gui/mainwindow.cpp` lines 3797-3828):

```cpp
void MainWindow::activateCurrentItem() {
    PlatformWindowPtr lastWindow = m_windowForMainPaste;
    const bool paste = m_options.activatePastes() && canPaste();
    const bool activateWindow = m_options.activateFocuses();

    // 1. Copy selected item to clipboard
    c->moveToClipboard();

    // 2. Hide the main window
    if (m_options.activateCloses())
        hideWindow();

    // 3. Raise the previous window
    if (lastWindow && activateWindow)
        lastWindow->raise();

    // 4. Simulate paste keystroke (Ctrl+V) into the previous window
    if (paste) {
        if (lastWindow)
            lastWindow->pasteFromClipboard();
    }
}
```

**Config options** (`src/common/appconfig.h`):

| Config Key | Description |
|---|---|
| `activate_closes` | Close main window after item activation |
| `activate_focuses` | Focus the last active window after activation |
| `activate_pastes` | Paste into the focused window after activation |

### Current Clippy Behavior

In `src-tauri/src/commands.rs` (`select_clip` command, lines 62-93):

```rust
pub fn select_clip(id: i64, app_handle: tauri::AppHandle, state: State<AppState>) -> Result<(), String> {
    // 1. Read clip text
    // 2. Write to system clipboard (with skip hash to avoid re-capturing)
    // 3. Hide the main window
    if let Some(window) = app_handle.get_webview_window("main") {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}
```

Clippy already hides after paste (step 3), but does NOT refocus the previous window or simulate a paste keystroke. CopyQ's approach is more complete.

---

## 3. Taskbar/Dock Hiding

### CopyQ: `Qt::Tool` Window Flag

CopyQ uses `Qt::Tool` to hide from the taskbar (`src/gui/mainwindow.cpp` line 3149):

```cpp
flags.set(Qt::Tool, appConfig->option<Config::hide_main_window_in_task_bar>());
```

On X11, `Qt::Tool` sets `_NET_WM_WINDOW_TYPE_UTILITY`, which tells the window manager to skip the taskbar entry.

**Other X11 hints used by clipboard managers:**
- `_NET_WM_STATE_SKIP_TASKBAR` — Directly tells WM to skip taskbar
- `_NET_WM_WINDOW_TYPE_UTILITY` — Window type that implies no taskbar entry
- `_NET_WM_WINDOW_TYPE_DIALOG` — Some WMs also skip these
- `_NET_WM_WINDOW_TYPE_DOCK` — For panel-like apps
- `_NET_WM_WINDOW_TYPE_POPUP_MENU` — Auto-dismissed, no taskbar

### Parcellite / Clipman

These use GTK popup menus which naturally don't appear in the taskbar (they use `_NET_WM_WINDOW_TYPE_POPUP_MENU` or `_NET_WM_WINDOW_TYPE_UTILITY`).

### Current Clippy Configuration

`tauri.conf.json`:
```json
{
  "windows": [{
    "decorations": false,
    "alwaysOnTop": true,
    "visible": false
  }]
}
```

Missing: `skip_taskbar` is not set. The window currently appears in the taskbar when shown.

---

## 4. Tray Close Behavior (Close = Hide, Not Quit)

### CopyQ: `closeEvent` -> `hideWindow()`

CopyQ intercepts the close event and hides instead of closing (`src/gui/mainwindow.cpp` lines 932-937):

```cpp
void MainWindow::closeEvent(QCloseEvent *event)
{
    hideWindow();
    event->accept();
    COPYQ_LOG("Got main window close event.");
}
```

The `hideWindow()` function either minimizes or hides the window depending on config, but never actually destroys it:

```cpp
void MainWindow::hideWindow()
{
    if (closeMinimizes())
        minimizeWindow();
    else
        hide();

    // Reset search and selection for next show
    if (!browseMode()) {
        enterBrowseMode();
        auto c = browser();
        if (c) c->setCurrent(0);
    }
}
```

### Quit Behavior

Quitting only happens through:
1. Tray icon "Quit" menu item -> `emit requestExit()`
2. Explicit "File > Exit" with confirmation dialog

### Current Clippy State

Clippy does NOT intercept the close event. Closing the window via Alt+F4 or the WM close button would destroy it. The app stays alive via the tray icon, but the window is gone. Tauri's `CloseRequested` event should be used to intercept this.

---

## 5. CopyQ: X11 / Wayland Workspace Handling

### Move to Current Workspace on Show

When showing the window, CopyQ moves it to the current desktop/workspace (`src/gui/geometry.cpp` lines 249-305):

**X11:**
```cpp
// Read _NET_CURRENT_DESKTOP and set _NET_WM_DESKTOP on the window
XClientMessageEvent e{};
e.type = ClientMessage;
e.message_type = atomWmDesktop;
e.data.l[0] = currentDesktop;
e.data.l[1] = 2; // source indication: direct user action
XSendEvent(display, root, False,
    SubstructureNotifyMask | SubstructureRedirectMask,
    reinterpret_cast<XEvent*>(&e));
```

**Wayland:**
```cpp
// Hide and re-show to reinitialize in compositor
w->hide();
// (caller will show again)
```

This ensures the popup appears on whichever virtual desktop the user is currently using.

---

## Caveats / Not Found

- **Diodon**: Source code not analyzed in detail; it is a GTK-based clipboard manager with GNOME integration, similar behavior to GPaste.
- **Wayland click-outside**: No clipboard manager was found to use mouse capture or click-outside detection on Wayland. All use focus-lost events. Wayland's security model restricts global input monitoring.
- **Tauri on Wayland**: Tauri uses tao (which uses gtk on Linux). Focus events should work similarly to GTK apps on both X11 and Wayland.
