# State Management

> How state is managed in this project.

---

## Overview

Clippy uses **module-scoped variables** for state — no state management library. Each JS module manages its own state as module-level `let` variables. There is no global store.

---

## State by Module

| Module | State it owns |
|--------|---------------|
| `clipboard-list.js` | `clips[]` array, `offset`, `currentQuery`, `favoritesOnly`, `selectedIndex` |
| `search.js` | `debounceTimer`, current search value |
| `theme.js` | Current theme name |
| `app.js` | None — wiring only |

---

## Patterns

### Module-scoped state

```javascript
// clipboard-list.js
let clips = [];
let offset = 0;
let selectedIndex = -1;

export function init(container) {
  // Initialize from backend
}

export function getSelectedClip() {
  return clips[selectedIndex] ?? null;
}
```

### State flows one direction: backend → frontend

1. Backend event (`clip-added`) → `app.js` listener → calls `clipboardList.prependClip()`
2. User action → `api.js` IPC call → backend processes → optional event back

### No shared mutable state between modules

Modules communicate via function calls, not shared variables:

```javascript
// Good: app.js orchestrates
search.onSearch((query) => clipboardList.setQuery(query));

// Bad: clipboard-list.js reads search.js internals
import { currentQuery } from './search.js'; // Don't do this
```

---

## When to Refresh from Backend

- On panel show (may have missed events while hidden)
- On search query change
- On tab switch (All / Favorites)
- On `clip-added` / `clip-removed` events (incremental update)
