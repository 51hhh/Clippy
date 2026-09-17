# Clippy 架构与平台约束整改计划

制定日期：2026-09-17

审阅基线：`dev` / `80889e32381dcaf28949c7b7b6cbdab4b61d6483`

来源：[`完整结构与平台边界审阅`](../../reviews/2026-09-17-architecture-platform-review.md)

## Goal

在不改变现有产品交互和截图像素语义的前提下，把平台判断、窗口权限、IPC 合同、前端静态检查和
架构文档变成可自动验证的边界，并降低长截图等大型模块的继续维护风险。

## Requirements

- 所有正式平台必须在同一提交上通过原生 CI；
- Linux 平台/会话决策只有一个生产事实源；
- main 以外的每类窗口只能调用明确授权的业务命令；
- Rust commands、前端 wrappers 与查看器 allowlist 不能静默漂移；
- vanilla JS 获得增量静态检查，现有 DOMPurify 富文本能力保持可用；
- 移除与生产状态不符的长截图注释和宽范围 lint 豁免；
- 大文件拆分保持行为、IPC wire format、错误码和像素结果不变；
- 文档能从功能入口追踪到状态所有者、平台后端、输出和验证边界。

## Acceptance Criteria

- [ ] Ubuntu、Windows、macOS 三个 CI job 在同一 40 位 SHA 上 `completed/success`；
- [ ] 除明确登记的启动/诊断例外外，业务模块不直接读取 Linux 会话环境变量；
- [ ] settings、pin、capture overlay、longshot controller、image viewer 均有命令 allowlist 回归；
- [ ] CI 会在 command 未注册、wrapper 指向不存在命令或 viewer allowlist 漏项时失败；
- [ ] `src/js` 的目标目录通过新增静态检查，且没有扩大忽略清单；
- [ ] 用户富文本 HTML sink 只存在于固定 allowlist，全部经过 DOMPurify；
- [ ] `capture` 不再用“尚未接入”解释生产模块，也没有模块级 `allow(dead_code)` 掩盖整域；
- [ ] 第一轮拆分后，长截图窗口宿主的生产职责至少分成 registry/state、window lifecycle、worker/output；
- [ ] `./scripts/ci-local.sh` 完整通过，并记录跳过项；
- [ ] `docs/architecture.md`、`CLAUDE.md`、`AGENTS.md` 与最终实现一致。

## Out of Scope

- 不重做截图、Pin、图片查看器和主窗口的 UI；
- 不改变截图后端 fallback 顺序或跨平台能力承诺；
- 不一次性把 vanilla JS 全部迁移到 TypeScript；
- 不在结构拆分中修改长截图拼接算法、OCR 模型或图片像素输出；
- 不以 Linux 交叉编译替代 Windows/macOS 原生 runner；
- 不在未获明确授权时向 npm 或其他服务外发项目依赖树；
- 不在本计划中改写历史或发布新版本。

## 实施原则

1. 每个 Phase 使用独立分支和独立提交；高风险 Phase 不并行修改同一状态机。
2. 先补约束和 characterization test，再移动实现。
3. 每个 Phase 结束执行与改动匹配的定向测试；合入前执行完整 `./scripts/ci-local.sh`。
4. 平台代码只能由对应 native runner 判定完成；本机交叉检查只作早期提示。
5. wire contract、错误 code、事件名、窗口 label、custom protocol 路径和 PNG 字节语义默认保持不变。

## 执行记录（2026-09-17）

| Phase | 本地状态 | 分支 / 提交 | 完整本地门禁 |
|---|---|---|---|
| 0 | 三平台 CI 与安装包基线已具备；真机矩阵未执行 | `dev` / `80889e3` | 远程 CI `35180661672` 三平台 success |
| 1 | 已完成，待推送与原生 CI | `codex/platform-single-source` / `75af809` | 14 通过、0 失败、2 跳过 |
| 2 | 已完成，待推送与原生 CI | `codex/longshot-production-boundary` / `b75a08b` | 14 通过、0 失败、2 跳过 |
| 3 | 已完成，待推送与原生 CI | `codex/window-ipc-access` / `5d3a6f8` | 14 通过、0 失败、2 跳过 |
| 4 | 已完成，堆叠在 Phase 3；待推送与原生 CI | `codex/ipc-contract-gate` / `1314490` | 15 通过、0 失败、2 跳过 |
| 5 | 已完成，待推送与原生 CI | `codex/frontend-static-boundary` / `3f8161a`、`a18e3b1` | 16 通过、0 失败、2 跳过 |

