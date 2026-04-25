# Research: i18n for Vanilla HTML/CSS/JS (No Framework, No Bundler)

- **Query**: Lightweight i18n approaches for Tauri v2 vanilla JS app -- JSON translations, language detection, DOM text replacement, minimal libraries
- **Scope**: mixed (internal codebase + external best practices)
- **Date**: 2026-04-25

## Current Implementation State

### Existing Hardcoded English Strings

No i18n infrastructure exists. All UI text is hardcoded in HTML and JS files:

| File | Type | Examples of Hardcoded Strings |
|---|---|---|
| `src/index.html` | HTML attributes | `placeholder="Search clipboard history..."`, button text `"All"`, `"Favorites"`, `"No clipboard items yet"` |
| `src/settings.html` | HTML text content | `"Settings"`, `"Global Shortcut"`, `"Theme"`, `"History Limit"`, `"Save"`, `"Cancel"`, option text `"Light"`, `"Dark"`, `"Ocean"`, `"Forest"` |
| `src/settings.html` | HTML attributes | `placeholder="Click to record..."`, hint text `"Click 'Record' then press..."`, `"0 = unlimited..."` |
| `src/js/settings.js` | JS strings | `"Press keys..."`, `"Record"`, `"Stop"`, `"Settings saved!"`, `"Save failed: "` |
| `src/js/clipboard-list.js` | JS strings | (would need to check, but likely contains action labels, timestamps, etc.) |
| `src/js/app.js` | JS strings | Minimal -- mostly logic, no user-facing strings |

### Design Constraints from CLAUDE.md

> - v1 前端仅英文 UI
> - 前端无框架：纯 HTML/CSS/JS，通过 `window.__TAURI__` 调用后端 IPC 命令

The project uses **no bundler** -- Tauri serves static files directly from `src/`. Any i18n solution must work as plain ES modules loaded via `<script type="module">` or as a single JS file.

### Current Module System

All JS files use ES modules (`import`/`export`). Entry points:
- `src/js/app.js` -- main window entry (loaded by `index.html`)
- `src/js/settings.js` -- settings window entry (loaded by `settings.html`)

Modules communicate via imports. No build step, no bundler, no npm.

## Findings

### 1. Architecture: JSON-Based Translation Files

The simplest approach for a no-bundler Tauri app:

```
src/
  locales/
    en.json
    zh.json
  js/
    i18n.js       <-- translation engine module
```

**Translation file format** (`en.json`):
```json
{
  "app.title": "Clippy",
  "search.placeholder": "Search clipboard history...",
  "tabs.all": "All",
  "tabs.favorites": "Favorites",
  "empty.message": "No clipboard items yet",
  "settings.title": "Settings",
  "settings.shortcut.label": "Global Shortcut",
  "settings.shortcut.placeholder": "Click to record...",
  "settings.shortcut.hint": "Click \"Record\" then press your desired key combination",
  "settings.shortcut.record": "Record",
  "settings.shortcut.stop": "Stop",
  "settings.shortcut.reset": "Reset",
  "settings.shortcut.conflict": "This shortcut may conflict with an existing shortcut",
  "settings.theme.label": "Theme",
  "settings.theme.light": "Light",
  "settings.theme.dark": "Dark",
  "settings.theme.ocean": "Ocean",
  "settings.theme.forest": "Forest",
  "settings.history.label": "History Limit",
  "settings.history.hint": "0 = unlimited. Favorites are never deleted.",
  "settings.save": "Save",
  "settings.cancel": "Cancel",
  "settings.saved": "Settings saved!",
  "settings.saveFailed": "Save failed: {error}"
}
```

**Loading JSON in a no-bundler environment**:
```javascript
// Option A: fetch (works in Tauri webview)
const translations = await fetch("/locales/en.json").then(r => r.json());

// Option B: dynamic import with assert (browser support varies)
// Not recommended for broad compatibility
```

`fetch("/locales/en.json")` works in Tauri because it serves from the frontend dist directory. The path resolves relative to the app root.

### 2. Lightweight i18n Engine (Custom, ~50 Lines)

A minimal i18n module that handles everything needed:

