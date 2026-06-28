# 搜索匹配体验与测试覆盖

## Goal

修复剪贴板搜索中短输入（例如单字母）没有结果的问题，使搜索逻辑符合主流用户直觉：历史中存在包含输入内容的条目时应能被找到，同时补齐后端搜索单元测试和必要前端行为测试，降低回归风险。

## What I already know

* 用户反馈：剪贴板历史中有内容，但输入一个字母没有搜索结果。
* 前端搜索框会 debounce 后调用 `clipboardList.setQuery(q)`。
* `clipboardList.refresh()` 会调用 `getClips(_query || null, false, 0, PAGE_SIZE)`，不是只在当前已加载列表内过滤。
* 后端 `StorageEngine::get_clips()` 当前非空查询使用 SQLite FTS5 `MATCH`。
* 当前查询被双引号包裹为 phrase query，例如 `"a"`，更接近 token 精确匹配，不是 substring/prefix 模糊搜索。
* `clips_fts` 仅索引 `text_content`，不覆盖 `ocr_text`。
* 插入时会写 `clips_fts`，但旧数据/迁移/重复 upsert 可能存在 FTS 索引缺失风险。
* 现有后端搜索测试只覆盖完整词 `apple`/`banana`，没有覆盖单字母、短前缀、中文、特殊字符或索引修复。

## Assumptions (temporary)

* 用户期望搜索行为接近常见应用的“包含即可命中”，尤其短输入不应该空结果。
* 当前任务优先修复本地剪贴板历史搜索，不引入新的搜索服务或大型依赖。
* 搜索结果仍按 `created_at DESC` 排序，不做复杂相关性排序。

## Open Questions

* ~~是否把本次 MVP 限定为 text/OCR 搜索，不处理 HTML 去标签全文搜索？~~ → 已确定：本次 MVP 限定 text/OCR 搜索，HTML 去标签暂不处理。

## Requirements (evolving)

* 单字母/短输入应能匹配包含该字符/短串的历史条目。
* 较长输入应保留 FTS 性能优势，并具备 fallback，避免 FTS 语法或 tokenization 导致明显漏搜。
* 搜索不应因特殊字符、引号、控制字符等输入报错。
* 收藏模式搜索仍应只返回收藏条目。
* 已有历史/旧库应有 FTS 索引修复路径。
* 后端测试覆盖主流搜索路径和边界。

## Acceptance Criteria

* [x] 输入单字母 `a` 能命中包含 `a` 的文本历史。
* [x] 输入短前缀 `app` 能命中 `apple`/`application` 等文本历史。
* [x] 中文单字/短词能命中中文历史。
* [x] 特殊字符输入不会导致 SQLite/FTS 查询错误。
* [x] 收藏模式搜索只返回收藏条目。
* [x] 旧数据缺失 FTS 索引时初始化或查询前能修复/重建。
* [x] `cargo test` 通过（21 tests）。
* [x] 前端测试通过（306 tests，含 search.test.js 3 个新测试）。

## Definition of Done (team quality bar)

* Tests added/updated (unit/integration where appropriate)
* Lint / typecheck / CI green where practical
* Docs/notes updated if behavior changes or a new pattern is learned
* Rollout/rollback considered if risky

## Out of Scope (explicit)

* 不引入外部搜索引擎。
* 不做复杂搜索 UI、高亮、排序权重、搜索语法说明。
* 不改变剪贴板历史存储上限或分页机制。
* 不做跨设备/云同步搜索。

## Research References

* [`research/search-ux-conventions.md`](research/search-ux-conventions.md) — 短输入 LIKE 包含，长输入 FTS prefix + fallback
* [`research/sqlite-fts-search.md`](research/sqlite-fts-search.md) — LIKE 转义、FTS prefix、FTS rebuild 策略

## Decision (ADR-lite)

**Context**: 用户输入单字母时搜索无结果，因为旧实现用 FTS5 phrase query `"a"` 做精确 token 匹配。
**Decision**: 短输入(<3字符)走 LIKE `%q%`，长输入走 FTS prefix `q*` + LIKE 合并（覆盖 OCR 文本），FTS 语法错误自动降级到 LIKE。初始化时执行一次 FTS rebuild 修复旧库。
**Consequences**: 搜索行为符合直觉，FTS 索引一致性得到保证；LIKE 对大库有性能开销但剪贴板历史量级可接受。

## Technical Notes

* 关键后端文件：`src-tauri/src/storage.rs`
* 关键前端文件：`src/js/search.js`、`src/js/clipboard-list.js`、`src/js/app.js`
* 测试位置：`src-tauri/src/storage.rs` 内部测试、`src/tests/search.test.js`
* 项目约束：Rust/Tauri 后端 + vanilla JS 前端，避免不必要依赖。
* 新增 helper：`sanitize_search_query()`、`like_pattern()`、`build_fts_prefix_query()`、`rebuild_fts_once()`、`search_like()`、`search_fts_like()`