Phase 4 选择轻量 parity gate 与三个共享 JSON fixture，没有引入代码生成依赖；完整绑定生成可在
Phase 7 拆分 API facade 时重新评估。Phase 5 的静态类型范围先固定为 `js/preview/` 与
`js/settings/`，HTML/Tauri 边界则覆盖全部 `src/js` 和 `src/react`。

以上“已完成”仅表示本地实现与 Linux 门禁完成。Windows/macOS 原生 CI、分支合入和真机 QA 尚未执行。
Phase 6 依赖 Phase 2、Phase 4 先合入并在同一 SHA 上通过远程门禁，因此当前不提前拆分
`window_host.rs`。

---

## Phase 0：关闭当前三平台发布阻塞

**目标：** 先证明 `80889e3` 的 Windows/macOS 修复真实成立，避免在红基线上继续结构改造。

**涉及：**

- `.github/workflows/build.yml`
- `scripts/verify-native-ci.mjs`
- `docs/native-qa.md`
- GitHub 分支保护设置

- [x] 记录 CI `35179825379` 结论：Ubuntu、Windows 成功；macOS OCR 取消测试失败；
- [x] 修复后的 CI `35180661672` 已完成，逐个 job 结论：`Check (ubuntu-22.04)` success、
  `Native Check (windows-latest)` success、`Native Check (macos-latest)` success；
- [x] 该失败在独立分支 `fix/macos-ocr-interpreter-test` 修复后以 `--no-ff` 合入 `dev`
  （`80889e3`），未改动被测产品代码；
- [x] 对最终 SHA 运行
  `node scripts/verify-native-ci.mjs --repo 51hhh/Clippy --sha 80889e32381dcaf28949c7b7b6cbdab4b61d6483`，
  输出 `Result: PASS`；证据文件按仓库惯例只留在本地工作区；
- [x] `dev` / `main` 的 required checks 已由 ruleset `23578066` 覆盖三个原生 job，同时禁止强推与
  删除，且不设 bypass actor；
- [x] 最终 SHA `80889e3` 与 run URL 已写入审阅记录；
- [ ] 真机矩阵未执行 —— 需要真实 Windows / macOS / KDE 环境，按 `docs/native-qa.md` §3 起逐项完成。
  输入已就绪：`Native QA Packages` run `35181379112` 在同一 SHA 上构建成功（Linux x64、Windows x64、
  macOS Intel、macOS Apple-Silicon 四套包，外加 Ubuntu 24.04 X11 Runtime Smoke），产物与
  `qa-record-templates-80889e32381dcaf28949c7b7b6cbdab4b61d6483` 均以完整 SHA 命名。
  这批包版本仍是 `0.1.20`；Windows 为临时自签名、macOS 仅 Ad-Hoc 签名，不能作为 Developer ID、
  公证或 Gatekeeper 信任证据，updater 验证必须改用同 SHA 的正式 release 产物。

**验收：** 同一 SHA 的三个 job 均为 success，验证脚本输出成功；不以“已启动”或“Linux 通过”代替完成。

**当前状态：** 门禁部分已在 `80889e3` 上达成验收；真机矩阵仍为未执行，因此 Phase 0 整体未关闭。
三平台 CI 只覆盖条件编译、原生 API 与单元测试，不覆盖桌面权限、输入注入、混合 DPI 和签名信任链。

**建议提交：** 若无需改代码则不提交；如需 workflow 调整，单独使用
`ci: 将三平台原生检查设为发布门禁`。

---

## Phase 1：统一平台与桌面会话事实源

**目标：** 截图、自动粘贴、快捷键与能力展示对同一环境给出同一结论。

**修改文件：**

- `src-tauri/src/platform/mod.rs`
- `src-tauri/src/paste/mod.rs`
- `src-tauri/src/screenshot/backends.rs`
- 相关 Rust tests

