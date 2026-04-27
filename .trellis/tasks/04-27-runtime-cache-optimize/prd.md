# 运行时缓存优化 - 全阶段内存与性能

## 目标
降低 Clippy 运行时内存占用，减少面板打开延迟和交互卡顿。

## 用户痛点
- 整体内存占用过高（glibc arena 膨胀 + 200 条全量加载）
- 面板打开时无条件全量刷新（IPC + DOM 重建）
- 鼠标划过列表触发全量 DOM 重建

## 优化清单

### Phase 1 — 零风险快速修复（改动 < 10 行/项）

#### OPT-6: `prepare()` → `prepare_cached()`
- **文件**: `src-tauri/src/storage.rs`
- **改动**: 所有 `conn.prepare(sql)` 改为 `conn.prepare_cached(sql)`
- **收益**: rusqlite 内建 LRU 缓存，避免重复 SQL 编译
- **风险**: 极低

#### OPT-4: SQLite 启用 WAL 模式
- **文件**: `src-tauri/src/storage.rs` `init_tables()`
- **改动**: 添加 `PRAGMA journal_mode = WAL;`
- **收益**: 读写并发，减少写阻塞
- **风险**: 极低

#### OPT-10: 应用 MALLOC_ARENA_MAX 环境变量
- **文件**: `src-tauri/src/main.rs`
- **改动**: 程序启动时设置 `MALLOC_ARENA_MAX=2`
- **收益**: 预计减少 20-60MB PSS（glibc arena 膨胀）
- **风险**: 极低，仅影响 Linux

#### OPT-2: 脏标记避免无效刷新
- **文件**: `src/js/clipboard-list.js`, `src/js/app.js`
- **改动**: 添加 `_dirty` 标记，仅在 `clip-added`/`clip-removed` 事件后才 refresh
- **收益**: 减少约 50% 的无效 IPC 调用和 DOM 重建
- **风险**: 极低

#### OPT-9: mouseenter 只切换 CSS class
- **文件**: `src/js/clipboard-list.js`
- **改动**: `mouseenter` 事件只切换 `.focused` class，不调用 `render()`
- **收益**: 消除鼠标移动时的 DOM 重建
- **风险**: 极低

### Phase 2 — 中等改动

#### OPT-3: cleanup_old_entries 批量删除
- **文件**: `src-tauri/src/storage.rs`
- **改动**: 用 `DELETE FROM clips WHERE id IN (...)` 替代逐条删除；FTS 同步改为批量
- **收益**: SQL 操作减少 80%
- **风险**: 低（需测试 FTS 一致性）

#### OPT-5: insert_clip 使用 UPSERT
- **文件**: `src-tauri/src/storage.rs`
- **改动**: 用 `INSERT ... ON CONFLICT(content_hash) DO UPDATE` 替代先查后改
- **收益**: 哈希重复时 3 条 SQL → 1 条
- **风险**: 低

#### OPT-8: config Mutex 优化
- **文件**: `src-tauri/src/clipboard_watcher.rs`, `src-tauri/src/lib.rs`
- **改动**: `max_history` 改为 `Arc<AtomicU32>` 或仅在内容变化时读 config
- **收益**: 减少热路径锁竞争
- **风险**: 极低

### Phase 3 — 架构级改动

#### OPT-1: render() 差量 DOM 更新
- **文件**: `src/js/clipboard-list.js`
- **改动**: 焦点变化只切换 CSS class；新增/删除条目用 `prepend`/`removeChild` 而非 `replaceChildren()`
- **收益**: CPU 降低 60%+，消除滚动卡顿
- **风险**: 中（需重构 render 逻辑）

#### OPT-7: 虚拟滚动 / 分页加载
- **文件**: `src/js/clipboard-list.js`
- **改动**: 初始加载 30 条，滚动到底追加加载；或实现简单虚拟滚动
- **收益**: 减少 JSON 序列化和 DOM 节点数
- **风险**: 中（较大改动）

## 验收标准
- [ ] `cargo check` 无错误
- [ ] `cargo clippy -- -D warnings` 无警告
- [ ] `cargo test` 通过
- [ ] `cd src && npx vitest run` 通过
- [ ] 面板打开响应 < 100ms
- [ ] 鼠标快速划过列表无卡顿
- [ ] RSS 内存占用低于优化前
