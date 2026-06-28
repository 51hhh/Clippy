# Quality Guidelines

> Code quality standards for frontend development.

---

## Overview

Clippy main frontend is vanilla HTML/CSS/JS with Vite as the build tool. React/TS feature islands are allowed for complex tools and must pass typecheck. Quality is maintained through code conventions, automated tests, manual review, and browser DevTools testing.

---

## Forbidden Patterns

| Pattern | Why | Alternative |
|---------|-----|-------------|
| `innerHTML` with user data | XSS vulnerability | `textContent` or `createElement` |
| Global variables (`window.x`) | Namespace pollution | ES module scope |
| `var` declarations | Hoisting bugs | `const` or `let` |
| `document.write()` | Overwrites entire page | DOM manipulation methods |
| Direct `__TAURI__` access outside `api.js` | Breaks decoupling | Import from `api.js` |
| Inline `style` attributes in JS | Hard to maintain/theme | CSS classes |
| `setTimeout` for animation | Janky rendering | CSS transitions or `requestAnimationFrame` |
| `alert()` / `confirm()` / `prompt()` | Blocks thread, ugly | Custom UI elements |

---

## Required Patterns

| Pattern | Where |
|---------|-------|
| ES modules (`import`/`export`) | All JS files |
| CSS custom properties for colors | All color values reference `var(--*)` |
| `textContent` for user data | Any clip content display |
| `dataset` for element data | `data-clip-id`, `data-theme` |
| Debounce on search input | 200ms delay before IPC call |
| Keyboard navigation support | ↑/↓/Enter/Escape in clip list |

---

## Testing

- **Manual testing via `cargo tauri dev`** — verify UI in the Tauri webview
- **Browser testing** — run `cd src && npx vite` for dev server, `npx vitest` for unit tests, and `npx tsc --noEmit` for React/TS islands
- **DevTools** — use Tauri's built-in DevTools (F12) for debugging

---

## CSS Organization

1. `base.css` — resets, scrollbar, font, body
2. `themes.css` — only CSS custom property definitions
3. `components.css` — all component styles, using `var(--*)` for theming

---

## Code Review Checklist

- [ ] No `innerHTML` with dynamic content
- [ ] All Tauri calls go through `api.js`
- [ ] Colors use CSS custom properties (no hardcoded hex)
- [ ] Keyboard navigation works (↑/↓/Enter/Escape)
- [ ] UI text is in English
- [ ] Code comments in Chinese
- [ ] No global variables or `var`
- [ ] Debounce on any user input that triggers IPC
