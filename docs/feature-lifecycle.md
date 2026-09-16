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

需求说明与调研记录放在本地工作区，不随仓库分发；进入仓库的是代码、测试、
`CHANGELOG.md` 与 `docs/`。

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

**所有检查必须通过才能进入 Phase 3。**

**跳过不等于通过。** 像素 smoke 缺少依赖时整步跳过，这既不算通过也不能计入
测试数量。安装包与平台矩阵同理。

### 2.4 合入 dev `[required · once]`

```bash
git checkout dev
git merge feat/feature-name --no-ff
git branch -d feat/feature-name
```

使用 `--no-ff` 保留合并记录。

---

## Phase 3: Finish

### 3.1 更新规范 `[required · once]`

检查是否产生了新的编码约定或架构模式。有新约定就更新对应文档；没有也必须
走一遍判断，不能默认跳过。

新增的跨模块约定要写进 `docs/architecture.md`，否则下一个改这块代码的人
会再踩一次。

### 3.2 更新 CHANGELOG `[required · once]`

在 `CHANGELOG.md` 顶部添加条目：

```markdown
## vX.Y.Z

### ✨ 新功能
- **功能名**：简要描述

### 🐛 修复
- （如有）

### 🧪 测试
- 测试数量和覆盖情况
```

**未验证的边界必须写清楚。** 例如「本地通过、未运行远程 CI」「未构建安装包」
「某平台未实测」。把未完成项写成完成是本项目明确禁止的。

---

## 快速参考

| 阶段 | 动作 | 产出 |
|------|------|------|
| 需求 | 写 Goal / Requirements / AC / Out of Scope | 可测试的验收标准 |
| 调研 | ADR-lite | Context / Decision / Consequences |
| 开发 | 分支 + commit | 功能代码 |
| 检查 | `./scripts/ci-local.sh` | 质量门禁结果（通过/失败/跳过） |
| 合入 | `git merge --no-ff` | dev 上的合并记录 |
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
