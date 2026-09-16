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
| 前端 | `src/js/` | ES Module，`api.ts` 是唯一 Tauri IPC 入口 |
| 后端 | `src-tauri/src/` | 扁平模块：commands / storage / clipboard_watcher / config / models / portal_shortcuts / tray_icon |
| 数据库 | SQLite + FTS5 | `clips` 表 + `clips_fts` 虚拟表，SHA-256 去重 |
| 快捷键 | X11: tauri-plugin-global-shortcut; Wayland: XDG Portal (ashpd) |

## 关键约定
- **前端 XSS 防护**：所有用户内容用 `textContent`，禁止 `innerHTML`
- **IPC 封装**：只有 `api.ts` 直接访问 `window.__TAURI__`
- **语言**：代码注释 / commit 中文，前端 UI 英文
- **构建目标**：Linux x64（deb、AppImage）、Windows x64（NSIS、MSI）、macOS Intel/Apple Silicon（DMG、updater bundle）
- **编码规范**：见项目 Wiki 的「后端编码规范」与「前端编码规范」

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

变更记录必须与代码实际状态同步：

1. **一个分支做一件事**，分支名反映改动范围，不把无关改动混入同一分支
2. **合入 dev 前必须通过 `./scripts/ci-local.sh`**，跳过的步骤不计为通过
3. **用户可见的变更必须写入 `CHANGELOG.md`**，并写清未验证的边界
4. **新功能先写清 Goal / Requirements / Acceptance Criteria / Out of Scope**，
   再动代码；验收项必须在代码或测试中有对应体现
5. **未完成项一律保留**，不写成全部完成；平台矩阵与安装包构建不计入测试通过数

## 功能开发流程

详见 [docs/feature-lifecycle.md](docs/feature-lifecycle.md)。核心流程：

```
create task → PRD → 实现 → ci-local.sh → 更新 spec → 更新 CHANGELOG → archive
```