```javascript
// src/js/i18n.js
let currentLocale = "en";
let translations = {};
let fallbackTranslations = {};

/**
 * Initialize i18n: detect language, load translation files.
 */
export async function init() {
  const savedLocale = localStorage.getItem("clippy-locale");
  const browserLocale = navigator.language.split("-")[0]; // "en", "zh", etc.
  currentLocale = savedLocale || browserLocale || "en";
  
  // Always load English as fallback
  fallbackTranslations = await loadLocale("en");
  
  if (currentLocale !== "en") {
    try {
      translations = await loadLocale(currentLocale);
    } catch {
      translations = {};
      currentLocale = "en";
    }
  } else {
    translations = fallbackTranslations;
  }
  
  applyToDOM();
}

async function loadLocale(locale) {
  const resp = await fetch(`/locales/${locale}.json`);
  if (!resp.ok) throw new Error(`Locale ${locale} not found`);
  return resp.json();
}

/**
 * Get translated string by key, with optional interpolation.
 * @param {string} key - dot-notation key like "settings.save"
 * @param {Object} params - interpolation values, e.g. {error: "timeout"}
 * @returns {string}
 */
export function t(key, params = {}) {
  let str = translations[key] || fallbackTranslations[key] || key;
  for (const [k, v] of Object.entries(params)) {
    str = str.replace(`{${k}}`, v);
  }
  return str;
}

/**
 * Apply translations to all DOM elements with data-i18n attributes.
 */
export function applyToDOM() {
  // Text content
  document.querySelectorAll("[data-i18n]").forEach(el => {
    el.textContent = t(el.dataset.i18n);
  });
  // Placeholder attribute
  document.querySelectorAll("[data-i18n-placeholder]").forEach(el => {
    el.placeholder = t(el.dataset.i18nPlaceholder);
  });
  // Title attribute
  document.querySelectorAll("[data-i18n-title]").forEach(el => {
    el.title = t(el.dataset.i18nTitle);
  });
  // aria-label attribute
  document.querySelectorAll("[data-i18n-aria-label]").forEach(el => {
    el.setAttribute("aria-label", t(el.dataset.i18nAriaLabel));
  });
}

export function setLocale(locale) {
  localStorage.setItem("clippy-locale", locale);
  currentLocale = locale;
}

export function getLocale() {
  return currentLocale;
}
```

### 3. HTML Markup Pattern

Using `data-i18n*` attributes to mark translatable elements:

```html
<!-- Text content -->
<h1 data-i18n="settings.title">Settings</h1>

<!-- Placeholder -->
<input data-i18n-placeholder="search.placeholder" placeholder="Search clipboard history...">

<!-- Button text -->
<button data-i18n="settings.save">Save</button>

<!-- Title/tooltip -->
<button data-i18n-title="actions.delete.tooltip" title="Delete this item">X</button>

<!-- Select options -->
<select id="theme-select">
  <option value="light" data-i18n="settings.theme.light">Light</option>
  <option value="dark" data-i18n="settings.theme.dark">Dark</option>
</select>
```

The English text remains in the HTML as the default/fallback. The `data-i18n` attribute specifies the translation key. When `applyToDOM()` runs, it overwrites the text with the translated version.

**Advantage**: The app works without JS (graceful degradation), and the HTML is human-readable with the English defaults.

### 4. Language Detection and Fallback

```javascript
// Detection priority:
// 1. User preference saved in localStorage
// 2. navigator.language (browser/OS language)
// 3. Fallback to "en"

const savedLocale = localStorage.getItem("clippy-locale");
const browserLocale = navigator.language.split("-")[0]; // "zh-CN" -> "zh"
const locale = savedLocale || browserLocale || "en";
```

**`navigator.language`** in Tauri webview:
- On Linux, this reflects the system locale (`LANG` or `LANGUAGE` environment variable).
- Returns BCP 47 tags like `"en-US"`, `"zh-CN"`, `"ja"`.
- Splitting on `"-"` and taking the first part gives the ISO 639-1 code.

**Fallback chain**: `requested locale -> en -> raw key`. This ensures the app never shows blank strings even if a translation file is missing.

### 5. Dynamic Strings in JS

For strings generated in JavaScript (not in HTML), use the `t()` function directly:

```javascript
import { t } from "./i18n.js";

// In settings.js shortcut recording:
function startRecording() {
  shortcutInput.value = t("settings.shortcut.recording"); // "Press keys..."
  recordBtn.textContent = t("settings.shortcut.stop");     // "Stop"
}

// With interpolation:
showToast(t("settings.saveFailed", { error: err.message }));
// "Save failed: timeout"
```

