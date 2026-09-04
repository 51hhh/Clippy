<!-- TRELLIS:START -->
# Trellis Instructions

These instructions are for AI assistants working in this project.

Use the `/trellis:start` command when starting a new session to:
- Initialize your developer identity
- Understand current project context
- Read relevant guidelines

Use `@/.trellis/` to learn:
- Development workflow (`workflow.md`)
- Project structure guidelines (`spec/`)
- Developer workspace (`workspace/`)

If you're using Codex, project-scoped helpers may also live in:
- `.agents/skills/` for reusable Trellis skills
- `.codex/agents/` for optional custom subagents

Keep this managed block so 'trellis update' can refresh the instructions.

<!-- TRELLIS:END -->

# Clippy — AI Agent 速查指引

## 项目简介
Clippy 是跨平台轻量剪贴板管理器。技术栈：Tauri v2 + Rust（后端）+ vanilla HTML/CSS/JS（主前端）+ React/TS（截图编辑功能岛）。
详细架构与数据流见 [CLAUDE.md](CLAUDE.md)；设计文档见 [docs/superpowers/specs/](docs/superpowers/specs/)。

> ⚠️ 本仓库包含两个独立项目：根目录的 **Clippy** 和 `fxxkDJTU/` 下的发票工具（Vue + TS）。二者无代码依赖。

## 常用命令
```bash
cargo tauri dev                            # 热重载开发（前端 + Rust）
cd src-tauri && cargo check                # 快速编译检查
cd src-tauri && cargo test                 # Rust 单元测试
cd src-tauri && cargo clippy -- -D warnings # Lint（警告即错误）
cd src-tauri && cargo fmt                  # 格式化
cd src && npx vitest run                   # 前端测试（jsdom）
cd src && npx tsc --noEmit                 # React/TS 功能岛类型检查
./scripts/ci-local.sh                      # 本地质量预检（与 CI 一致）
./scripts/ci-local.sh --quick              # 跳过构建，仅 lint + test
```

## 架构要点
| 层 | 路径 | 说明 |
|----|------|------|
| 前端 | `src/js/` | ES Module，`api.js` 是唯一 Tauri IPC 入口 |
| 后端 | `src-tauri/src/` | 扁平模块：commands / storage / clipboard_watcher / config / models / portal_shortcuts / tray_icon |
| 数据库 | SQLite + FTS5 | `clips` 表 + `clips_fts` 虚拟表，SHA-256 去重 |
| 快捷键 | X11: tauri-plugin-global-shortcut; Wayland: XDG Portal (ashpd) |

## 关键约定
- **前端 XSS 防护**：所有用户内容用 `textContent`，禁止 `innerHTML`
- **IPC 封装**：只有 `api.js` 直接访问 `window.__TAURI__`
- **语言**：代码注释 / commit 中文，前端 UI 英文
- **构建目标**：Linux x64（deb、AppImage）、Windows x64（NSIS、MSI）、macOS Intel/Apple Silicon（DMG、updater bundle）
- **编码规范**：见 `.trellis/spec/backend/` 和 `.trellis/spec/frontend/`

## Git Commit 规范

遵循 [Conventional Commits](https://www.conventionalcommits.org/) 格式：

```
<type>(<scope>): <description>

[optional body]
```

### Type（仅限以下 9 种）

| Type | 用途 |
|------|------|
| `feat` | 用户可见的新功能 |
| `fix` | 用户可见的 bug 修复 |
| `docs` | 文档变更 |
| `style` | 代码格式（不改变逻辑） |
| `refactor` | 重构（不改变功能） |
| `perf` | 性能优化 |
| `test` | 测试补充或修正 |
| `chore` | 构建/依赖/工具链维护 |
| `ci` | CI/CD 配置变更 |

### 规则（不可违反）

- **first line 不超过 72 字符**，超出部分放 body
- **"修复"类改动必须用 `fix`**，不用 `feat`
- **版本号只出现在 `release:` type 中**，不在 feat/fix 里夹带 `(vX.Y.Z)`
- **scope 可选但一致**：使用模块名（`storage`、`settings`、`pin`、`ocr`、`search`）
- **body 说明 what/why**，不堆在 first line
- **一个 commit 做一件事**：不要把 CI + release + bugfix 混在同一个 commit

### 示例

```
feat(search): 短输入 LIKE 模糊匹配 + FTS prefix fallback
fix(watcher): select_clip 后 last_hash 更新时序错误
ci: npm install → npm ci + 添加 vite build 检查
release: v0.1.16
```

## 任务闭环规则（不可违反）

Trellis task 生命周期必须与代码实际状态同步：

1. **任何代码变更必须在某个 task 的 `in_progress` 状态下进行**
   - 开发前：`python3 ./.trellis/scripts/task.py start <name>`
   - 不能在无 task 的情况下直接提交代码

2. **代码合入 dev 后，task 必须标记 `completed`**
   - 填写 `commit`、`completedAt`、`branch` 字段
   - 所有 PRD 验收项必须在代码中体现

3. **定期清理**
   - 每次 release 后 archive 已完成的 task：`task.py archive <name>`
   - 超过 2 周未推进的 task 标记 `stale` 或关闭

4. **新功能必须有 PRD**
   - 使用 `task.py create` 创建 task 目录
   - `prd.md` 必须包含：Goal、Requirements、Acceptance Criteria、Out of Scope
   - 参考 `docs/feature-lifecycle.md` 了解完整流程

## 功能开发流程

详见 [docs/feature-lifecycle.md](docs/feature-lifecycle.md)。核心流程：

```
create task → PRD → 实现 → ci-local.sh → 更新 spec → 更新 CHANGELOG → archive
```
