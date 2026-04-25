# Hook Guidelines

> Data fetching and event handling patterns in this project.

---

## Overview

Clippy has no React — "hooks" in this context means **event listeners and data fetching patterns** in vanilla JS. The project uses Tauri events for backend-to-frontend push, and `invoke()` for frontend-to-backend requests.

---

## Data Fetching via Tauri IPC

All data fetching goes through `api.js`:

```javascript
// api.js
export async function getClips(query, favoritesOnly, offset, limit) {
  return invoke('get_clips', { query, favoritesOnly, offset, limit });
}
```

Callers use `async/await`:

```javascript
// clipboard-list.js
import { getClips } from './api.js';

async function loadMore() {
  const clips = await getClips(currentQuery, favOnly, offset, PAGE_SIZE);
  appendClips(clips);
  offset += clips.length;
}
```

---

## Event Listening

Backend pushes events for real-time updates:

```javascript
// api.js
export function onClipAdded(callback) {
  return listen('clip-added', (event) => callback(event.payload));
}

export function onClipRemoved(callback) {
  return listen('clip-removed', (event) => callback(event.payload));
}
```

Event listeners are registered once in `app.js` during init:

```javascript
// app.js
import { onClipAdded, onClipRemoved } from './api.js';

onClipAdded((clip) => clipboardList.prependClip(clip));
onClipRemoved((clipId) => clipboardList.removeClip(clipId));
```

---

## Debounce Pattern

Search input uses debounce to avoid excessive IPC calls:

```javascript
// search.js
let debounceTimer;

function onSearchInput(event) {
  clearTimeout(debounceTimer);
  debounceTimer = setTimeout(() => {
    triggerSearch(event.target.value);
  }, 200);
}
```

---

## Cleanup

- Store `unlisten` handles from Tauri `listen()` for cleanup if needed
- Clear `setTimeout`/`setInterval` on teardown
