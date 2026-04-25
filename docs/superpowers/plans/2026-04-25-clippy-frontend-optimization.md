# Clippy 前端优化实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix window behaviors (auto-hide, skip taskbar, close≠quit), improve settings page (shortcut recording, theme sync), and add i18n (Chinese/English).

**Architecture:** Three independent modules. Module A (Rust-only) fixes window lifecycle. Module B (Rust + JS) improves settings IPC and shortcut recording. Module C (JS-only) adds a lightweight i18n layer with JSON translation files and `data-i18n` DOM binding.

**Tech Stack:** Tauri v2.10.3, Rust 2021, tauri-plugin-global-shortcut 2.3.1, vanilla JS (ES modules), CSS custom properties.

---

## File Structure

### New files
- `src/i18n/i18n.js` — i18n core module (~60 lines): `init()`, `t()`, `applyToDOM()`
- `src/i18n/en.json` — English translations
- `src/i18n/zh-CN.json` — Chinese translations

### Modified files
- `src-tauri/tauri.conf.json` — add `skipTaskbar`
- `src-tauri/src/lib.rs` — add `on_window_event`, fix ShortcutState filtering
- `src-tauri/src/commands.rs` — add `app_handle` to `update_config`, add `pause_shortcuts`/`resume_shortcuts`, register new commands
- `src-tauri/src/models.rs` — add `language` field to `AppConfig`
- `src-tauri/src/config.rs` — update test for new field
- `src/index.html` — add `data-i18n` attributes, load i18n module
- `src/settings.html` — add `data-i18n` attributes, add language selector
- `src/js/app.js` — add `config-changed` listener, init i18n
- `src/js/settings.js` — pause/resume shortcuts during recording, init i18n
- `src/js/clipboard-list.js` — use `t()` for dynamic text
- `src/js/api.js` — add `pauseShortcuts`, `resumeShortcuts`, `onConfigChanged` exports

---

### Task 1: 任务栏隐藏 — `tauri.conf.json` 添加 `skipTaskbar`

**Files:**
- Modify: `src-tauri/tauri.conf.json`

- [ ] **Step 1: Add `skipTaskbar` to main window config**

In `src-tauri/tauri.conf.json`, add `"skipTaskbar": true` to the first window object inside `app.windows[]`:

```json
{
  "title": "Clippy",
  "width": 380,
  "height": 500,
  "resizable": false,
  "decorations": false,
  "alwaysOnTop": true,
  "visible": false,
  "center": true,
  "skipTaskbar": true
}
```

- [ ] **Step 2: Verify config is valid JSON**

Run: `cd src-tauri && cargo check 2>&1 | head -20`
Expected: compiles successfully (Tauri validates config at build time)

- [ ] **Step 3: Commit**

```bash
git add src-tauri/tauri.conf.json
git commit -m "fix: 主窗口隐藏任务栏图标（skipTaskbar）"
```

---

### Task 2: 关闭≠退出 + 失焦自动隐藏 — `lib.rs` 添加 `on_window_event`

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add `on_window_event` to the Tauri Builder chain**

In `src-tauri/src/lib.rs`, add `.on_window_event()` to the `tauri::Builder::default()` chain, right before `.invoke_handler(...)`. The handler needs to:
1. Intercept `CloseRequested` on both `main` and `settings` windows — prevent close and hide instead
2. Intercept `Focused(false)` on `main` window only — hide after 200ms delay (checking focus state before hiding)

Add this code **between** the `.setup(|app| { ... })` block and the `.invoke_handler(...)` call:

```rust
.on_window_event(|window, event| {
    match event {
        tauri::WindowEvent::CloseRequested { api, .. } => {
            // 所有窗口：关闭时只隐藏，不退出（仅托盘 Quit 才真正退出）
            api.prevent_close();
            let _ = window.hide();
        }
        tauri::WindowEvent::Focused(false) => {
            // 仅主窗口：失焦后延迟隐藏（模拟浮动面板行为）
            if window.label() == "main" {
                let window = window.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    if !window.is_focused().unwrap_or(true) {
                        let _ = window.hide();
                    }
                });
            }
        }
        _ => {}
    }
})
```

- [ ] **Step 2: Fix ShortcutState filtering in `register_shortcut`**

In the same file, the `register_shortcut` function's `on_shortcut` callback at line ~67 fires on both Pressed and Released events. Add a state check. Change the closure body inside `app.global_shortcut().on_shortcut(...)`:

Current code (line 67-77):
```rust
app.global_shortcut()
    .on_shortcut(shortcut, move |_app, _shortcut, _event| {
        if let Some(window) = handle.get_webview_window("main") {
            if window.is_visible().unwrap_or(false) {
                let _ = window.hide();
            } else {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
    })?;
```

Replace with:
```rust
app.global_shortcut()
    .on_shortcut(shortcut, move |_app, _shortcut, event| {
        use tauri_plugin_global_shortcut::ShortcutState;
        if event.state != ShortcutState::Pressed {
            return;
        }
        if let Some(window) = handle.get_webview_window("main") {
            if window.is_visible().unwrap_or(false) {
                let _ = window.hide();
            } else {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
    })?;
```

