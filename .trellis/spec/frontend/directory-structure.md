# Directory Structure

> How frontend code is organized in this project.

---

## Overview

Clippy main frontend is **HTML + CSS + vanilla JS**. Complex editing surfaces may live as isolated React/TypeScript feature islands, currently `src/react/capture/`. Uses Vite as dev server and bundler (`src/vite.config.mjs`). Source files in `src/` are built to `dist/`, which is the Tauri `frontendDist`.

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
│   ├── api.ts                # ALL typed Tauri IPC calls (sole coupling point)
│   ├── ipc-types.ts          # Rust serde payload contracts
│   ├── preview/              # Preview renderer modules by concern
│   ├── clipboard-list.js     # List rendering + infinite scroll
│   ├── search.js             # Search box logic + debounce
│   └── theme.js              # Theme switching logic
└── assets/                   # Icons, images (app icon, type icons)
```

---

## Module Organization

### `api.ts` is the only Tauri coupling point

```javascript
// api.ts — every other module imports typed wrappers, never calls Tauri directly
import { invoke } from "@tauri-apps/api/core";

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

This keeps the IPC contract explicit and makes Rust serde field names testable without exposing Tauri details to feature modules.

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

- Vite handles dev server and production bundling; React/TS feature islands are transpiled by Vite
- Use ES modules (`<script type="module">`) for imports between JS files
- Browser-native APIs only (no polyfills needed — Tauri webview supports modern JS)