- [x] 为 `platform::detect_session_from` 补全显式 X11、显式 Wayland、XWayland、仅 display、无 display 表；
- [x] 让 `PasteManager::new` 根据 `platform::current_session()` 映射 backend；
- [x] 删除 `paste::detect_backend` 对环境变量的重复读取，保留纯映射函数供测试；
- [x] 让截图后端使用 `platform::is_wayland()`，删除本地 `is_wayland_session()`；
- [x] 明确允许直接读取原始环境变量的两个例外：Tauri 初始化前的 GDK backend 选择、诊断采集；
- [x] 增加静态回归，限制 `XDG_SESSION_TYPE`、`WAYLAND_DISPLAY`、`DISPLAY` 的生产读取位置。

**定向验证：**

```bash
cd src-tauri
cargo test platform::
cargo test paste::
cargo test screenshot::
cargo clippy --all-targets -- -D warnings
```

**验收：** 业务决策只调用 platform API；现有 Linux backend 选择测试保持通过。

**建议提交：** `refactor(platform): 统一桌面会话与后端选择事实源`

---

## Phase 2：收紧长截图生产边界

**目标：** 让注释、可见性和 lint 与已经上线的 IPC/UI 一致。

**修改文件：**

- `src-tauri/src/capture/mod.rs`
- `src-tauri/src/capture/longshot.rs`
- `src-tauri/src/capture/longshot/controller.rs`
- `src-tauri/src/capture/longshot/lifecycle.rs`
- `src-tauri/src/capture/longshot/recapture.rs`
- `src-tauri/src/capture/manager.rs`
- `src-tauri/src/pin/commands.rs`
- `src-tauri/src/commands.rs`

- [x] 更新“IPC 尚未接入”“未来长截图”等过期注释；
- [x] 删除 `longshot`、`mode_gate` 的模块级 `allow(dead_code)`；
- [x] 删除仅用于压制生产 unused 的 re-export，或把真实调用改为最小可见性；
- [x] 对编译器新暴露的 dead code 逐项分类：删除、接线、测试专用 `#[cfg(test)]`、窄范围保留；
- [x] 检查 Pin 输出 certainty 注释是否与当前长截图输出调用一致；
- [x] 增加源码约束测试，禁止长截图生产入口重新出现“尚未接入”说明或模块级 dead-code 豁免。

**定向验证：**

```bash
cd src-tauri
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
cargo test capture::longshot::
cargo test pin::
```

**验收：** 生产构建无长截图模块级 dead-code 豁免，所有保留豁免均附具体原因和最小作用域。

**建议提交：** `refactor(capture): 对齐长截图生产边界与 lint 合同`

---

## Phase 3：建立所有窗口的业务 IPC 权限矩阵

**目标：** 子窗口只能调用其职责范围内的自定义 command。

**新增/修改文件：**

- 将 `src-tauri/src/viewer/access.rs` 提升为通用窗口访问模块，或新增 `src-tauri/src/ipc_access.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/capabilities/default.json`
- `src/tests/window-capabilities.test.js`
- Rust allowlist tests

- [x] 盘点所有 command，按 `main`、`settings`、`pin-*`、`capture-overlay-*`、
  `longshot-controller-*`、`image-viewer-*` 建矩阵；
- [x] 默认拒绝未知受限窗口标签，main 保留经过审阅的完整能力；
- [x] 把 viewer 现有 allowlist 合并进统一模块，保持错误码 `forbidden`；
- [x] 对每类窗口测试一项允许命令和多项跨域拒绝；
- [x] 测试动态标签、旧窗口标签、未知 label 和 command 拼写错误；
- [x] 检查 core/plugin capability 是否仍超出每类窗口实际需要，能拆则拆成多个 capability 文件。

**定向验证：**

```bash
cd src-tauri && cargo test ipc_access
cd src && npx vitest run tests/window-capabilities.test.js tests/viewer-api.test.js
```

**验收：** 新增全局业务命令时，未分配窗口权限会让测试失败；现有窗口流程不受影响。

**建议提交：** `refactor(ipc): 按窗口类型限制自定义命令`

---

## Phase 4：把 IPC 合同漂移纳入 CI

**目标：** 自动发现 command 注册、前端 wrapper、viewer allowlist 和类型合同的不一致。

**新增/修改文件：**

- `scripts/check-ipc-contract.mjs`
- `src/js/api.ts`
- `src/js/ipc-types.ts`
- `src/tests/ipc-api.test.js`
- `.github/workflows/build.yml`
- `scripts/ci-local.sh`

