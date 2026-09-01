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
| `png_dimensions_1080p` | 505 ns | 只解 PNG 头（2026-08-31 前是完整解码的 19.6 ms） |
| `validate_png_1080p` | 20.0 ms | 完整解码，只在信任边界上用 |
| `decode_png_base64_1080p` | 1.27 ms | 前端回传的 data URL 解码 |
| `compute_hash_1mib` | 1.56 ms | 1 MiB 文本的 SHA-256 去重哈希 |
| `is_sensitive_text_worst_case_100kib` | 31.2 µs | 不命中前缀，走整段转小写 |
| `is_sensitive_text_prefix_hit` | 4.2 ns | 命中 `sk-` 前缀提前返回 |
| `strip_html_tags_1mib` | 1.41 ms | 逐字符扫描 1 MiB HTML |
| `rgba_fingerprint_1080p` | 1.47 ms | 轮询之间"还是上一张图吗"，替掉一次 PNG 编码 |
| `get_clips_first_page` | 23.2 µs | 2000 行内存库，无查询，limit 50 |
| `get_clips_fts_prefix` | 298 µs | ≥3 字符走 FTS5 前缀查询，命中大量行 |
| `get_clips_like_fallback_short_query` | 38.1 µs | <3 字符走 LIKE 回退 |
| `get_clips_no_match` | 259 µs | 空结果 |

## 2026-09-01 复测

同机同参数复跑一遍，确认全流程 review 那几处改动没有碰到共享路径。除
`strip_html_tags_1mib`（1.41 → 1.58 ms，+12%）与 `get_clips_no_match`
（259 → 282 µs，+8%）之外都在 ±5% 以内；这两条对应的代码这次一行没动，
按机器状态漂移记录，不作为回归处理。同时新增一条：

| 基准 | 中位数 | 说明 |
|---|---|---|
| `validate_png_1440p` | 36.3 ms | 2560x1440 整张解码，即截图提交的常见最坏尺寸 |

这一条就是"截图提交少解一遍 PNG"省下的钱：覆盖层点对钩之后，信任边界必须整张解码
一次（1080p 19.5 ms / 1440p 36.3 ms），复制那条路以前还要再解一遍同一张图才能拿到
RGBA。现在校验解出来的像素直接交给剪贴板（`capture::CommitImage`），
省下的正是表里这个数——而它整段都落在"点了对钩还没生效"的等待里。

两点值得记住：

- 截图动作的墙钟时间几乎全在 `encode_png`（~77 ms/1080p）。想让截图更快，
  该动的是编码参数或编码时机，不是裁剪循环——裁剪是 memcpy，量级差两个数量级。
- **把舞台裁剪缩回原生分辨率是笔亏本账，量过了才不做。** 混合缩放的多屏上，
  低缩放那块屏在舞台图里是被放大过的（本机 eDP 逻辑 1920x1200、自己缩放 1.3333，
  在 ×1.5 的舞台图里切出 2880x1800，而原生只有 2560x1600）。把它缩回去看似能省
  4.3 MB。实测同机 release 下 `image::imageops::resize` 2880x1800 → 2560x1600：
  Triangle **204 ms**、Lanczos3 331 ms、Nearest 343 ms（近邻反而慢，它没走
  可向量化的那条路）。而省下的是一次**进程内** `tauri::ipc::Response` 的字节数，
  代价是每块非最大缩放的屏在截图路径上多等 200 ms 以上——同一条路上连 140 ms 的
  `HIDE_SETTLE_MS` 都专门优化掉了。**画质也不是理由**：帧已经被插值放大过，缩回去拿不回
  原始细节；交给浏览器按 devicePixelRatio 缩是 GPU 上的一次采样，比 CPU 先缩一遍再缩一遍更干净。
  真正的修法是**一开始就别拍成放大的**——见下一条。
- **逐屏原生取像素：画质靠它，速度靠"不编 PNG"**（协议 v5 的 `CaptureArea`，
  见 docs/capture-linux.md §3.1、§3.3）。整屏舞台图的尺寸是"逻辑并集 × 桌面最大缩放"，
  本机双屏是 6720x2412 = 16.2 Mpx；按每块屏自己的矩形分别拍则是
  2560x1600 + 3840x2160 = 12.4 Mpx，少 24%，而且每块屏拿到的是原生像素、不含插值。
- **但"少 24% 像素"不是它变快的理由，测这件事也踩过两次坑。** v4（逐屏 + PNG）实测冻结帧
  1945 / 1904 / 1850 ms，而整屏那条路曾测出 1052 ms——**这两个数不构成对照**，
  是不同时刻、不同桌面内容测的。后来同一会话里量：逐屏两次之和 1830 ms、整屏一次 1900 ms，
  **v4 本身并不比整屏慢**。同理"两块屏同样 4.1 Mpx，一块 884 ms 一块 124 ms"也是混淆实验
  （窗口数量、缩放、显示器全不一样）。唯一站得住的对照是**同一批像素、两种处理**：
  拍一块屏，把拿到的 PNG 解开，再用同一个 gdk-pixbuf 把这些**一模一样的像素**重编一次。

  | 屏 | 原生像素 | 端到端 `ScreenshotArea` | 本地重编同一批像素 | PNG | ⇒ 合成器取像素 |
  |---|---|---|---|---|---|
  | 外接 4K | 3840x2160 = 8.29 Mpx | **1704 ms** | **1607 ms** | 6185 KiB | ≈ 100 ms |
  | eDP | 2560x1600 = 4.10 Mpx | **126 ms** | **81 ms** | 562 KiB | ≈ 45 ms |

  **PNG 编码占了 4K 那块屏 94% 的耗时**，合成器绘制 + 读回只有约 100 ms。
  按输出字节数也自洽（同屏放大区域）：100x100 → 20 ms/24 KiB、640x400 → 238 ms/607 KiB、
  1280x720 → 647 ms/1905 KiB、整块 → 1784 ms/6185 KiB。内容熵是放大器，但它解释的是
  "字节为什么多"，时间花在 **deflate**，而 `Shell.Screenshot` 不暴露压缩档位。
