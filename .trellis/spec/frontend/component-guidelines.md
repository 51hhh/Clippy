# Component Guidelines

> How UI components are built in this project.

---

## Overview

Clippy's main UI uses **no JS framework** — it is built with vanilla HTML/CSS/JS. Screenshot editing is an isolated React/TS feature island. For vanilla modules, "components" are DOM elements created and managed by dedicated JS modules. Each module owns its DOM subtree.

---

## Component Pattern

Each UI component is a JS module that exports an `init()` function and manages its own DOM:

```javascript
// clipboard-list.js
let listContainer;

export function init(container) {
  listContainer = container;
  // Set up event listeners, render initial state
}

export function renderClips(clips) {
  listContainer.innerHTML = '';
  clips.forEach(clip => {
    listContainer.appendChild(createClipElement(clip));
  });
}

function createClipElement(clip) {
  const el = document.createElement('div');
  el.className = 'clip-item';
  el.dataset.clipId = clip.id;
  // ... build DOM
  return el;
}
```

### Wiring in app.js

```javascript
import * as clipboardList from './clipboard-list.js';
import * as search from './search.js';

document.addEventListener('DOMContentLoaded', () => {
  clipboardList.init(document.getElementById('clip-list'));
  search.init(document.getElementById('search-input'));
});
```

---

## DOM Manipulation Rules

- **Use `document.createElement()`** for dynamic content — not innerHTML with user data (XSS prevention)
- **`innerHTML` is OK** only for static templates with no user-supplied content
- **Use `textContent`** (not `innerHTML`) when inserting user-generated text
- **Use `dataset`** for attaching data to elements (e.g., `el.dataset.clipId = id`)

---

## Styling Patterns

- All styles in `src/styles/` CSS files — no inline styles in JS
- Use CSS classes for state: `.clip-item.selected`, `.clip-item.active`
- Theme colors via CSS custom properties: `var(--bg-primary)`, `var(--accent)`
- Toggle states by adding/removing CSS classes, never by manipulating `style` directly

连续内容（例如图片及其 OCR 文本）应共享最外层内容区的滚动容器。不要给下半段内容设置固定
`max-height` 和第二个 `overflow: auto`，否则鼠标滚轮会被嵌套滚动区域截获，内容也容易被挤成
不可读的小窗口。使用普通文档流排列子项，并让面板级容器统一承担滚动。

```javascript
// Good
el.classList.add('selected');

// Bad
el.style.backgroundColor = '#4a90d9';
```

---

## Accessibility

- Focusable elements need `tabindex` where native focus doesn't apply
- Keyboard navigation: ↑/↓ for list, Enter to select, Escape to close
- Use `role` and `aria-*` attributes for custom interactive elements
- Visible focus indicators via CSS `:focus-visible`

---

## Common Mistakes to Avoid

- **Don't call Tauri IPC outside `api.ts`** — breaks the typed decoupling contract
- **Don't use `innerHTML` with clip content** — user clipboard data is untrusted
- **Don't create global variables** — each module manages its own state
- **Don't manipulate styles directly in JS** — use CSS classes