- [x] 提取或解析 Rust command 名称与 `generate_handler!` 注册列表；
- [x] 收集 `api.ts` 中 literal invoke 和显式动态 command 清单；
- [x] 校验：command 有注册、wrapper 有目标、viewer command 同时存在于访问矩阵；
- [x] 为无法静态解析的动态调用建立显式常量表，禁止任意字符串散落；
- [x] 评估 `tauri-specta` 或等价生成绑定的迁移成本；本 Phase 默认先做轻量 parity gate；
- [x] 选择 viewer、capture、pin 各一个 DTO 做 serde JSON fixture 双向合同试点；
- [x] 将脚本加入本地门禁和 Ubuntu CI。

**验收：** 故意删除一个 handler、改错一个 wrapper 名称、漏掉一个 viewer allowlist 项时，脚本均失败。

**建议提交：** `test(ipc): 校验命令注册与前端合同一致性`

---

## Phase 5：增量覆盖 vanilla JS 静态检查与 HTML sink

**目标：** 不迁移框架的情况下，为主前端补足编译期约束。

**新增/修改文件：**

- `src/eslint.config.mjs` 或 `src/tsconfig.js.json`
- `src/package.json`
- `src/js/**`
- `scripts/check-html-sinks.mjs`
- `AGENTS.md`
- `CLAUDE.md`
- `scripts/ci-local.sh`
- `.github/workflows/build.yml`

- [x] 先只覆盖 `src/js/preview/`、`src/js/settings/` 和新增文件，避免一次性处理全仓遗留；
- [x] 启用 no-undef、no-unused-vars、Promise 处理、import 边界和必要的浏览器 globals；
- [x] 建立规则：生产代码只有 `api.ts` 可导入 `@tauri-apps/*`；
- [x] 把清空容器的 `innerHTML = ""` 改为 `replaceChildren()`；
- [x] 建立 HTML sink allowlist，只允许 DOMPurify 清洗后的 Markdown、富文本、highlight 结果；
- [x] 将 `AGENTS.md` 的绝对禁令改成与 `CLAUDE.md` 一致的 sanitizer 规则；
- [x] 每个后续 Phase 扩大一批 JS 目录，忽略必须带 owner/原因/移除条件。

**验收：** 新增直接 Tauri import、裸 `innerHTML = userValue`、未声明变量和未处理 Promise 会让门禁失败。

**建议提交：**

1. `chore(frontend): 增加 vanilla JS 静态检查`
2. `test(security): 固定富文本 HTML sink 白名单`

---

## Phase 6：拆分长截图窗口宿主

**前置：** Phase 2 与 Phase 4 完成，现有状态和 IPC 合同已被固定。

**目标：** 降低 `window_host.rs` 的状态组合复杂度，不改变任何状态转移。

**建议结构：**

```text
capture/longshot/window_host/
├── mod.rs          # 对外 open/activate/ready/append/preview/finish/cancel
├── model.rs        # wire DTO、registry state、token/handle
├── registry.rs     # claim/commit/rollback 与 generation
├── lifecycle.rs    # window create/show/hide/destroy/deadline
├── workers.rs      # activation/append/preview spawn_blocking
├── finish.rs       # 已有输出完成逻辑
├── output.rs       # 已有复制/保存/Pin 输出
└── tests/          # 按状态域拆分 characterization tests
```

- [ ] 先按现有测试名称绘制状态转移表，不修改行为；
- [ ] 移动纯 DTO 和 registry 操作，保持可见性 `pub(super)`；
- [ ] 移动窗口副作用和 deadline；
- [ ] 移动 append/preview worker；
- [ ] 将 3600 行测试按 registry、activation、append、preview、finish/cancel 分类；
- [ ] 每次移动后执行全部 `capture::longshot::window_host` 测试；
- [ ] 确认 custom protocol、event、label、error code 和 JSON wire format 无变化。

**验收：** `mod.rs` 只保留 facade 与组合；任何子文件不同时承担 registry 状态转移和真实窗口副作用。

**建议提交：** `refactor(capture): 拆分长截图窗口宿主职责`

---

## Phase 7：按领域拆分前端 API 与后端热点

**目标：** 保持唯一 Tauri 边界，同时降低单文件修改冲突。

### 7A：前端 API facade

