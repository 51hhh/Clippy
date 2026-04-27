# 运行时缓存现状分析

## SQLite 层现状

| 配置项 | 当前值 | 位置 |
|--------|--------|------|
| `cache_size` | 128 页 (512KB) | storage.rs:53 |
| `temp_store` | MEMORY | storage.rs:54 |
| `journal_mode` | 未设置 (默认 DELETE) | — |
| `synchronous` | 未设置 (默认 FULL) | — |
| Prepared Statements | 未缓存 (`prepare()`) | storage.rs 全局 |

## 前端数据层现状

| 策略 | 现状 |
|------|------|
| 本地缓存 | 无 — 每次 refresh() 直接 IPC |
| 列表虚拟化 | 无 — 全量 DOM 渲染（最多 200 条） |
| 搜索防抖 | 200ms |
| 窗口聚焦刷新 | 每次 focus 全量刷新 |
| 失焦释放内存 | 有 — releaseMemory() 清空数组+DOM |

## 锁竞争分析

watcher 线程每 500ms 锁 config + storage，此时所有 IPC 命令被阻塞。
竞争窗口：insert_clip + cleanup_old_entries 期间（可能 10-50ms）。

## mouseenter 全量渲染问题

clipboard-list.js 中 mouseenter 事件调用 render() → replaceChildren()，
鼠标划过列表时每行都触发 3000+ DOM 操作。

## 关键代码位置

- storage.rs `init_tables()`: SQLite PRAGMA 配置
- storage.rs `get_clips()`: 列表查询，已排除 image_data
- storage.rs `insert_clip()`: 哈希重复时 3 条 SQL
- storage.rs `cleanup_old_entries()`: N+1 逐条删除
- clipboard_watcher.rs: 500ms 轮询，锁 config + storage
- clipboard-list.js `render()`: replaceChildren 全量重建
- clipboard-list.js mouseenter: 触发 render()
- app.js focus: 无条件 refresh()
