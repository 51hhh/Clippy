# State Management

> How state is managed in this project.

---

## Overview

Clippy 的传统功能模块使用模块级变量；React 功能岛使用 `useSyncExternalStore` 对接模块级 store。
不引入跨窗口的全局状态库，后端仍是持久数据的权威来源。

---

## State by Module

| Module | State it owns |
|--------|---------------|
| `clipboard-list.js` | `clips[]` array, `offset`, `currentQuery`, `favoritesOnly`, `selectedIndex` |
| `react/main/clipboardStore.ts` | 当前页列表、分页、查询、面板与键盘焦点；隐藏时必须释放 |
| `react/main/translationStore.ts` | 当前条目、翻译结果、历史与请求代次 |
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
2. User action → `api.ts` typed IPC call → backend processes → optional event back

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

## 异步预览的渲染代次

- 复用同一个 DOM 容器显示不同条目时，只比较条目 id 不足以防竞态；每次真正切换或清空都要递增
  单调 `render generation`，异步工作捕获当时代次。
- 每个 `await`、Promise continuation、图片 `load/error` 等回调在修改共享 DOM、元信息或触发下一项
  昂贵工作前，都必须同时确认条目 id 和代次仍然有效。
- 同一条目的重复焦点通知应保持当前在途渲染，不能先使代次失效再因“id 未变化”提前返回；如果取消了
  另一个待处理切换，则必须完整重渲染当前条目。
- 旧代次不仅不能写 UI，也不能继续触发 OCR、复制等有副作用的 IPC。

## 可复用 WebView 的隐藏生命周期

- Tauri/GTK 的 `window.hide()` **不保证产生新的 DOM `blur`**：窗口可能早已失焦，或由
  D-Bus、托盘、关闭请求和后端命令直接隐藏。
- 大列表、图片缩略图、预览和翻译结果不能只靠 `blur` 释放。所有原生主窗口隐藏入口必须经过
  `window_controller::hide_main_window`，由 `main-window-will-hide` 通知前端执行同一份幂等清理。
- 前端自己调用 `hideCurrentWindow()` 的路径要在隐藏时显式清理；正常 `blur` 仍负责用户切走窗口的
  路径。新增隐藏入口时，两条路径都要有回归守卫。
- 释放列表后把 store 标成 dirty；下一次真正获得焦点时只从后端重新加载首屏，不能保留上一次
  10,000 条分页快照或滚动位置。
