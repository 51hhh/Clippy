# Clippy — AI Agent 速查指引

## 项目简介
Clippy 是跨平台轻量剪贴板管理器。完整的 v0.1.x 使用 Tauri 2.12.4 + Rust +
vanilla HTML/CSS/JS + React/TS；v0.2.0 正在迁移到 Slint 1.19.0，已接入共享配置、数据路径和
SQLite/FTS 历史读取，尚未完成全部功能迁移。
详细架构与数据流见 [CLAUDE.md](CLAUDE.md)；设计文档见 [docs/superpowers/specs/](docs/superpowers/specs/)。

> ⚠️ 本仓库包含两个独立项目：根目录的 **Clippy** 和 `fxxkDJTU/` 下的发票工具（Vue + TS）。二者无代码依赖。

## 常用命令
```bash
cargo tauri dev                            # 热重载开发（前端 + Rust）
cargo run -p clippy-slint                  # 当前 v0.2 原生应用
cd apps/legacy-tauri/native && cargo check                # 快速编译检查
cd apps/legacy-tauri/native && cargo test                 # Rust 单元测试
cd apps/legacy-tauri/native && cargo clippy --all-targets -- -D warnings # Lint（警告即错误）
cd apps/legacy-tauri/native && cargo fmt                  # 格式化
cd apps/legacy-tauri/frontend && npx vitest run # 旧版前端测试（jsdom）
cd apps/legacy-tauri/frontend && npx tsc --noEmit # React/TS 功能岛类型检查
./scripts/ci-local.sh                      # 本地质量预检（与 CI 一致）
./scripts/ci-local.sh --segment legacy --quick # 仅维护旧版时跳过其生产构建
./scripts/ci-windows.ps1                   # 原生 Windows 默认门禁，Linux smoke 另验
./scripts/ci-windows.ps1 -RecordingQa      # 追加 Windows 录屏 QA feature 门禁
```

## 架构要点
| 层 | 路径 | 说明 |
|----|------|------|
| legacy 前端 | `apps/legacy-tauri/frontend/js/` | ES Module；`api.ts` 是公共 facade，`api/*.ts` 是受控 Tauri IPC 边界 |
| legacy 后端 | `apps/legacy-tauri/native/src/` | Tauri v2 入口与 Rust 业务，v0.1.x 维护线 |
| Slint 应用 | `apps/slint/` | Slint 1.19.0 原生 UI，v0.2.0 及以上 |
| 共享领域 | `crates/clippy-core/` | UI 无关模型、配置与业务协议 |
| 平台层 | `crates/clippy-platform/` | 部署身份、兼容旧版的数据路径与私有文件能力 |
| 存储层 | `crates/clippy-storage/` | SQLite + FTS5、迁移、搜索与有界专用工作线程 |
| 快捷键 | X11: tauri-plugin-global-shortcut; Wayland: XDG Portal (ashpd) |

## 关键约定
- **前端 XSS 防护**：用户纯文本用 `textContent`；富文本 `innerHTML` 仅限
  `scripts/check-html-sinks.mjs` 登记且经 DOMPurify 清洗的渲染点
- **IPC 封装**：业务代码只从 `api.ts` 导入；只有 `scripts/frontend-api-boundary.mjs`
  登记的 `api/*.ts` 领域模块可导入 `@tauri-apps/*` 或直接访问 `window.__TAURI__`
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
2. **合入 dev 前必须通过 `./scripts/ci-local.sh`**，跳过的步骤不计为通过。该脚本只验证当前宿主平台，
   Windows / macOS 的条件编译代码不在本机编译图内；发布前须确认 `Check (ubuntu-22.04)`、
   `Native Check (windows-latest)`、`Native Check (macos-latest)` 在**同一个 SHA** 上均为 success
3. **用户可见的变更必须写入 `CHANGELOG.md`**，并写清未验证的边界
4. **新功能先写清 Goal / Requirements / Acceptance Criteria / Out of Scope**，
   再动代码；需求必须有稳定的 GitHub issue、Wiki 页面或仓库文档 ID，PR 和用户可见 CHANGELOG
   引用同一 ID；验收项必须在代码或测试中有对应体现
5. **未完成项一律保留**，不写成全部完成；平台矩阵与安装包构建不计入测试通过数
6. **验证证据分层记录**：本地完整门禁、可选交叉检查、同一 SHA 原生 CI、Native/人工 QA
   不能互相替代；跳过项不计为通过

## 功能开发流程

详见 [docs/feature-lifecycle.md](docs/feature-lifecycle.md)。核心流程：

```
create task → PRD → 实现 → ci-local.sh → 更新 spec → 更新 CHANGELOG → archive
```