- [ ] **Step 3: Verify compilation**

Run: `cd src-tauri && cargo check 2>&1 | tail -5`
Expected: compiles with no errors

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "fix: 窗口关闭改为隐藏、主窗口失焦自动隐藏、修复快捷键双重触发"
```

---

### Task 3: `AppConfig` 添加 `language` 字段

**Files:**
- Modify: `src-tauri/src/models.rs`
- Modify: `src-tauri/src/config.rs` (update test)

- [ ] **Step 1: Add `language` field to `AppConfig`**

In `src-tauri/src/models.rs`, add a `language` field to the `AppConfig` struct:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub max_history: u32,
    pub storage_mode: String,
    pub global_shortcut: String,
    pub theme: String,
    #[serde(default = "default_language")]
    pub language: String,
}

fn default_language() -> String {
    "auto".to_string()
}
```

And update the `Default` impl:

```rust
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            max_history: 100,
            storage_mode: "persistent".to_string(),
            global_shortcut: "Super+V".to_string(),
            theme: "light".to_string(),
            language: "auto".to_string(),
        }
    }
}
```

The `#[serde(default = "default_language")]` ensures backwards compatibility — existing `config.json` files without this field will deserialize with `"auto"`.

- [ ] **Step 2: Update config test**

In `src-tauri/src/config.rs`, update the `test_default_config` test to verify the new field:

```rust
#[test]
fn test_default_config() {
    let config = AppConfig::default();
    assert_eq!(config.max_history, 100);
    assert_eq!(config.storage_mode, "persistent");
    assert_eq!(config.global_shortcut, "Super+V");
    assert_eq!(config.theme, "light");
    assert_eq!(config.language, "auto");
}
```

- [ ] **Step 3: Run tests**

