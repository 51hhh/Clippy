# 参考项目集成分阶段方案

制定日期：2026-08-29
起点：`dev` 分支 `ebba448 feat:暂存`
参考项目：`example/flashot`（`23f16b5` / v0.7.1 / MIT）、`example/translator`（`a8ac6cc` / v0.3.2 / GPL-3.0-only）

两个参考项目版本与 `.trellis/tasks/08-08-clippy-integrated-refactor/research/example-integration-analysis.md`
记录一致，上游无新增可借鉴变更，本方案沿用该分析的许可与边界结论。

## 当前进度（2026-08-29 收尾）

Phase 0～4 的**代码侧全部完成**，门禁 `./scripts/ci-local.sh` 10 通过 / 0 失败 / 1 跳过。
剩余两项都不该由 AI 自动做：

1. **真实桌面矩阵**（Phase 1 下半段）——只能在真机 GNOME X11 / GNOME Wayland / KDE Wayland
   上人工完成，结果写回 `qa-matrix.md` 与 `completion-audit.md`，Trellis 任务才能标 `completed`。
2. **拆分 `feat:暂存`**（Phase 0 最后一条）——需要改写已推送的 `origin/dev` 历史并 force push。

本地 `dev` 领先 `origin/dev` 若干提交，**尚未推送**，推送时机由项目所有者决定。

## 暂存点状态

`ebba448` 是一次未整理的提交，工作树干净、无 stash，内容为三项收尾：

1. 主窗口翻译面板 JS→React（`src/react/main/TranslationPanel.tsx`、`translationStore.ts`、
   `mount.tsx`；删除 `src/js/translation-panel.js` 与旧测试）。
2. Pin React 化扩展（`src/react/pin/App.tsx` + `src/tests/pin-react-app.test.js`）。
3. Portal restore token 生命周期改为 `PortalAuthorizationStage` 阶段状态机（`src-tauri/src/paste/portal.rs`）。

Trellis 任务 `08-08-clippy-integrated-refactor` 仍为 `in_progress`，阻断项是 `qa-matrix.md`
中全部为空的真实桌面矩阵。

## Phase 0：恢复可验证基线

本机开发环境缺依赖，当前两条门禁命令都无法执行，任何改动都没有验证网，必须先修。

- [x] `sudo apt install libgtk-3-dev`，并把该包补进 CLAUDE.md 的依赖清单
- [x] `cd src && npm ci`
- [x] 跑通 `cargo fmt/check/clippy --all-targets`、`cargo test`、`vitest`、`tsc --noEmit`、
      `./scripts/ci-local.sh`
- [ ] 把 `feat:暂存` 拆成三个语义化 commit（React 翻译面板 / Pin React / Portal token 状态机）
      — **未做，留给项目所有者决定**：`ebba448` 已经推到 `origin/dev`，现在它下面还压着 11 个
      提交，拆分等于改写已发布历史 + force push。这属于对外可见且难以回退的动作，不自动执行

验收：门禁全绿（当前 `./scripts/ci-local.sh` 10 通过 / 0 失败 / 1 跳过，跳过项是需要显式开启的
AppImage 可视 smoke）。`git log` 里仍有一条 `feat:暂存`，见上条。

## Phase 1：关闭综合重构任务（当前瓶颈）

代码侧剩余低风险项 + 只能由真实桌面完成的验收。

### 代码侧

- [x] **P4 错误类型化**：以现有 `translation/types.rs::TranslationError` 为模板（`thiserror` +
      稳定 `code()` + 不泄漏底层上下文的 `ipc_message()`），扩展到 storage / capture / pin / paste；
      command 层继续对外返回 `String`，内部保持结构化
- [x] **P7 `require_cmd`**：`scripts/ci-local.sh` 增加前置命令存在性检查（参考 flashot
      `scripts/ci-local.sh`），缺 `xvfb-run`/`npm` 等时明确报错而非中途失败
- [x] **P0 Wayland Pin 置顶**：只产出调研结论文档（layer-shell 取舍、是否值得引入），
      不改代码；X11/通用路径的 `always_on_top` 保持现状
- [x] **P3 关闭为"不做"**：图像调整保留在前端 canvas filter。理由：导出走
      `src/react/capture/pngPipeline.ts` 已与预览一致，再在 Rust 写一份逐像素实现属重复实现，
      维护两份归一化逻辑反而增加不一致风险

### 真实桌面矩阵（需人工，无法由 Xvfb/单测替代）