- [ ] 保留 `src/js/api.ts` 作为公共 re-export；
- [ ] 新增 `src/js/api/clipboard.ts`、`capture.ts`、`pin.ts`、`viewer.ts`、`settings.ts`；
- [ ] 只有这些受控模块可导入 Tauri，其他前端继续从 facade 引用；
- [ ] IPC parity gate 使用聚合后的显式 command 清单；
- [ ] 不改变任何现有 export 名称。

### 7B：Pin commands

- [ ] 将输入校验、窗口命令、项目保存、渲染输出拆到现有领域模块；
- [ ] 保留 commands 文件为 Tauri adapter；
- [ ] 固定可编辑 PNG、扁平 PNG 与复制结果的字节语义。

### 7C：OCR

- [ ] 将可执行文件探测/缓存、进程执行、并发/single-flight、Tesseract 解析分别落到子模块；
- [ ] 保持取消、超时、子进程回收和 fallback 原因不变；
- [ ] 保持 viewer structured OCR wire contract 不变。

**验收：** 每个 adapter 只做参数转换和错误映射；领域实现可不依赖 Tauri command 直接测试。

**建议提交：** 每个子阶段独立 `refactor(api)`、`refactor(pin)`、`refactor(ocr)`，不得混合。

---

## Phase 8：同步架构与开发流程文档

**修改文件：**

- `docs/architecture.md`
- `CLAUDE.md`
- `AGENTS.md`
- `docs/feature-lifecycle.md`
- `docs/CI.md`
- 领域文档

- [ ] 在 `architecture.md` 顶部加入本审阅中的三条核心调用链；
- [ ] 完整列出 capture-overlay、longshot-controller、viewer、pin、shared 等功能岛；
- [ ] 用领域 bundle/所有者描述 `AppState`，避免复制会快速过期的字段清单；
- [ ] 把实现历史和性能数字下沉到 capture/pin 专题文档；
- [ ] 统一 DOMPurify/HTML sink 规则；
- [ ] 要求需求记录拥有稳定 issue、Wiki 或仓库文档 ID，并在 PR/CHANGELOG 引用；
- [ ] 明确本地门禁、交叉检查、原生 CI、真机 QA 四种证据不能互相替代。

**验收：** 文档中的路径全部存在；平台矩阵和质量门禁与当前 workflow/script 一致。

**建议提交：** `docs(architecture): 同步模块所有权与验证边界`

---

## Phase 9：依赖漏洞核查（需明确外发授权）

**目标：** 判断 `npm ci` 报告的 2 个 moderate vulnerability 是否可达生产包。

- [ ] 项目所有者明确授权向 npm 漏洞服务发送依赖树和版本元数据；
- [ ] 执行 `cd src && npm audit --json` 并保存 advisory ID、依赖路径和修复版本摘要；
- [ ] 区分 runtime dependency、build-only dependency、test-only dependency；
- [ ] 对运行时可达问题建立独立修复分支；
- [ ] 不直接执行 `npm audit fix --force`，先审阅 lockfile diff、Node/Tauri/Vite 兼容性和生产构建。

**验收：** 每个 advisory 有“可达/不可达/待确认”结论和升级验证证据。

---

## 合入顺序与提交边界

```text
Phase 0  三平台基线
  ↓
Phase 1  平台事实源
  ↓
Phase 2  长截图 lint/注释
  ↓
Phase 3  窗口 IPC 权限
  ↓
Phase 4  IPC parity gate
  ├── Phase 5  前端静态检查
  └── Phase 6  长截图拆分
          ↓
Phase 7  其他热点拆分
          ↓
Phase 8  最终文档同步

Phase 9  取得外发授权后可独立执行
```

Phase 1～5 可以各自形成小型 PR。Phase 6、7 属于结构重构，必须建立在 Phase 4 的合同门禁上，
并且一次只拆一个领域。

## 每阶段完成记录模板

```markdown
### Phase N 完成记录

- Commit / PR：
- 行为变化：无 / 说明
- 定向测试：
- `./scripts/ci-local.sh`：通过 / 失败 / 跳过项
- Ubuntu CI：
- Windows Native Check：
- macOS Native Check：
- 真机 QA：不适用 / 未执行 / 结果
- 未完成项：
```

未执行的平台或 smoke 必须保留为“未执行”，不能写入通过数量。