- **v5 想在扩展里不生成 PNG，但那条路在 GJS 上不可能成立。**
  `Cogl.Texture.get_data` 的 `data` 参数是**没有长度标注**的 `array<uint8>`，GJS 一律按
  入参处理：复制一份给 C、返回就释放。用同形状但会保留指针的
  `GdkPixbuf.Pixbuf.new_from_data` 做别名实验：4×4 时改 JS 侧看不到变化（是副本）、
  196 608 B 时读回来全是垃圾（副本已释放）、3 MB 时**直接段错误**。所以扩展里的
  `is_get_data_supported()` + 32 个 `0xCD` 哨兵只是把"静默黑图"变成能 catch 的异常，
  不是加速手段；实际跑的一直是它内部的 PNG 回退。
- **真正不生成 PNG 的路子在合成器外面：`org.gnome.Mutter.ScreenCast` + PipeWire**
  （`screenshot/screencast.rs`，见 docs/capture-linux.md §3.4）。同一个用户直接可调，
  不经 Portal、不弹对话框、不需要扩展也不需要注销。本机双屏实测：

  | 段 | 耗时 |
  |---|---|
  | CreateSession / RecordMonitor / Start | 1 / 1 / 61 ms |
  | node 就绪 + 第一帧 | 18 + 104 ms |
  | 一个会话同时录两块屏（3840x2160 + 2560x1600，都是原生） | **351 ms**（dev，含 48 MB 通道重排） |
  | `capture_monitor_frames` 全程 | **280 ms**（此前 1900 ms） |

  代价是顶栏闪一下录制点（约 200 ms），且**必须传 `is-recording: true`**——传 false 会落到
  有 5 秒最短显示时间的"停止共享"胶囊上。**要现场量就跑**
  `cargo test --lib screencast_timings -- --ignored --nocapture`（打印每块屏"逻辑 × 真实缩放 =
  期望像素"和实收尺寸），整条链路的分段仍看
  `cargo test --lib capture_stage_timings -- --ignored --nocapture`。
  扩展那条路要跳过注销就跑 `scripts/probe-shell-capture.sh`，它借 `org.gnome.Shell.Eval`
  在活着的会话里把同一段取像素代码量一遍（需要先在 Looking Glass 里开 `unsafe_mode`，
  细节写在脚本头部注释里）。
- **剪贴板里躺着一张图，是个持续开销，不是一次开销。** 轮询走到图片分支时唯一能
  和 `last_hash` 比较的东西是 **PNG 的哈希**，于是每 500 ms 都要把 RGBA 重新编码成
  PNG（1080p ~85 ms）才发现"还是刚才那张"——截图复制完之后这就一直烧着 CPU。
  改成先按 RGBA 算一个非密码学指纹（1.47 ms，只在轮询之间用、永不入库），
  同一张图第二轮就到不了编码器。文本/HTML 接管剪贴板时清空指纹，
  于是"复制别的再复制回这张图"仍然走完整入库路径。
- **"读个尺寸"和"证明整张图能解出来"是两件事，价钱差四个数量级。** 原来只有一个
  `png_dimensions`，实现是 `load_from_memory`，于是每个只想知道宽高的地方（贴图窗口算
  多大、`PinOrigins::lookup` 按尺寸预筛、`save_png` 落盘前确认这是张 PNG）都白解一整张图。
  拆成只读 IHDR 的 `png_dimensions`（505 ns）和整张解码的 `validate_png`（20 ms）之后，
  后者只留在信任边界上——前端提交的 base64 一次。`PinOrigins::lookup` 尤其受益：
  它的注释一直写着"读个头做预筛"，而实现在拆分前是解两遍整张图。
- **已知边界（量过，暂不改）：列表查询返回的是完整 `text_content`。** `get_clips`
  的 SELECT 没有截断，于是库里躺着一条几 MB 的文本时，每次刷新列表（新增条目、
  切收藏、每次搜索输入）都要把那几 MB 从 SQLite 读出、序列化成 JSON、再由 webview
  解析一遍。前端在**渲染**上已经封了口（列表行只画一行、预览封在 200 KiB，
  见 `src/js/preview/large-text.js`），封不住的是这段传输。
  之所以没顺手加 `substr(text_content, 1, N)`：前端不只拿它来画，还拿它做类型判定
  （`preview/detectors.js`：JSON 要 parse 成功、base64 要整段解码）和翻译取词。
  在 SQL 里截断会让一条大 JSON 悄悄降级成 TEXT，要做对就得同时给出"少了多少字节"
  的诚实信号 + 一条按需取全文的路径，而 `get_clip_detail` 目前是同步跑在 GTK 线程上。
  改动面比收益大，因此记在这里而不是偷偷截断——真要动，先把 `get_clip_detail`
  挪出 GTK 线程。
- 搜索在 2000 行量级是几十到几百微秒，远低于输入节流，当前不是瓶颈。
  注意 FTS5 前缀查询（298 µs）比 LIKE 回退（38 µs）慢，这不是 FTS5 的问题：
  基准数据里每行都含 `clipboard`，FTS 分支要为几乎全部行算相关性，而两字符查询走的
  LIKE 分支被 limit 提前截断。真实数据的选择性不同，两条路径的相对快慢也会不同。