- [ ] GNOME X11：原窗口恢复、Ctrl+V 注入、无 Portal 弹窗、主页面非黑屏/不越界
- [ ] GNOME Wayland：首次授权、同进程会话复用、重启后 restore token 恢复、撤权仅提示一次
- [ ] KDE Wayland：Portal、截图覆盖层、Pin 置顶
- [ ] 真实 Secret Service 保存/读取/删除，以及至少一个真实翻译服务回环

验收：结果写回 `qa-matrix.md` 与 `completion-audit.md`，Trellis 任务方可标 `completed`。

## Phase 2：翻译全量对齐 translator

**已确认决策**：服务集合照 translator 全量对齐（含无 key 的非官方 web 路径）；TTS 用在线
`dict.youdao.com/dictvoice`。这两项与 `docs/reference-project-guidance.md` 原有
"不把非官方接口设为默认 / 减少外发" 原则相冲突，取舍已由项目所有者确认，需同步更新该文档，
不留下两份互相矛盾的原则。

### 许可约束（硬约束）

translator 为 GPL-3.0-only，**不能复制源码**。四个服务在 translator 中分别是
google 976 行、bing 1256 行、deepl 634 行、youdao 1372 行，其中非官方路径涉及 token 抓取与
签名构造，必须按可观察行为独立实现。这是本阶段最大的成本项。

### 服务矩阵

| 服务 | 官方路径（需凭据） | 非官方 fallback（无 key） | 新增凭据/配置 |
|---|---|---|---|
| OpenAI 兼容 | `api.openai.com/v1` 等 | — | 已有 |
| DeepL | `api.deepl.com` / `api-free.deepl.com` | `www2.deepl.com/jsonrpc` | api_key |
| Google | `translation.googleapis.com` | `translate.googleapis.com`（gtx） | api_key + project id |
| Bing | `api.cognitive.microsofttranslator.com` | `cn.bing.com` | api_key + region |
| 有道 | `openapi.youdao.com`（sign v3 = SHA-256） | `fanyi.youdao.com` / `dict.youdao.com` | app_key **+** app_secret |

### 需要的模型改造

- **多服务并行**：`AppConfig` 的单 `translation_provider` 字符串改为启用服务列表 +
  每服务子配置（endpoint 覆盖、model、region、project）；结果改为多结果卡，各服务结果互不覆盖，
  复用已有 `request_id` 防陈旧，单服务可独立重试
- **凭据模型**：现在是每 provider 单 api_key；有道需要 app_key + app_secret 两段，
  Secret Service 条目需支持一个 provider 多凭据；沿用"仅 Secret Service、失败不落明文"
- **HTTP**：Clippy 用阻塞 `ureq`，并行需按服务 `spawn_blocking`，不引入 `reqwest`
- **语言方向**：参考 `language_direction.rs` 引入 `preferred_languages` 与源=目标自动换向
- **翻译历史**：写入现有 SQLite
- **TTS**：构造 dictvoice URL 播放。注意这会把被朗读文本外发给非官方接口，
  敏感条目必须沿用现有阻断策略，不允许朗读

### 风险与缓解

非官方路径会随对方网页改动失效。缓解：官方路径优先实现并作为默认，非官方仅作显式可选 fallback；
新增独立错误码（如 `provider_endpoint_broken`）让前端能区分"接口失效"与"配置错误"；
每个非官方路径必须有本地 mock 回环测试。

### 落地切片

- [x] **2a/2b** 共享 HTTP 层与官方/非官方路由，四个服务适配器（官方 + 非官方双路径）
      与本地 mock 回环测试
- [x] **2c** 凭据模型支持一个 provider 两段凭据，仅写 Secret Service
- [x] **2d-1** `AppConfig` 改为 `translation_services` 列表 + v1→v2 迁移，
      设置页可配置全部 6 个服务（仍是单选启用语义）
- [x] **2d-2** 多服务同时启用：按服务 `spawn_blocking` 并行、多结果卡、单服务重试
      （`ServiceTranslation`/`TranslationBatch` 标签联合，失败作为数据返回；
      截图选区浮层仍只用 `primary_service`）
- [x] **2e** `language_direction` / `preferred_languages` 与源=目标自动换向
      （`translation/direction.rs`：字符集粗判只用于决定是否换向，发给 provider 的源语言
      仍是 `auto`；实际目标语言随结果返回，结果卡按它展示而不是按设置里的目标语言。
      `preferred_languages` 用 `#[serde(default)]` 加入，空列表沿用「目标 + 源」，无需迁移）
- [x] **2f** 翻译历史写入现有 SQLite
      （`storage/translation_history.rs`：一条记录 = 条目 + 服务 + 目标语言 + 原文哈希，
      重复翻译 upsert 同一行，全库上限 500 条；条目删除/历史清空/上限清理都会带走它的译文。
      写入失败只记日志，不影响本次翻译结果。选中条目时按启用的服务回填卡片并标为
      "Saved earlier"，汇总行保持 idle；设置页提供"清空已保存的译文"）
