# 功能开发生命周期指南

> Clippy 新功能从构思到发布的标准流程。

---

## 流程总览

```
Phase 1: Plan    → 写需求说明、技术调研
Phase 2: Execute → 实现代码、质量检查
Phase 3: Finish  → 更新规范、更新 CHANGELOG
```

每个 Phase 的步骤有 `[required]` 和 `[optional]` 之分，不可跳过 required 步骤。

每项需求必须有稳定、可访问的标识：GitHub issue、项目 Wiki 页面或仓库内 `docs/` 路径三选一。
本地草稿可以辅助工作，但不能是唯一需求来源。后续 PR 和用户可见 CHANGELOG 使用同一标识，
使需求、实现、验收和发布记录可以双向追踪。

---

## Phase 1: Plan

### 1.1 编写需求说明 `[required · once]`

必须包含以下章节：

```markdown
# 功能名称

## Goal
一句话说明这个功能要解决什么问题。

## Requirements
- 具体需求 1
- 具体需求 2

## Acceptance Criteria
- [ ] 验收条件 1（可测试）
- [ ] 验收条件 2（可测试）

## Out of Scope
- 明确不做的事情 1
- 明确不做的事情 2
```

验收标准必须是**可测试的**：要么有自动化测试覆盖，要么有明确的手动验证步骤。

`Out of Scope` 不是可选项。写不出来通常说明需求还没想清楚。

需求页或文档路径就是本功能的 requirement ID。实现前记录它，PR 描述必须引用；用户可见改动进入
`CHANGELOG.md` 时也引用同一 ID 或已关联该 ID 的 PR。

### 1.2 技术调研 `[optional · repeatable]`

如果功能涉及新技术或跨模块改动，先写调研文档。结论应形成 ADR-lite
（Architecture Decision Record）：

```markdown
## Decision
**Context**: 面临什么选择
**Decision**: 做了什么决定
**Consequences**: 带来什么影响
```

调研的负结果同样要记录，并限定到明确场景——它防止同一条路被反复重试。

---

## Phase 2: Execute

### 2.1 创建功能分支 `[required · once]`

```bash
git checkout -b feat/feature-name dev
```

分支命名规范：

- 新功能：`feat/<slug>`
- Bug 修复：`fix/<slug>`
- 重构：`refactor/<slug>`

Codex 自动化分支使用 `codex/<slug>`，仍需满足“一分支一件事”。

**一个分支做一件事。** 分支名应能反映改动范围。

### 2.2 实现代码 `[required · repeatable]`

遵循项目编码规范：

- 后端与前端规范见项目 Wiki 的「后端编码规范」「前端编码规范」
- 关键约定见 `AGENTS.md`
- 架构与模块职责见 `docs/architecture.md`

每个逻辑完整的改动作为一个 commit，遵循 Git Commit 规范（见 `AGENTS.md`）。

### 2.3 质量检查 `[required · repeatable]`

```bash
./scripts/ci-local.sh
```

检查项见 `docs/CI.md` 与 Wiki 的「质量门禁」。核心是：

- `cargo fmt` / `cargo clippy --all-targets -- -D warnings` / `cargo test`
- TypeScript 类型检查、Vitest
- DOM/Xvfb smoke、Canvas 与布局像素 smoke
- Vite 生产构建

本地脚本的所有必需步骤必须通过才能进入 Phase 3。涉及平台条件分支时，还要在 PR 的同一 SHA 上
取得 Ubuntu、Windows、macOS 原生 CI 成功；本地结果不能替代远程原生 runner。

**跳过不等于通过。** 像素 smoke 缺少依赖时整步跳过，这既不算通过也不能计入
测试数量。安装包与平台矩阵同理。

### 2.4 合入 dev `[required · once]`

1. 推送功能分支并创建目标为 `dev` 的 PR；PR 描述引用 requirement ID，并列出本地通过/失败/跳过。
2. 在 PR **同一个 40 位 SHA** 上确认 `Check (ubuntu-22.04)`、
   `Native Check (windows-latest)`、`Native Check (macos-latest)` 全部 success。
