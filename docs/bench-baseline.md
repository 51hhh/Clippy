# 性能基线（criterion）

基准位于 `src-tauri/benches/`，覆盖三条用户能直接感知延迟的路径：截图编解码、
剪贴板轮询的全量扫描、搜索框每次输入触发的 `get_clips`。

## 运行

```bash
cd src-tauri
cargo bench                      # 全部基准
cargo bench --bench storage_bench            # 单个文件
cargo bench -- get_clips_fts_prefix          # 按名字过滤
# 快速对比（默认 100 samples 太慢时）
cargo bench -- --warm-up-time 0.5 --measurement-time 1.5 --sample-size 20
```

`./scripts/ci-local.sh` 的 `cargo check --all-targets` / `cargo clippy --all-targets`
会编译基准但不运行它们：门禁保证基准不腐烂，跑数字是手动动作。

基准不在 bench 文件里复制实现，一律通过 `src-tauri/src/bench_support.rs` 调用生产代码——
复制出来的基准量的是副本，和生产路径分叉后没有意义。`bench_support` 只为基准存在，不是稳定 API。

`[profile.bench]` 关掉了 release 的 `lto = "fat"` 与单 codegen-unit（否则每次编译都要几分钟），
所以数字用于版本间相对对比，不等于发布二进制的绝对性能。

## 已知坑

`bench` 与 `release` 共用 `target/release/`。如果那里躺着 `panic = "abort"` 的 release 产物
（跑过 `cargo tauri build`），`cargo bench` 会报 `requires panic strategy abort` 链接冲突。
`cargo clean --release` 后重跑即可。

## 首次基线

2026-08-29，Intel Core Ultra 5 125H / 18 线程 / 30 GiB / rustc 1.98.0，
`--sample-size 20 --measurement-time 1.5`，取 criterion 的中位数：

| 基准 | 中位数 | 说明 |
|---|---|---|
| `encode_png_1080p` | 77.1 ms | 1080p 渐变帧，`CompressionType::Fast` |
| `png_dimensions_1080p` | 19.6 ms | 实现是完整解码后取尺寸 |
| `decode_png_base64_1080p` | 1.27 ms | 前端回传的 data URL 解码 |
| `compute_hash_1mib` | 1.56 ms | 1 MiB 文本的 SHA-256 去重哈希 |
| `is_sensitive_text_worst_case_100kib` | 31.2 µs | 不命中前缀，走整段转小写 |
| `is_sensitive_text_prefix_hit` | 4.2 ns | 命中 `sk-` 前缀提前返回 |
| `strip_html_tags_1mib` | 1.41 ms | 逐字符扫描 1 MiB HTML |
| `get_clips_first_page` | 23.2 µs | 2000 行内存库，无查询，limit 50 |
| `get_clips_fts_prefix` | 298 µs | ≥3 字符走 FTS5 前缀查询，命中大量行 |
| `get_clips_like_fallback_short_query` | 38.1 µs | <3 字符走 LIKE 回退 |
| `get_clips_no_match` | 259 µs | 空结果 |

两点值得记住：

- 截图动作的墙钟时间几乎全在 `encode_png`（~77 ms/1080p）。想让截图更快，
  该动的是编码参数或编码时机，不是裁剪循环——裁剪是 memcpy，量级差两个数量级。
- 搜索在 2000 行量级是几十到几百微秒，远低于输入节流，当前不是瓶颈。
  注意 FTS5 前缀查询（298 µs）比 LIKE 回退（38 µs）慢，这不是 FTS5 的问题：
  基准数据里每行都含 `clipboard`，FTS 分支要为几乎全部行算相关性，而两字符查询走的
  LIKE 分支被 limit 提前截断。真实数据的选择性不同，两条路径的相对快慢也会不同。
