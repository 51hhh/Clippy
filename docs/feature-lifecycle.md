# 功能开发生命周期指南

> Clippy 新功能从构思到发布的标准流程。参照 translator/flashot 的实践制定。

---

## 流程总览

```
Phase 1: Plan    → 创建 task、写 PRD、技术调研
Phase 2: Execute → 实现代码、质量检查
Phase 3: Finish  → 更新 spec、更新 CHANGELOG、archive task
```

每个 Phase 的步骤有 `[required]` 和 `[optional]` 之分，不可跳过 required 步骤。

---

## Phase 1: Plan

### 1.1 创建 task `[required · once]`

```bash
python3 ./.trellis/scripts/task.py create "功能名称" --slug feature-name
```

这会创建 `.trellis/tasks/MM-DD-feature-name/` 目录，包含 `task.json` 和 `prd.md` 模板。

### 1.2 编写 PRD `[required · once]`

`prd.md` 必须包含以下章节：

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

### 1.3 技术调研 `[optional · repeatable]`

如果功能涉及新技术或跨模块改动，在 `research/` 子目录写调研文档：

```bash
mkdir -p .trellis/tasks/MM-DD-feature-name/research/
```

调研结论应形成 ADR-lite（Architecture Decision Record）：

```markdown
## Decision
**Context**: 面临什么选择
**Decision**: 做了什么决定
**Consequences**: 带来什么影响
```

### 1.4 启动 task `[required · once]`

```bash
python3 ./.trellis/scripts/task.py start feature-name
```

这会将 task 状态从 `planning` 切换到 `in_progress`，并写入 `.trellis/.current-task`。

**规则：没有 start 的 task 不允许写代码。**

---

## Phase 2: Execute

### 2.1 创建功能分支 `[required · once]`

```bash
git checkout -b feat/feature-name dev
```

分支命名规范：
- 新功能：`feat/<task-slug>`
- Bug 修复：`fix/<task-slug>`
- 重构：`refactor/<task-slug>`

### 2.2 实现代码 `[required · repeatable]`

遵循项目编码规范：
- 后端：`.trellis/spec/backend/`
- 前端：`.trellis/spec/frontend/`
- 关键约定见 `AGENTS.md`

每个逻辑完整的改动作为一个 commit，遵循 Git Commit 规范（见 AGENTS.md）。

### 2.3 质量检查 `[required · repeatable]`

```bash
./scripts/ci-local.sh          # 完整检查
./scripts/ci-local.sh --quick  # 快速检查（跳过构建）
```

检查项：
- `cargo fmt --check` — Rust 格式
- `cargo clippy -- -D warnings` — Rust lint
- `cargo test` — Rust 测试
- `npx vitest run` — 前端测试
- `npx vite build` — 前端构建

**所有检查必须通过才能进入 Phase 3。**

### 2.4 合入 dev `[required · once]`

```bash
git checkout dev
git merge feat/feature-name --no-ff
git branch -d feat/feature-name
```

使用 `--no-ff` 保留合并记录。

---

## Phase 3: Finish

### 3.1 更新 spec `[required · once]`

检查是否产生了新的编码约定或架构模式：

```bash
# 如果有新约定，更新对应的 spec 文件
# 如果没有新约定，跳过（但必须走一遍判断）
```

### 3.2 更新 CHANGELOG `[required · once]`

在 `CHANGELOG.md` 顶部添加新版本条目：

```markdown
## vX.Y.Z

### ✨ 新功能
- **功能名**：简要描述

### 🐛 修复
- （如有）

### 🧪 测试
- 测试数量和覆盖情况
```

### 3.3 关闭 task `[required · once]`

```bash
python3 ./.trellis/scripts/task.py finish
```

然后更新 `task.json`：

```json
{
  "status": "completed",
  "completedAt": "YYYY-MM-DD",
  "commit": "<merge commit hash>",
  "branch": "dev"
}
```

**规则：task 的 `completedAt` 和 `commit` 字段必须填写。**

### 3.4 Archive `[optional]`

release 后 archive 已完成的 task：

```bash
python3 ./.trellis/scripts/task.py archive feature-name
```

---

## 快速参考

| 阶段 | 命令 | 产出 |
|------|------|------|
| 创建 task | `task.py create "名称"` | `.trellis/tasks/` 目录 |
| 写 PRD | 编辑 `prd.md` | 需求 + 验收标准 |
| 启动 | `task.py start <name>` | status → in_progress |
| 开发 | 写代码 + commit | 功能代码 |
| 检查 | `./scripts/ci-local.sh` | 质量门禁 |
| 关闭 | `task.py finish` + 更新 task.json | status → completed |
| 归档 | `task.py archive <name>` | 移入 archive/ |

---

## 常见错误

| 错误 | 正确做法 |
|------|----------|
| 无 task 直接写代码 | 先 `task.py create` + `task.py start` |
| 代码已发布但 task 仍在 planning | 立即更新 task.json 状态为 completed |
| PRD 验收项未勾选但代码已合入 | 勾选所有已实现的验收项 |
| feat commit 里夹带版本号 | 版本号只放 `release:` commit |
| 把 bug 修复写成 feat | 用 `fix` type |
| 一个 commit 混合多种改动 | 拆成独立 commit |
