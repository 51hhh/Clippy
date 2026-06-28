# SQLite/FTS5 搜索策略调研

## 结论

SQLite FTS5 适合本地全文搜索，但默认 tokenizer 不是 substring 搜索；单字母、短串、CJK、标点和非分词片段很容易与用户直觉不一致。剪贴板历史更适合“FTS + LIKE fallback”的混合策略。

## FTS5 注意点

- `MATCH '"foo"'` 是 phrase/token 匹配，不是 `%foo%`。
- Prefix 查询应使用 `foo*`，但只对 token 前缀有效，不匹配 token 中间。
- FTS 查询语法特殊字符多，用户输入必须转义或转换为安全 token。
- CJK 在默认 tokenizer 下可能不是理想分词，短中文查询需要 fallback。
- external content table 需要同步维护索引；旧数据/迁移可能需要 rebuild。

## LIKE fallback 注意点

- `LIKE '%term%' ESCAPE '\'` 更接近用户直觉。
- 需要转义 `%`、`_`、`\`，避免用户输入被当作通配符。
- 对本地剪贴板历史，短输入 LIKE 的性能通常可接受，尤其已有分页 limit。
- 可用 `COALESCE(text_content, '') || ' ' || COALESCE(ocr_text, '')` 或 OR 条件覆盖多个字段。

## FTS rebuild/backfill

- 对 external content FTS 表，SQLite 推荐 `INSERT INTO clips_fts(clips_fts) VALUES('rebuild')` 重建。
- 初始化时执行 rebuild 最简单可靠，但数据库大时有启动成本。
- 更稳妥做法：建一个轻量 schema/meta 标记，只在版本升级时 rebuild；本次 MVP 可先做安全 rebuild 或缺失检测后 rebuild。

## 测试策略

- 用内存库插入文本，验证空查询、短 substring、prefix、完整词。
- 构造中文文本，验证中文短查询 fallback。
- 输入 `%`、`_`、`"`、控制字符，确保不报错且语义安全。
- 收藏模式下验证 WHERE 条件一致。
- 人为删除/漏建 FTS 数据后调用 repair/rebuild，验证可恢复。

## 推荐 MVP

- 增加 `escape_like_query()`。
- 增加 `build_fts_prefix_query()`，只保留安全 token，生成 `token*` 查询。
- `get_clips()` 中：短查询直接 LIKE；长查询 FTS，有结果则返回，否则 LIKE fallback。
- 增加 `rebuild_fts_index()` 并在初始化后调用，或通过缺失检测触发。