Run: `cd src-tauri && cargo test 2>&1 | tail -10`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/models.rs src-tauri/src/config.rs
git commit -m "feat: AppConfig 新增 language 字段（默认 auto，向后兼容）"
```

---

### Task 4: `commands.rs` — config-changed 事件广播 + 快捷键暂停/恢复

**Files:**
- Modify: `src-tauri/src/commands.rs`

- [ ] **Step 1: Add `app_handle` to `update_config` and emit `config-changed`**

In `src-tauri/src/commands.rs`, modify the `update_config` command to accept `app_handle` and emit an event after saving:

```rust
#[tauri::command]
pub fn update_config(
    new_config: AppConfig,
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    *config = new_config;
    save_config(&state.config_path, &config);
    // 广播配置变更事件，通知所有窗口（尤其是主窗口更新主题）
    use tauri::Emitter;
    let _ = app_handle.emit("config-changed", &*config);
    Ok(())
}
```

- [ ] **Step 2: Add `pause_shortcuts` command**

Add a new command that unregisters all global shortcuts (used during shortcut recording in settings):

```rust
/// 暂停全局快捷键（录制新快捷键时调用，避免冲突）
#[tauri::command]
pub fn pause_shortcuts(app_handle: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    app_handle
        .global_shortcut()
        .unregister_all()
        .map_err(|e| e.to_string())
}
```

- [ ] **Step 3: Add `resume_shortcuts` command**

Add a command that re-registers the current shortcut from config:

```rust
/// 恢复全局快捷键（录制结束后调用）
#[tauri::command]
pub fn resume_shortcuts(app_handle: tauri::AppHandle, state: State<AppState>) -> Result<(), String> {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    let config = state.config.lock().map_err(|e| e.to_string())?;
    let shortcut_str = config.global_shortcut.clone();
    drop(config);

    let handle = app_handle.clone();
    app_handle
        .global_shortcut()
        .on_shortcut(shortcut_str.as_str(), move |_app, _shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            if let Some(window) = handle.get_webview_window("main") {
                if window.is_visible().unwrap_or(false) {
                    let _ = window.hide();
                } else {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .map_err(|e| e.to_string())
}
```

- [ ] **Step 4: Register new commands in `lib.rs`**

In `src-tauri/src/lib.rs`, add the new commands to the `invoke_handler` macro:

```rust
.invoke_handler(tauri::generate_handler![
    commands::get_clips,
    commands::delete_clip,
    commands::toggle_favorite,
    commands::clear_history,
    commands::select_clip,
    commands::get_config,
    commands::update_config,
    commands::update_shortcut,
    commands::check_shortcut_conflict,
    commands::show_settings,
    commands::pause_shortcuts,
    commands::resume_shortcuts,
])
```

- [ ] **Step 5: Fix ShortcutState filtering in `update_shortcut`**

In `commands.rs`, the `update_shortcut` command at line ~130 also has the same double-fire bug. Update its `on_shortcut` closure:

```rust
#[tauri::command]
pub fn update_shortcut(
    new_shortcut: String,
    app_handle: tauri::AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

    app_handle
        .global_shortcut()
        .unregister_all()
        .map_err(|e| e.to_string())?;

    let handle = app_handle.clone();
    app_handle
        .global_shortcut()
        .on_shortcut(new_shortcut.as_str(), move |_app, _shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            if let Some(window) = handle.get_webview_window("main") {
                if window.is_visible().unwrap_or(false) {
                    let _ = window.hide();
                } else {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .map_err(|e| e.to_string())?;

    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    config.global_shortcut = new_shortcut;
    save_config(&state.config_path, &config);

    Ok(())
}
```

- [ ] **Step 6: Verify compilation**

Run: `cd src-tauri && cargo check 2>&1 | tail -5`
Expected: compiles with no errors

- [ ] **Step 7: Run tests**

Run: `cd src-tauri && cargo test 2>&1 | tail -10`
Expected: all tests pass

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat: config-changed 事件广播 + 快捷键暂停/恢复命令 + ShortcutState 过滤"
```

---

### Task 5: `api.js` — 新增 IPC 和事件封装

**Files:**
- Modify: `src/js/api.js`

- [ ] **Step 1: Add new IPC wrappers and event listener**

Add the following exports to the bottom of `src/js/api.js`, before the file ends:

```javascript
// ── 快捷键录制支持 ────────────────────────────────────────────────────────

/**
 * 暂停全局快捷键（录制新快捷键前调用）。
 * @returns {Promise<void>}
 */
export function pauseShortcuts() {
  return invoke("pause_shortcuts");
}

/**
 * 恢复全局快捷键（录制结束后调用）。
 * @returns {Promise<void>}
 */
export function resumeShortcuts() {
  return invoke("resume_shortcuts");
}

// ── 配置变更事件 ─────────────────────────────────────────────────────────

/**
 * 订阅配置变更事件（设置页面保存后触发）。
 * @param {function(AppConfig): void} callback
 * @returns {Promise<UnlistenFn>}
 */
export function onConfigChanged(callback) {
  return listen("config-changed", (event) => callback(event.payload));
}
```

- [ ] **Step 2: Commit**

```bash
git add src/js/api.js
git commit -m "feat: api.js 新增 pauseShortcuts/resumeShortcuts/onConfigChanged"
```

---

### Task 6: i18n 核心模块 + 翻译文件

**Files:**
- Create: `src/i18n/i18n.js`
- Create: `src/i18n/en.json`
- Create: `src/i18n/zh-CN.json`

- [ ] **Step 1: Create English translation file**

Create `src/i18n/en.json`:

```json
{
  "search.placeholder": "Search clipboard history...",
  "tabs.all": "All",
  "tabs.favorites": "Favorites",
  "empty.text": "No clipboard items yet",
  "action.favorite": "☆  Favorite",
  "action.unfavorite": "★  Unfavorite",
  "action.copy": "⎘  Copy",
  "action.delete": "✕  Delete",
  "action.more": "More actions",
  "time.justNow": "just now",
  "time.minutesAgo": "{n} min ago",
  "time.hoursAgo": "{n} hr ago",
  "time.yesterday": "yesterday",
  "time.daysAgo": "{n} days ago",
  "settings.title": "Settings",
  "settings.shortcut.label": "Global Shortcut",
  "settings.shortcut.placeholder": "Click Record to set...",
  "settings.shortcut.record": "Record",
  "settings.shortcut.stop": "Stop",
  "settings.shortcut.reset": "Reset",
  "settings.shortcut.hint": "Click \"Record\" then press your desired key combination",
  "settings.shortcut.conflict": "This shortcut may conflict with an existing shortcut",
  "settings.shortcut.recording": "Press keys...",
  "settings.theme.label": "Theme",
  "settings.theme.light": "Light",
  "settings.theme.dark": "Dark",
  "settings.theme.ocean": "Ocean",
  "settings.theme.forest": "Forest",
  "settings.history.label": "History Limit",
  "settings.history.hint": "0 = unlimited. Favorites are never deleted.",
  "settings.language.label": "Language",
  "settings.language.auto": "Auto",
  "settings.language.en": "English",
  "settings.language.zhCN": "中文",
  "settings.save": "Save",
  "settings.cancel": "Cancel",
  "settings.saved": "Settings saved!",
  "settings.saveFailed": "Save failed: {error}"
}
```

- [ ] **Step 2: Create Chinese translation file**

Create `src/i18n/zh-CN.json`:

```json
{
  "search.placeholder": "搜索剪贴板历史...",
  "tabs.all": "全部",
  "tabs.favorites": "收藏",
  "empty.text": "暂无剪贴板记录",
  "action.favorite": "☆  收藏",
  "action.unfavorite": "★  取消收藏",
  "action.copy": "⎘  复制",
  "action.delete": "✕  删除",
  "action.more": "更多操作",
  "time.justNow": "刚刚",
  "time.minutesAgo": "{n} 分钟前",
  "time.hoursAgo": "{n} 小时前",
  "time.yesterday": "昨天",
  "time.daysAgo": "{n} 天前",
  "settings.title": "设置",
  "settings.shortcut.label": "全局快捷键",
  "settings.shortcut.placeholder": "点击录制以设置...",
  "settings.shortcut.record": "录制",
  "settings.shortcut.stop": "停止",
  "settings.shortcut.reset": "重置",
  "settings.shortcut.hint": "点击「录制」后按下想要的快捷键组合",
  "settings.shortcut.conflict": "此快捷键可能与已有快捷键冲突",
  "settings.shortcut.recording": "请按键...",
  "settings.theme.label": "主题",
  "settings.theme.light": "浅色",
  "settings.theme.dark": "深色",
  "settings.theme.ocean": "海洋",
  "settings.theme.forest": "森林",
  "settings.history.label": "历史上限",
  "settings.history.hint": "0 = 不限。收藏条目不受清理影响。",
  "settings.language.label": "语言",
  "settings.language.auto": "跟随系统",
  "settings.language.en": "English",
  "settings.language.zhCN": "中文",
  "settings.save": "保存",
  "settings.cancel": "取消",
  "settings.saved": "设置已保存！",
  "settings.saveFailed": "保存失败：{error}"
}
```

- [ ] **Step 3: Create i18n core module**

Create `src/i18n/i18n.js`:

```javascript
/**
 * i18n.js — 轻量国际化模块
 * 从 JSON 文件加载翻译，支持 data-i18n DOM 绑定和程序式 t() 调用。
 */

const SUPPORTED_LOCALES = ["en", "zh-CN"];
const FALLBACK_LOCALE = "en";

let _translations = {};
let _currentLocale = FALLBACK_LOCALE;

/**
 * 初始化 i18n：检测语言 → 加载翻译文件 → 应用到 DOM。
 * @param {string} configLanguage — AppConfig.language 值（"auto" | "en" | "zh-CN"）
 */
export async function init(configLanguage) {
  _currentLocale = resolveLocale(configLanguage);
  try {
    const resp = await fetch(`/i18n/${_currentLocale}.json`);
    _translations = await resp.json();
  } catch {
    if (_currentLocale !== FALLBACK_LOCALE) {
      const resp = await fetch(`/i18n/${FALLBACK_LOCALE}.json`);
      _translations = await resp.json();
      _currentLocale = FALLBACK_LOCALE;
    }
  }
  applyToDOM();
}

/**
 * 程序式翻译。
 * @param {string} key — 翻译键（如 "time.minutesAgo"）
 * @param {Record<string, string|number>} [params] — 插值参数（如 {n: 5}）
 * @returns {string}
 */
export function t(key, params) {
  let text = _translations[key] || key;
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      text = text.replace(`{${k}}`, String(v));
    }
  }
  return text;
}

/** 返回当前生效的语言代码。 */
export function currentLocale() {
  return _currentLocale;
}

/** 扫描所有 [data-i18n] 元素并替换文本/属性。 */
export function applyToDOM() {
  document.querySelectorAll("[data-i18n]").forEach((el) => {
    const key = el.dataset.i18n;
    const attr = el.dataset.i18nAttr;
    const translated = t(key);
    if (translated === key) return;
    if (attr) {
      el.setAttribute(attr, translated);
    } else {
      el.textContent = translated;
    }
  });
}

/**
 * 将配置语言值解析为实际 locale 代码。
 * @param {string} configLanguage
 * @returns {string}
 */
function resolveLocale(configLanguage) {
  if (configLanguage && configLanguage !== "auto") {
    return SUPPORTED_LOCALES.includes(configLanguage) ? configLanguage : FALLBACK_LOCALE;
  }
  const browserLang = navigator.language || "en";
  if (browserLang.startsWith("zh")) return "zh-CN";
  return FALLBACK_LOCALE;
}
```

- [ ] **Step 4: Commit**

```bash
git add src/i18n/
git commit -m "feat: 新增 i18n 模块（中英双语翻译 + data-i18n DOM 绑定）"
```

---

### Task 7: 主窗口 HTML + JS 接入 i18n 和 config-changed

**Files:**
- Modify: `src/index.html`
- Modify: `src/js/app.js`
- Modify: `src/js/clipboard-list.js`

- [ ] **Step 1: Add `data-i18n` attributes to `index.html`**

Replace the body content of `src/index.html` (lines 12-54) with:

```html
<body>
  <div id="app">
    <!-- 搜索栏 -->
    <div class="search-container">
      <input
        id="search-input"
        class="search-input"
        type="text"
        data-i18n="search.placeholder"
        data-i18n-attr="placeholder"
        placeholder="Search clipboard history..."
        autocomplete="off"
        spellcheck="false"
      />
    </div>

    <!-- 标签页 -->
    <div class="tabs" role="tablist">
      <button
        class="tab-btn active"
        role="tab"
        data-tab="all"
        data-i18n="tabs.all"
        aria-selected="true"
      >All</button>
      <button
        class="tab-btn"
        role="tab"
        data-tab="favorites"
        data-i18n="tabs.favorites"
        aria-selected="false"
      >Favorites</button>
    </div>

    <!-- 剪贴板列表 -->
    <div
      id="clip-list"
      class="clip-list"
      role="listbox"
      aria-label="Clipboard history"
    >
      <div class="empty-state" id="empty-state">
        <span class="empty-state-icon">📋</span>
        <span data-i18n="empty.text">No clipboard items yet</span>
      </div>
    </div>
  </div>

  <script type="module" src="js/app.js"></script>
</body>
```

- [ ] **Step 2: Update `app.js` to init i18n and listen for config-changed**

Replace the entire content of `src/js/app.js` with:

```javascript
/**
 * app.js — 应用入口
 * 负责模块初始化、事件总线连接、键盘导航和窗口焦点处理。
 */

import * as theme          from "./theme.js";
import * as clipboardList  from "./clipboard-list.js";
import * as search         from "./search.js";
import * as i18n           from "../i18n/i18n.js";
import { getConfig, onClipAdded, onClipRemoved, onConfigChanged } from "./api.js";

// ── DOMContentLoaded ───────────────────────────────────────────────────────
window.addEventListener("DOMContentLoaded", async () => {
  // ── 0. 加载配置（主题 + i18n 共用）──
  let config;
  try {
    config = await getConfig();
  } catch (err) {
    console.warn("配置加载失败，使用默认值:", err);
    config = { theme: "light", language: "auto" };
  }

  // ── 1. 主题 ──
  theme.applyTheme(config.theme || "light");

  // ── 2. i18n ──
  await i18n.init(config.language || "auto");

  // ── 3. 剪贴板列表 ──
  const clipListEl   = document.getElementById("clip-list");
  const emptyStateEl = document.getElementById("empty-state");
  clipboardList.init(clipListEl, emptyStateEl);
  await clipboardList.refresh();

  // ── 4. 搜索框 ──
  const searchInput = document.getElementById("search-input");
  search.init(searchInput, (query) => {
    clipboardList.setQuery(query);
  });

  // ── 5. 标签页 ──
  const tabBtns = document.querySelectorAll(".tab-btn");
  tabBtns.forEach((btn) => {
    btn.addEventListener("click", () => {
      tabBtns.forEach((b) => {
        b.classList.remove("active");
        b.setAttribute("aria-selected", "false");
      });
      btn.classList.add("active");
      btn.setAttribute("aria-selected", "true");

      const favOnly = btn.dataset.tab === "favorites";
      clipboardList.setFavoritesOnly(favOnly);
    });
  });

  // ── 6. 后端事件监听 ──
  await onClipAdded((clip) => {
    clipboardList.prependClip(clip);
  });

  await onClipRemoved((id) => {
    clipboardList.removeClip(id);
  });

  // ── 7. 配置变更监听（设置页面保存后实时应用主题和语言）──
  await onConfigChanged(async (newConfig) => {
    theme.applyTheme(newConfig.theme || "light");
    await i18n.init(newConfig.language || "auto");
  });

  // ── 8. 键盘导航 ──
  window.addEventListener("keydown", _onKeyDown);

  // ── 9. 窗口获得焦点时刷新列表并聚焦搜索框 ──
  window.addEventListener("focus", _onWindowFocus);
});

// ── 键盘处理 ───────────────────────────────────────────────────────────────

function _onKeyDown(e) {
  switch (e.key) {
    case "Escape":
      clipboardList.closeOpenMenu();
      break;

    case "ArrowUp":
      e.preventDefault();
      clipboardList.moveSelection(-1);
      break;

    case "ArrowDown":
      e.preventDefault();
      clipboardList.moveSelection(1);
      break;

    case "Enter":
      e.preventDefault();
      clipboardList.confirmSelection();
      break;

    default:
      break;
  }
}

// ── 窗口焦点 ───────────────────────────────────────────────────────────────

async function _onWindowFocus() {
  await clipboardList.refresh();
  search.focus();
}
```

- [ ] **Step 3: Update `clipboard-list.js` to use `t()` for dynamic text**

In `src/js/clipboard-list.js`, add the i18n import at the top (after the existing api.js import at line 7):

```javascript
import { t } from "../i18n/i18n.js";
```

Then update `formatRelativeTime` (around line 405) to use translation keys:

```javascript
export function formatRelativeTime(timestamp) {
  const now  = Math.floor(Date.now() / 1000);
  const diff = now - timestamp;

  if (diff < 60)           return t("time.justNow");
  if (diff < 3600)         return t("time.minutesAgo", { n: Math.floor(diff / 60) });
  if (diff < 86400)        return t("time.hoursAgo", { n: Math.floor(diff / 3600) });
  if (diff < 86400 * 2)    return t("time.yesterday");
  return t("time.daysAgo", { n: Math.floor(diff / 86400) });
}
```

Update `_createActionMenu` (around line 326) to use `t()` for button text. Replace the three button `textContent` assignments:

For the favorite button (around line 335-336):
```javascript
favBtn.textContent = clip.is_favorite ? t("action.unfavorite") : t("action.favorite");
```

And the update inside the click handler (around line 343):
```javascript
favBtn.textContent = newState ? t("action.unfavorite") : t("action.favorite");
```

For the copy button (around line 367):
```javascript
copyBtn.textContent = t("action.copy");
```

For the delete button (around line 381):
```javascript
delBtn.textContent = t("action.delete");
```

Update the actions button title (around line 289-290):
```javascript
actionsBtn.title = t("action.more");
actionsBtn.setAttribute("aria-label", t("action.more"));
```

- [ ] **Step 4: Verify no syntax errors**

Open `src/index.html` in the browser (or run `cargo tauri dev`) and check the console for import errors.

- [ ] **Step 5: Commit**

```bash
git add src/index.html src/js/app.js src/js/clipboard-list.js
git commit -m "feat: 主窗口接入 i18n + 监听 config-changed 实时更新主题和语言"
```

---

### Task 8: 设置页面 HTML + JS 优化（i18n + 快捷键录制 + 语言选择器）

**Files:**
- Modify: `src/settings.html`
- Modify: `src/js/settings.js`

- [ ] **Step 1: Update `settings.html` with `data-i18n` and language selector**

Replace the entire content of `src/settings.html`:

```html
<!DOCTYPE html>
<html lang="en" data-theme="light">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Clippy Settings</title>
  <link rel="stylesheet" href="styles/themes.css">
  <link rel="stylesheet" href="styles/base.css">
  <link rel="stylesheet" href="styles/settings.css">
</head>
<body>
  <div class="settings-container">
    <h1 class="settings-title" data-i18n="settings.title">Settings</h1>

    <div class="setting-group">
      <label class="setting-label" data-i18n="settings.shortcut.label">Global Shortcut</label>
      <div class="shortcut-recorder" id="shortcut-recorder">
        <input type="text" class="shortcut-input" id="shortcut-input" readonly
               data-i18n="settings.shortcut.placeholder" data-i18n-attr="placeholder"
               placeholder="Click Record to set...">
        <button class="btn btn-small" id="shortcut-record-btn"
                data-i18n="settings.shortcut.record">Record</button>
        <button class="btn btn-small btn-secondary" id="shortcut-clear-btn"
                data-i18n="settings.shortcut.reset">Reset</button>
      </div>
      <div class="setting-hint" id="shortcut-hint"
           data-i18n="settings.shortcut.hint">Click "Record" then press your desired key combination</div>
      <div class="setting-warning hidden" id="shortcut-warning"
           data-i18n="settings.shortcut.conflict">This shortcut may conflict with an existing shortcut</div>
    </div>

    <div class="setting-group">
      <label class="setting-label" for="theme-select" data-i18n="settings.theme.label">Theme</label>
      <select class="setting-select" id="theme-select">
        <option value="light" data-i18n="settings.theme.light">Light</option>
        <option value="dark" data-i18n="settings.theme.dark">Dark</option>
        <option value="ocean" data-i18n="settings.theme.ocean">Ocean</option>
        <option value="forest" data-i18n="settings.theme.forest">Forest</option>
      </select>
    </div>

    <div class="setting-group">
      <label class="setting-label" for="max-history-input"
             data-i18n="settings.history.label">History Limit</label>
      <input type="number" class="setting-input" id="max-history-input" min="0" max="10000" step="10">
      <div class="setting-hint" data-i18n="settings.history.hint">0 = unlimited. Favorites are never deleted.</div>
    </div>

    <div class="setting-group">
      <label class="setting-label" for="language-select"
             data-i18n="settings.language.label">Language</label>
      <select class="setting-select" id="language-select">
        <option value="auto" data-i18n="settings.language.auto">Auto</option>
        <option value="en" data-i18n="settings.language.en">English</option>
        <option value="zh-CN" data-i18n="settings.language.zhCN">中文</option>
      </select>
    </div>

    <div class="settings-actions">
      <button class="btn btn-primary" id="save-btn" data-i18n="settings.save">Save</button>
      <button class="btn btn-secondary" id="cancel-btn" data-i18n="settings.cancel">Cancel</button>
    </div>

    <div class="toast hidden" id="toast"></div>
  </div>

  <script type="module" src="js/settings.js"></script>
</body>
</html>
```

- [ ] **Step 2: Rewrite `settings.js` with i18n, pause/resume shortcuts, and language support**

Replace the entire content of `src/js/settings.js`:

```javascript
/**
 * settings.js — 设置面板逻辑
 * 独立于主窗口，通过 Tauri IPC 读写配置、录制快捷键。
 */

import * as i18n from "../i18n/i18n.js";

const { invoke } = window.__TAURI__.core;

// ── DOM 引用 ──────────────────────────────────────────────────────────────────

const shortcutInput    = document.getElementById("shortcut-input");
const recordBtn        = document.getElementById("shortcut-record-btn");
const clearBtn         = document.getElementById("shortcut-clear-btn");
const shortcutHint     = document.getElementById("shortcut-hint");
const shortcutWarning  = document.getElementById("shortcut-warning");
const themeSelect      = document.getElementById("theme-select");
const maxHistoryInput  = document.getElementById("max-history-input");
const languageSelect   = document.getElementById("language-select");
const saveBtn          = document.getElementById("save-btn");
const cancelBtn        = document.getElementById("cancel-btn");
const toast            = document.getElementById("toast");

// ── 状态 ──────────────────────────────────────────────────────────────────────

/** 从后端加载的原始配置（用于重置和对比变更） */
let savedConfig = null;

/** 是否处于快捷键录制模式 */
let isRecording = false;

// ── 初始化 ─────────────────────────────────────────────────────────────────────

document.addEventListener("DOMContentLoaded", async () => {
  try {
    savedConfig = await invoke("get_config");
    fillForm(savedConfig);
    applyTheme(savedConfig.theme || "light");
    await i18n.init(savedConfig.language || "auto");
  } catch (err) {
    console.error("加载配置失败:", err);
    await i18n.init("auto");
  }
});

/**
 * 用配置值填充表单控件。
 * @param {AppConfig} config
 */
function fillForm(config) {
  shortcutInput.value   = config.global_shortcut || "";
  themeSelect.value     = config.theme || "light";
  maxHistoryInput.value = config.max_history ?? 100;
  languageSelect.value  = config.language || "auto";
}

/**
 * 设置文档主题。
 * @param {string} theme
 */
function applyTheme(theme) {
  document.documentElement.dataset.theme = theme;
}

// ── 主题实时预览 ───────────────────────────────────────────────────────────────

themeSelect.addEventListener("change", () => {
  applyTheme(themeSelect.value);
});

// ── 语言实时预览 ───────────────────────────────────────────────────────────────

languageSelect.addEventListener("change", async () => {
  await i18n.init(languageSelect.value);
  // 重新填入录制按钮状态文本（因为 data-i18n 已刷新）
  if (isRecording) {
    recordBtn.textContent = i18n.t("settings.shortcut.stop");
  }
});

// ── 快捷键录制 ─────────────────────────────────────────────────────────────────

recordBtn.addEventListener("click", () => {
  if (isRecording) {
    stopRecording();
  } else {
    startRecording();
  }
});

clearBtn.addEventListener("click", () => {
  if (savedConfig) {
    shortcutInput.value = savedConfig.global_shortcut || "";
  }
  shortcutWarning.classList.add("hidden");
  stopRecording();
});

async function startRecording() {
  isRecording = true;
  // 暂停全局快捷键，避免录制时触发主窗口弹出
  try {
    await invoke("pause_shortcuts");
  } catch (err) {
    console.warn("暂停快捷键失败:", err);
  }
  shortcutInput.value = i18n.t("settings.shortcut.recording");
  shortcutInput.classList.add("recording");
  recordBtn.textContent = i18n.t("settings.shortcut.stop");
  shortcutWarning.classList.add("hidden");
  document.addEventListener("keydown", onKeyDown);
}

async function stopRecording() {
  isRecording = false;
  shortcutInput.classList.remove("recording");
  recordBtn.textContent = i18n.t("settings.shortcut.record");
  document.removeEventListener("keydown", onKeyDown);
  // 恢复全局快捷键
  try {
    await invoke("resume_shortcuts");
  } catch (err) {
    console.warn("恢复快捷键失败:", err);
  }
  // 如果用户没有按到有效组合，恢复原值
  if (shortcutInput.value === i18n.t("settings.shortcut.recording")) {
    shortcutInput.value = savedConfig ? savedConfig.global_shortcut : "";
  }
}

/**
 * 将 keydown 事件转为 Tauri 快捷键格式字符串。
 * @param {KeyboardEvent} e
 * @returns {string|null}
 */
function keyEventToShortcut(e) {
  const modifiers = [];
  if (e.ctrlKey)  modifiers.push("CmdOrCtrl");
  if (e.altKey)   modifiers.push("Alt");
  if (e.shiftKey) modifiers.push("Shift");
  if (e.metaKey)  modifiers.push("Super");

  const modifierKeys = ["Control", "Alt", "Shift", "Meta", "OS"];
  if (modifierKeys.includes(e.key)) return null;
  if (modifiers.length === 0) return null;

  let key = e.key;
  if (key === " ")          key = "Space";
  else if (key.length === 1) key = key.toUpperCase();

  return [...modifiers, key].join("+");
}

/**
 * keydown 事件处理：录制快捷键组合。
 * @param {KeyboardEvent} e
 */
async function onKeyDown(e) {
  e.preventDefault();
  e.stopPropagation();

  const shortcut = keyEventToShortcut(e);
  if (!shortcut) return;

  shortcutInput.value = shortcut;
  await stopRecording();

  try {
    const conflict = await invoke("check_shortcut_conflict", { shortcut });
    if (conflict) {
      shortcutWarning.classList.remove("hidden");
    } else {
      shortcutWarning.classList.add("hidden");
    }
  } catch (err) {
    console.warn("快捷键冲突检测失败:", err);
  }
}

// ── 保存 ───────────────────────────────────────────────────────────────────────

saveBtn.addEventListener("click", async () => {
  const newShortcut   = shortcutInput.value.trim();
  const newTheme      = themeSelect.value;
  const newMaxHistory = parseInt(maxHistoryInput.value, 10) || 0;
  const newLanguage   = languageSelect.value;

  try {
    if (savedConfig && newShortcut !== savedConfig.global_shortcut && newShortcut) {
      await invoke("update_shortcut", { newShortcut });
    }

    const newConfig = {
      max_history:     newMaxHistory,
      storage_mode:    savedConfig ? savedConfig.storage_mode : "persistent",
      global_shortcut: newShortcut || (savedConfig ? savedConfig.global_shortcut : "Super+V"),
      theme:           newTheme,
      language:        newLanguage,
    };

    await invoke("update_config", { newConfig });
    savedConfig = newConfig;

    showToast(i18n.t("settings.saved"));
  } catch (err) {
    console.error("保存配置失败:", err);
    showToast(i18n.t("settings.saveFailed", { error: err }));
  }
});

// ── 取消 ───────────────────────────────────────────────────────────────────────

cancelBtn.addEventListener("click", async () => {
  try {
    const { getCurrentWindow } = window.__TAURI__.window;
    await getCurrentWindow().close();
  } catch (err) {
    console.warn("关闭窗口失败:", err);
  }
});

// ── Toast 通知 ──────────────────────────────────────────────────────────────────

function showToast(message) {
  toast.textContent = message;
  toast.classList.remove("hidden");
  void toast.offsetWidth;
  toast.classList.add("show");

  setTimeout(() => {
    toast.classList.remove("show");
    setTimeout(() => {
      toast.classList.add("hidden");
    }, 300);
  }, 2000);
}
```

- [ ] **Step 3: Commit**

```bash
git add src/settings.html src/js/settings.js
git commit -m "feat: 设置页面接入 i18n + 快捷键录制暂停/恢复 + 语言选择器"
```

---

### Task 9: 设置窗口权限配置

**Files:**
- Modify: `src-tauri/capabilities/default.json`

- [ ] **Step 1: Verify settings window has needed permissions**

The settings window was added to the capabilities file at some point. Check that it includes `"core:window:allow-close"` (needed for the Cancel button's `getCurrentWindow().close()` call, which will now be intercepted by our `on_window_event` handler and converted to a hide).

Read `src-tauri/capabilities/default.json` and confirm `"settings"` is listed in the `"windows"` array and `"core:window:allow-close"` is in `"permissions"`. The current file already has both, so no changes are needed.

If for some reason they're missing, add them:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Clippy 主窗口及设置窗口权限",
  "windows": ["main", "settings"],
  "permissions": [
    "core:default",
    "core:window:allow-show",
    "core:window:allow-hide",
    "core:window:allow-set-focus",
    "core:window:allow-close",
    "global-shortcut:default"
  ]
}
```

- [ ] **Step 2: Commit (only if changes were needed)**

```bash
git add src-tauri/capabilities/default.json
git commit -m "fix: 确保设置窗口权限完整"
```

---

### Task 10: 最终集成验证

**Files:** (no file changes — verification only)

- [ ] **Step 1: Compile and run**

Run: `cd src-tauri && cargo check 2>&1 | tail -5`
Expected: no errors

Run: `cd src-tauri && cargo clippy -- -D warnings 2>&1 | tail -10`
Expected: no warnings

Run: `cd src-tauri && cargo test 2>&1 | tail -10`
Expected: all tests pass

- [ ] **Step 2: Manual testing with `cargo tauri dev`**

Run: `cargo tauri dev`

Test checklist:
1. **任务栏隐藏**: main 窗口不出现在任务栏/dock 中
2. **快捷键弹出**: 按 Super+V 弹出主窗口（仅触发一次，不双重切换）
3. **失焦隐藏**: 点击主窗口外部区域，窗口在约 200ms 后自动隐藏
4. **粘贴后隐藏**: 点击条目后窗口立即隐藏
5. **关闭≠退出**: 关闭主窗口（如果有关闭手段），应用不退出，托盘图标仍在
6. **设置窗口**: 从托盘菜单打开 Settings，窗口有边框、不可调整大小
7. **设置窗口关闭**: 点击 X 关闭设置窗口，应用不退出，再次打开秒开
8. **快捷键录制**: 点击 Record → 全局快捷键暂停 → 按组合键 → 捕获成功 → 全局快捷键恢复
9. **主题切换**: 在设置页面切换主题 → 保存 → 主窗口实时应用新主题
10. **i18n**: 切换语言为中文 → 保存 → 两个窗口均显示中文文本
11. **语言回退**: language 设为 auto → 根据系统语言自动选择

- [ ] **Step 3: Final commit (if any fixes needed)**

```bash
git add -A
git commit -m "fix: 集成测试修复"
```