- [x] **2g** dictvoice TTS（敏感条目沿用阻断策略）
      （`translation/tts.rs`：Rust 经共享 HTTP 层取回音频，前端以 data URL 播放，
      webview 不直接访问 dictvoice；文本上限 200 字符，非 MP3 响应按 invalid_response 拒绝。
      `speak_clip` 走翻译同一条内容选择路径，敏感条目被拒；一次只播一段）

## Phase 3：截图与导出增强（flashot，MIT 可复用）

- [x] **保存增强**：`image_io::SaveTarget` 统一解析保存目录与文件名模板
      （`screenshot_save_dir` / `screenshot_filename_template` 都用 `#[serde(default)]` 加入，
      空值表示内置默认，因此不需要迁移或提升配置版本）；截图覆盖层、编辑器和 Pin 三处保存
      共用同一份配置。模板支持 `{prefix}` `{date}` `{time}` `{unix}` `{seq}`，只生成文件名
      （分隔符与前导点被清洗），同名时用 `create_new` 追加序号而不是覆盖。
      另存为与「浏览」目录选择走 `tauri-plugin-dialog`（在 `dialogs.rs` 一处封装，
      插件已在主线程构造对话框，命令侧用 `spawn_blocking` 等结果），用户取消返回 `null`
- [x] **圆角导出**：功能已由前端导出管线实现，**不移植** `mask.rs::apply_rounded_corners`。
      `canvasRenderer.ts::renderExport` 末尾用 `destination-in` + `ctx.roundRect` 抠圆角，
      半径按 `min(radius, width/2, height/2)` 收敛，抗锯齿由 canvas 负责，与预览的
      `borderRadius`（`App.tsx`）取同一个 `cornerRadius`（`imageAdjustments.ts` 里钳到 0..120）。
      再写一份 Rust 逐像素超采样实现会让同一个视觉效果有两个必须保持一致的真值来源，
      理由与 P3 图像调整判定为"不做"完全相同。回归覆盖：`scripts/smoke-canvas-export.sh`
      在真实 canvas 上断言导出图角点 alpha < 128（`src/tests/fixtures/canvas-export-smoke.ts`）
- [x] **后端 i18n / 托盘菜单本地化**：新增 `src-tauri/src/i18n.rs`（`NativeText` 静态文案 +
      `resolve_locale`），托盘菜单、设置窗口标题、截图编辑器窗口标题都随 `AppConfig.language` 切换。
      语言解析规则与前端 `i18n.js::resolveLocale` 一致：显式 `en`/`zh-CN` 优先，`auto`（或空）
      读 `LC_ALL`/`LC_MESSAGES`/`LANG`，其余一律回退英文；环境变量通过 `resolve_locale_with`
      参数注入，测试不改进程环境。`config-changed` 里用 `MenuItem::set_text` 原地改文案而不是
      重建菜单（避免刷新过程中托盘短暂无菜单），文案刷新排在图标刷新之前，托盘句柄丢失时语言仍生效。
      设置窗口的两处重复开窗合并到 `window_controller::open_settings_window`

## Phase 4：可选

- [x] **滚动截图 — 决定不做**：`scroll_session.rs` + `scroll_stitch.rs` 约 1320 行，
      Wayland Portal 下拿不到高频连续帧。仅限 X11 就意味着同一个入口在两种会话里给出
      两种能力，而 Clippy 的截图路径一直按"X11/Wayland 行为一致"设计，成本与收益不匹配
- [x] **criterion benches**：`src-tauri/benches/` 三个文件（screenshot/clipboard/storage），
      经 `src-tauri/src/bench_support.rs` 调生产代码而不是在 bench 里复制实现。
      不做裁剪基准：裁剪是 memcpy，被同一路径上的 `encode_png`（~77 ms/1080p）掩盖两个数量级，
      而 flashot 的 `crop_bench.rs` 量的是 bench 文件里自己复制的一份 `crop_rgba`，参考价值为负。
      `[profile.bench]` 关掉 release 的 lto/单 codegen-unit 以保证编译时间可接受；
      门禁的 `--all-targets` 负责编译基准防腐烂，数字与坑见 `docs/bench-baseline.md`

## Phase 5：review 修复（2026-08-29 收尾追加）

全量 review 截图/编辑器/快捷键/翻译四条链路后发现八项，六项已修，两项按取舍保留：

- [x] **A 快捷键注册失败被静默吞掉**：两条注册路径都记账，`get_shortcut_failures` 让设置页
      读到启动期的存量失败（事件早于页面监听已丢），按动作显示可操作提示
