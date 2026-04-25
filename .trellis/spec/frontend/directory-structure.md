# Directory Structure

> How frontend code is organized in this project.

---

## Overview

Clippy frontend is **pure HTML + CSS + vanilla JS** — no framework, no bundler, no Node.js build step. Files are served directly by Tauri's webview. The `src/` directory is the Tauri `frontendDist`.

---

## Directory Layout

```
src/
├── index.html                # Single HTML file — the floating panel
├── styles/
│   ├── base.css              # Reset + body/scrollbar/typography base styles
│   ├── themes.css            # CSS custom properties for all themes (light/dark/ocean/forest)
│   └── components.css        # Styles for search bar, clip list, action menu, etc.
├── js/
│   ├── app.js                # Entry point: init, event listeners, keyboard nav
│   ├── api.js                # ALL Tauri IPC calls (sole __TAURI__ coupling point)
│   ├── clipboard-list.js     # List rendering + infinite scroll
│   ├── search.js             # Search box logic + debounce
│   └── theme.js              # Theme switching logic
└── assets/                   # Icons, images (app icon, type icons)
```

---

## Module Organization

### `api.js` is the only Tauri coupling point

```javascript
// api.js — every other JS module imports from here, never touches __TAURI__ directly
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

export async function getClips(query, favoritesOnly, offset, limit) {
  return invoke('get_clips', { query, favoritesOnly, offset, limit });
}

export async function selectClip(id) {
  return invoke('select_clip', { id });
}

export function onClipAdded(callback) {
  return listen('clip-added', (event) => callback(event.payload));
}
```

This enables browser-based UI development — `api.js` can provide mock fallbacks when `__TAURI__` is not available.

### One JS file per concern

- `clipboard-list.js` — rendering clip items, infinite scroll, item selection
- `search.js` — input handling, debounce, triggering list refresh
- `theme.js` — reading/applying theme from config, toggling `data-theme`
- `app.js` — wiring everything together on DOMContentLoaded

---

## Naming Conventions

- JS files: `kebab-case.js` (e.g., `clipboard-list.js`)
- CSS files: `kebab-case.css`
- JS functions: `camelCase`
- CSS classes: `kebab-case` (e.g., `.clip-item`, `.search-input`, `.action-menu`)
- CSS custom properties: `--category-name` (e.g., `--bg-primary`, `--text-primary`, `--accent`)
- Data attributes: `data-kebab-case` (e.g., `data-theme`, `data-clip-id`)

---

## No Build Step

- No npm, no bundler, no transpilation
- Use ES modules (`<script type="module">`) for imports between JS files
- Browser-native APIs only (no polyfills needed — Tauri webview supports modern JS)