3. 合入后再次核对最终 `dev` SHA 的三项门禁；旧 run 或分支 run 不能作为最终证据。
4. 只有明确需要真实桌面交互时，再触发 Native QA Packages 并按 `docs/native-qa.md` 记录人工结果。

仓库规则决定具体 merge 方式；不要绕过 required checks。结构重构可以没有用户可见 CHANGELOG，
但需求、PR、架构文档和验证记录仍需闭环。

---

## Phase 3: Finish

### 3.1 更新规范 `[required · once]`

检查是否产生了新的编码约定或架构模式。有新约定就更新对应文档；没有也必须
走一遍判断，不能默认跳过。

新增的跨模块约定要写进 `docs/architecture.md`，否则下一个改这块代码的人
会再踩一次。

### 3.2 更新 CHANGELOG `[required if user-visible · once]`

用户可见变更写入 `CHANGELOG.md` 顶部的 `## 未发布`，发布时再收口为版本章节：

```markdown
## 未发布

### ✨ 新功能
- **功能名**：简要描述（需求：#issue / Wiki / docs 路径）

### 🐛 修复
- （如有）

### 🧪 测试
- 测试数量和覆盖情况
```

**未验证的边界必须写清楚。** 例如「本地通过、未运行远程 CI」「未构建安装包」
「某平台未实测」。把未完成项写成完成是本项目明确禁止的。

纯文档、测试或不改变行为的结构重构可以不增加用户 CHANGELOG；PR 中要记录已检查且无需条目的结论。

### 3.3 记录验证证据 `[required · once]`

| 层级 | 入口 | 结论边界 |
|---|---|---|
| 本地完整门禁 | `./scripts/ci-local.sh` | 只证明当前宿主与实际执行的步骤 |
| 可选交叉检查 | `CLIPPY_CROSS_CHECK=1 ./scripts/ci-local.sh` | 只补充编译期提示，不执行目标平台测试 |
| 同 SHA 原生 CI | `build.yml` + `verify-native-ci.mjs` | 证明三平台原生 check/clippy/test |
| Native/人工 QA | `native-qa.yml` + `native-qa.md` | 证明安装包与真实桌面场景 |

四种证据不能互相替代。记录必须包含 SHA、run URL/编号和跳过项；真机 QA 未执行就明确保留未完成。

---

## 快速参考

| 阶段 | 动作 | 产出 |
|------|------|------|
| 需求 | 写 Goal / Requirements / AC / Out of Scope | 可测试的验收标准 |
| 调研 | ADR-lite | Context / Decision / Consequences |
| 开发 | 分支 + commit | 功能代码 |
| 检查 | `./scripts/ci-local.sh` | 质量门禁结果（通过/失败/跳过） |
| 合入 | PR + 同 SHA required checks | dev 上的合并记录与原生证据 |
| 记录 | 更新 `CHANGELOG.md` 与相关文档 | 用户可见变更 + 未验证边界 |

---

## 常见错误

| 错误 | 正确做法 |
|------|----------|
| 没写验收标准就开始写代码 | 先把 AC 写成可测试的条目 |
| 验收项未达成但代码已合入 | 未达成的项保留未勾选，并在 CHANGELOG 写明 |
| 把跳过的门禁步骤算作通过 | 区分通过/失败/跳过三态 |
| 把安装包、平台矩阵计入测试数 | 单独说明，不计入 |
| feat commit 里夹带版本号 | 版本号只放 `release:` commit |
| 把 bug 修复写成 feat | 用 `fix` type |
| 一个 commit 混合多种改动 | 拆成独立 commit |
| 一次手动启动就当作交互验收 | 启动只确认编译与服务可用 |
| 本地或交叉编译通过就宣称三平台通过 | 核对同一 SHA 的三个原生 CI job |
| CI 绿色就宣称桌面权限/混合 DPI 已验证 | 使用 Native QA 包在真实目标环境记录结果 |
| 需求只存在本地草稿 | 建立 issue、Wiki 或仓库文档 ID，并在 PR/CHANGELOG 引用 |