- [x] **B 缺少快捷键占用检测**：`shortcut_conflict.rs` 在 GNOME 逐 schema 枚举（排除 Clippy
      自己认领的那几个自定义条目）；X11 用 `enumerable = false` 区分"查不出来"与"没有冲突"；
      Clippy 三个动作的自冲突由前端判断，因为它能读到未保存的录制值
- [x] **C 翻译历史回填过于积极**：预览面板隐藏时不查历史，列表连按上下键只查停下的那条（120 ms 防抖）
- [x] **D 翻译面板键盘死区**：键盘路由抽成 `keyboard-router.js`，Tab/Esc 交回全局路由，并可单测
- [x] **E 标注工具缺 8 个**：补齐荧光笔/椭圆/直线/高亮块/测量/橡皮/聚光灯/放大镜，
      共 16 个按选择、绘制、效果三组呈现，详见 [architecture.md](../../architecture.md#图片编辑器工具)
- [x] **F "缺少 key" 退化成不透明 http_status**：4xx 正文限读 4 KiB 用于归类，5xx 正文不参与
- [ ] **G 选区翻译只用 `primary_service`** — **保持现状**：截图选区浮层空间只够一张结果卡，
      多服务并行的价值在主预览面板；改成多卡会让浮层遮挡截图内容
- [ ] **H 真实桌面矩阵** — 归属项目所有者，见 Phase 1

门禁：`./scripts/ci-local.sh` = 10 通过 / 0 失败 / 1 跳过（跳过项仍是需显式开启的 AppImage 可视 smoke）。
Canvas 导出像素 smoke 在缺 ffmpeg 时改用 python3-pil，因此本机不再整步跳过。

## Phase 6：二轮 review 修复（2026-08-29）

对 Phase 5 的产物再走一遍 review，又发现六项，五项已修，一项只报告：

- [x] **I 覆盖用户已有的自定义快捷键**（最严重，且是历史遗留）：GNOME 注册写死
      `custom0/1/2` 并覆盖那三个路径的 name/command/binding。用户若已在这些编号上建过快捷键，
      Clippy 一启动就把它们静默销毁，卸载时还会把用户的条目从列表里删掉。改为按 command
      认领自己的条目、认不出来再取未占用编号（`plan_slots`，纯函数 + 单测覆盖"用户占了
      custom0/1/2"、"复用上次的条目"、"编号有空洞"三种情况），进程内缓存一次解析结果
- [x] **J X11 注册失败无法归因**：`register_multiple` 是全有或全无，任何一个键位被占用都会
      整批失败并笼统上报成 `global`。改为逐个 `register`，键位相同的动作只注册一次
      （`plan_x11_registration` 定型去重/空值/解析失败），失败按动作记账，另外两个动作照常工作
- [x] **K 恢复路径不记账**：设置窗口关闭/销毁兜底走的 `resume_shortcuts_for_app` 只返回
      Result，成功不清失败记录、失败也不上报。现在 Wayland 用 `resume_with_results`
      逐个写 binding（仍只重启一次 gsd）并按动作记账，X11 复用 J 的逐个注册
- [x] **L 死代码 `update_shortcut`**：命令 + `api.ts` wrapper 都没有调用方，且绕过失败记账、
      只处理 global 一个键位，留着只会被误用。连 `invoke_handler` 条目一起删除
- [x] **M 文档与 UI 分组不一致**：`architecture.md` 把 highlight 放在"绘制"，侧栏放在"效果"。
      按 UI 为准修正文档，并说明 highlight 只是半透明矢量块（绘制顺序仍跟随矢量标注），
      分组成员由 `capture-editor-tools.test.js` 锁定，以后改分组会直接失败
- [ ] **N `is_gnome_desktop` 的假阴性** — **保持现状**：只看 `XDG_CURRENT_DESKTOP` /
      `XDG_SESSION_DESKTOP`，两者都为空的真实 GNOME 会话会被判为"不受管"，提示用户手动配置。
      方向是安全的（宁可不写 dconf 也不在非 GNOME 上假装注册成功），补探测反而会回到
      "能写入就算注册成功"的老坑

门禁复跑：`cargo fmt` / `cargo clippy --all-targets -D warnings` 干净，`cargo test` 229 通过，
`vitest` 31 文件 / 511 通过，`tsc --noEmit` 干净，`./scripts/ci-local.sh` 10 通过 / 0 失败 / 1 跳过。

## 执行顺序

Phase 0 → Phase 1（代码侧由 AI 完成，真实桌面矩阵由项目所有者完成）→ Phase 2 → Phase 3 → Phase 4。