### 6. Existing Minimal i18n Libraries (ESM-compatible)

#### Option A: Custom solution (~50 lines, as above)
- **Pros**: Zero dependencies, tiny, tailored to needs, no npm required.
- **Cons**: Must maintain it yourself.
- **Recommended for Clippy** given the "no bundler" constraint and small string count.

#### Option B: i18next (ESM build)
- Available via CDN: `https://cdn.jsdelivr.net/npm/i18next/dist/esm/i18next.bundled.min.js`
- Or download the ESM file into `src/vendor/`.
- **Pros**: Battle-tested, pluralization, nesting, interpolation, language detection plugin.
- **Cons**: ~40KB minified; overkill for <50 strings. CDN dependency violates offline-first principle of desktop app.

#### Option C: lit-translate (ESM)
- Very small (~2KB). Available as ESM.
- **Pros**: Tiny, well-designed API.
- **Cons**: Designed for Lit framework; can be adapted but awkward without Lit.

#### Option D: Rosetta (~1KB)
- GitHub: lukeed/rosetta. Ultra-minimal i18n.
- Available as ESM: `import rosetta from 'rosetta';`
- **Pros**: ~1KB, simple key-value with interpolation, no dependencies.
- **Cons**: No `data-i18n` DOM binding (must add yourself). Must be vendored (download into `src/vendor/`).

**Recommendation**: Custom solution (Option A). The string count is small (<50 keys), the requirements are straightforward (key-value lookup + interpolation + DOM binding), and vendoring a library adds complexity for marginal benefit.

### 7. Persisting Language Preference

Two options:

1. **localStorage** (simplest, frontend-only):
   ```javascript
   localStorage.setItem("clippy-locale", "zh");
   ```
   - Each window has its own localStorage if they load from different URLs. In Tauri v2, both `index.html` and `settings.html` share the same origin, so they share localStorage.
   
2. **AppConfig** (backend, synced):
   Add a `language` field to `AppConfig`:
   ```rust
   pub struct AppConfig {
       pub max_history: u32,
       pub storage_mode: String,
       pub global_shortcut: String,
       pub theme: String,
       pub language: String,  // new field
   }
   ```
   - **Pros**: Persisted alongside other settings, backed up with config file.
   - **Cons**: Requires backend round-trip on init (but `get_config` is already called).

**Recommendation**: Use the `AppConfig` approach. It's consistent with how theme is stored, and the config is already loaded on startup.

### 8. Integration Plan for Clippy

The migration path for adding i18n:

1. **Create** `src/locales/en.json` and `src/locales/zh.json`.
2. **Create** `src/js/i18n.js` module (~50 lines).
3. **Add `data-i18n*`** attributes to `index.html` and `settings.html`.
4. **Import and call `i18n.init()`** in both `app.js` and `settings.js` during `DOMContentLoaded`.
5. **Replace hardcoded JS strings** with `t()` calls.
6. **Add `language` field** to `AppConfig` in `models.rs` (with `#[serde(default)]` for backwards compatibility).
7. **Add language selector** to `settings.html`.
8. **On language change**, call `i18n.setLocale(newLang)` + `i18n.applyToDOM()` in current window, and emit event for other windows.

**String count estimate**: ~30-40 keys for both windows combined. Very manageable.

## Related Specs

| File | Description |
|---|---|
| `.trellis/spec/frontend/component-guidelines.md` | Frontend component patterns |
| `.trellis/spec/frontend/directory-structure.md` | Where to place new files |

## Caveats / Not Found

- `navigator.language` in Tauri webview on Linux reflects the system `LANG`/`LANGUAGE` env var. If these are not set, it may return `"en"` or `"en-US"` as default.
- JSON files in `src/locales/` will be included in the Tauri bundle automatically since `frontendDist` is `"../src"`. No build configuration change needed.
- Tauri's CSP in `tauri.conf.json` currently allows `default-src 'self'`, which permits `fetch("/locales/en.json")` since it's same-origin. No CSP changes needed.
- Pluralization (e.g., "1 item" vs "5 items") is not covered by the simple key-value approach. For MVP with <50 strings, this is unlikely to be needed. If required later, a simple `t_plural(key, count)` function can be added.
- Right-to-left (RTL) languages are not considered. The current app targets English and Chinese, both LTR.
