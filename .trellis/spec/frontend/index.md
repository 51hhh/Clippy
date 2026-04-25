# Frontend Development Guidelines

> Best practices for frontend development in this project.

---

## Overview

Clippy frontend is **pure HTML + CSS + vanilla JavaScript** — no framework, no bundler, no TypeScript. UI is a floating panel served by Tauri's webview. Multiple color themes via CSS custom properties.

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Directory Structure](./directory-structure.md) | `src/` layout, ES modules, no build step | Filled |
| [Component Guidelines](./component-guidelines.md) | Vanilla JS module pattern, DOM rules, accessibility | Filled |
| [Hook Guidelines](./hook-guidelines.md) | Tauri IPC calls, event listeners, debounce | Filled |
| [State Management](./state-management.md) | Module-scoped state, no global store | Filled |
| [Quality Guidelines](./quality-guidelines.md) | Forbidden patterns, CSS organization, review checklist | Filled |
| [Type Safety](./type-safety.md) | Data shapes from backend, JSDoc where needed | Filled |

---

## Quick Reference

- **Stack**: HTML + CSS + vanilla JS (ES modules)
- **Build**: None — files served directly by Tauri webview
- **Tauri coupling**: Only in `src/js/api.js` (sole `__TAURI__` access point)
- **Theming**: CSS custom properties in `themes.css`, toggle via `data-theme` attribute
- **UI language**: English
- **Code comments**: Chinese
- **Keyboard nav**: ↑/↓/Enter/Escape
