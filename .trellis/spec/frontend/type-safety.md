# Type Safety

> Type safety patterns in this project.

---

## Overview

Clippy main frontend uses **plain JavaScript**; React/TS feature islands may use TypeScript and must pass `npx tsc --noEmit`. Vanilla module type safety is achieved through consistent data shapes and JSDoc annotations where clarity helps.

---

## Data Shapes

Backend IPC returns JSON objects matching Rust structs. The canonical shapes:

```javascript
// ClipItem — returned by get_clips, clip-added event
{
  id: 1,                    // number
  content_type: "text",     // "text" | "html" | "image"
  text_content: "hello",   // string | null
  html_content: null,       // string | null
  image_data: null,         // number[] (byte array) | null
  content_hash: "abc123",  // string
  is_favorite: false,       // boolean
  created_at: 1714000000,  // number (Unix timestamp)
  byte_size: 5             // number
}

// AppConfig — returned by get_config
{
  max_history: 100,                       // number
  storage_mode: "persistent",             // "persistent" | "memory"
  global_shortcut: "CmdOrCtrl+Shift+V",  // string
  theme: "light"                          // "light" | "dark" | "ocean" | "forest"
}
```

---

## JSDoc for Complex Functions

Use JSDoc only where parameter types are non-obvious:

```javascript
/**
 * @param {string|null} query
 * @param {boolean} favoritesOnly
 * @param {number} offset
 * @param {number} limit
 * @returns {Promise<Array>}
 */
export async function getClips(query, favoritesOnly, offset, limit) { ... }
```

---

## Validation

- 普通进程内 IPC 数据可依赖 Rust serde 的类型契约；但由文件、网络或剪贴板恢复的持久化数据仍是用户可控输入。Rust 必须先校验信任边界，React/TS 在放入状态前再做防御性逐字段 schema 校验，不能用 type assertion 把 `unknown` 直接变成文档对象
- Validate user input at the UI boundary (search input sanitization)
- No runtime type checking library needed

---

## Forbidden Patterns

- Don't use `eval()` or `new Function()` — security risk
- Don't assume `image_data` is a base64 string — it's a byte array from Rust
- Don't assume all fields are present — check `content_type` before accessing type-specific fields
