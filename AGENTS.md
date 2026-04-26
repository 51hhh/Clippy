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
Clippy 是跨平台轻量剪贴板管理器。技术栈：Tauri v2 + Rust（后端）+ vanilla HTML/CSS/JS（前端）。  
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
- **构建目标**：仅 Linux（deb, AppImage）
- **编码规范**：见 `.trellis/spec/backend/` 和 `.trellis/spec/frontend/`
