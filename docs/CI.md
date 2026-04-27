# CI/CD 文档

## 概览

本项目使用 GitHub Actions 进行持续集成和发布。共有两个 workflow：

| Workflow | 文件 | 触发条件 | 用途 |
|----------|------|----------|------|
| CI Check | `.github/workflows/build.yml` | push/PR 到 `dev`/`main` | 代码质量检查 |
| Release  | `.github/workflows/release.yml` | push `v*.*.*` 标签 | 构建 + 发布 |

## CI Check（build.yml）

### 触发条件
- push 到 `dev` 或 `main` 分支
- PR 目标为 `dev` 或 `main`
- 手动触发（workflow_dispatch）

### 检查步骤

| 步骤 | 命令 | 说明 |
|------|------|------|
| Rust 格式检查 | `cargo fmt -- --check` | 代码格式必须符合 rustfmt 标准 |
| Rust Lint | `cargo clippy -- -D warnings` | 所有 warning 视为 error |
| Rust 单元测试 | `cargo test` | 后端测试 |
| 前端测试 | `npx vitest run` | 前端测试（jsdom） |

### 环境
- **Runner**: `ubuntu-22.04`
- **Rust**: stable（含 clippy + rustfmt 组件）
- **Node.js**: 20
- **缓存**: `Swatinem/rust-cache@v2`（加速 Rust 编译）

### 本地复现

```bash
# 格式检查
cd src-tauri && cargo fmt -- --check

# Lint
cd src-tauri && cargo clippy -- -D warnings

# Rust 测试
cd src-tauri && cargo test

# 前端测试
cd src && npx vitest run
```

## Release（release.yml）

### 触发条件
- push 符合 `v*.*.*` 格式的 tag（如 `v0.1.4`）

### 流程（3 个 Job）

```
check-version → build-linux → update-release
```

#### 1. check-version
- 提取 tag 版本号（去掉 `v` 前缀）
- **验证 tag 版本与 `src-tauri/tauri.conf.json` 中的 `version` 字段一致**
- 检查 tag 是否在 `main` 或 `dev` 分支上（仅 warning）

#### 2. build-linux
- 安装系统依赖 + Rust + Node.js
- 使用 `tauri-apps/tauri-action@v0.5` 构建
- 产出：deb 包 + AppImage（含自动更新 JSON）
- 创建 GitHub Release（draft 状态）
- 需要 Secrets：
  - `TAURI_SIGNING_PRIVATE_KEY` — AppImage 签名私钥
  - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — 签名密码

#### 3. update-release
- 从 `CHANGELOG.md` 提取当前版本的变更记录
- 生成包含下载链接的 Release Body
- 将 Release 从 draft 改为正式发布

### 发版检查清单

> **每次发版前必须确认以下事项：**

1. **版本号三处一致**：
   - `src-tauri/tauri.conf.json` → `"version": "x.y.z"`
   - `src-tauri/Cargo.toml` → `version = "x.y.z"`
   - Git tag → `vx.y.z`
2. **CHANGELOG.md** 包含 `## vx.y.z` 章节
3. **CI Check 通过**（push 到 dev/main 时自动运行）
4. **Secrets 已配置**（`TAURI_SIGNING_PRIVATE_KEY` + `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`）

### 发版命令

```bash
# 1. 更新版本号（tauri.conf.json + Cargo.toml）
# 2. 更新 CHANGELOG.md
# 3. 提交
git add -A
git commit -m "chore: bump version to x.y.z"

# 4. 打 tag
git tag -a vx.y.z -m "vx.y.z: 简要说明"

# 5. 推送
git push origin dev
git push origin vx.y.z
```

### 产出物

| 格式 | 文件名 | 说明 |
|------|--------|------|
| DEB | `Clippy_{version}_amd64.deb` | Debian/Ubuntu 安装包 |
| AppImage | `Clippy_{version}_amd64.AppImage` | 通用 Linux 可执行文件，支持自动更新 |

### 自动更新

`tauri-action` 会生成 `latest.json`，AppImage 版本可通过内置更新器检查新版本。DEB 包不支持自动更新，需手动下载。

## 常见 CI 失败原因

| 错误 | 原因 | 修复 |
|------|------|------|
| `cargo fmt -- --check` 失败 | 代码未格式化 | `cd src-tauri && cargo fmt` |
| `cargo clippy` 失败 | 代码有 warning | 按 clippy 提示修复 |
| Release 版本验证失败 | tag 版本 ≠ tauri.conf.json | 确保三处版本号一致 |
| vitest 失败 | 前端测试不通过 | `cd src && npx vitest run` 本地复现 |
