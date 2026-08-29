# 参考项目集成分阶段方案

制定日期：2026-08-29
起点：`dev` 分支 `ebba448 feat:暂存`
参考项目：`example/flashot`（`23f16b5` / v0.7.1 / MIT）、`example/translator`（`a8ac6cc` / v0.3.2 / GPL-3.0-only）

两个参考项目版本与 `.trellis/tasks/08-08-clippy-integrated-refactor/research/example-integration-analysis.md`
记录一致，上游无新增可借鉴变更，本方案沿用该分析的许可与边界结论。

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

- [ ] `sudo apt install libgtk-3-dev`（`cargo check` 现在挂在 `gdk-sys` 找不到 `gdk-3.0.pc`），
      并把该包补进 CLAUDE.md 的依赖清单
- [ ] `cd src && npm ci`（`src/node_modules` 缺失，`npm test` 报 `vitest: not found`）
- [ ] 跑通 `cargo fmt/check/clippy --all-targets`、`cargo test`、`vitest`、`tsc --noEmit`、
      `./scripts/ci-local.sh`，确认暂存点为绿
- [ ] 把 `feat:暂存` 拆成三个语义化 commit（React 翻译面板 / Pin React / Portal token 状态机）

验收：门禁全绿，且 `git log` 无 `feat:暂存` 这类无信息 message。

## Phase 1：关闭综合重构任务（当前瓶颈）

代码侧剩余低风险项 + 只能由真实桌面完成的验收。

### 代码侧

- [ ] **P4 错误类型化**：以现有 `translation/types.rs::TranslationError` 为模板（`thiserror` +
      稳定 `code()` + 不泄漏底层上下文的 `ipc_message()`），扩展到 storage / capture / pin / paste；
      command 层继续对外返回 `String`，内部保持结构化
- [ ] **P7 `require_cmd`**：`scripts/ci-local.sh` 增加前置命令存在性检查（参考 flashot
      `scripts/ci-local.sh`），缺 `xvfb-run`/`npm` 等时明确报错而非中途失败
- [ ] **P0 Wayland Pin 置顶**：只产出调研结论文档（layer-shell 取舍、是否值得引入），
      不改代码；X11/通用路径的 `always_on_top` 保持现状
- [ ] **P3 关闭为"不做"**：图像调整保留在前端 canvas filter。理由：导出走
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
- [ ] **2g** dictvoice TTS（敏感条目沿用阻断策略）

## Phase 3：截图与导出增强（flashot，MIT 可复用）

- [ ] **保存增强**：现在 `commands/capture_editor.rs::save_screenshot_image` 硬编码
      `Pictures/Clippy`；参考 `saver.rs` 加可配置保存目录 + 文件名模板 + 另存为对话框
      （需引 `rfd` 或 `tauri-plugin-dialog`）
- [ ] **圆角导出**：复用 `mask.rs::apply_rounded_corners`（约 234 行纯函数，含 2x2 超采样抗锯齿），
      保留 MIT 版权声明
- [ ] **后端 i18n / 托盘菜单本地化**：参考 `i18n.rs`

## Phase 4：可选

- [ ] **滚动截图**：`scroll_session.rs` + `scroll_stitch.rs` 约 1320 行。**建议不做，或仅限 X11**：
      Wayland Portal 下拿不到高频连续帧，可用性与成本不匹配
- [ ] **criterion benches**：参考 flashot `benches/` 五个基准，给截图/剪贴板/裁剪建性能回归基线

## 执行顺序

Phase 0 → Phase 1（代码侧由 AI 完成，真实桌面矩阵由项目所有者完成）→ Phase 2 → Phase 3 → Phase 4。
