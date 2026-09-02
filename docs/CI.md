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
| Rust 编译检查 | `cargo check --all-targets` | 检查所有本机 Rust target |
| Rust Lint | `cargo clippy --all-targets -- -D warnings` | 所有 warning 视为 error |
| Rust 单元测试 | `cargo test` | 后端测试 |
| 前端测试 | `npx vitest run` | 前端测试（jsdom） |
| 前端类型检查 | `npx tsc --noEmit` | 检查 React/TS 功能岛 |
| 前端构建 | `npx vite build` | 验证正式前端产物 |
| 原生平台检查 | Rust check/clippy/test + 前端 test/typecheck/build | Windows 与 macOS 原生 runner 完整门禁 |

### 环境
- **Runner**: Linux 使用 `ubuntu-22.04` 作为最低构建基线；原生编译门禁使用 `windows-latest` 与 `macos-latest`
- **系统依赖**: 默认 Linux 依赖图不包含 `pipewire-rs`，因此 Jammy 无需安装新版 PipeWire 开发头文件；
  仍需 `libgbm-dev/libegl-dev/libdrm-dev/libwayland-dev/libxcb1-dev`，以及 DOM smoke 使用的
  `xvfb`/`x11-utils`
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

### 流程（5 个 Job）

```
check-version ─┬→ build-linux ───┐
               ├→ build-windows ─┼→ update-release
               └→ build-macos ───┘
```

#### 1. check-version
- 提取 tag 版本号（去掉 `v` 前缀）
- **验证 tag 版本与 `src-tauri/tauri.conf.json` 中的 `version` 字段一致**
- 检查 tag 是否在 `main` 或 `dev` 分支上（仅 warning）

#### 2. build-linux
- 安装系统依赖 + Rust + Node.js
- 在 Ubuntu 22.04 上使用 `tauri-apps/tauri-action@v1` 构建
- 产出：deb 包 + AppImage + updater 签名
- 创建 GitHub Release（draft 状态）

#### 3. build-windows
- 在 `windows-latest` x64 runner 上生成 NSIS 与 MSI
- NSIS 安装程序作为 Windows updater 入口

#### 4. build-macos
- 在 `macos-latest` ARM64 runner 上分别编译 `aarch64-apple-darwin` 与 `x86_64-apple-darwin`
- 强制使用 Developer ID Application 证书签名，并通过 Apple 服务公证
- 产出两个架构的 DMG，以及 updater 使用的 `.app.tar.gz` 和签名

#### 5. update-release
- 等待三个平台构建完成并汇总原生产物
- 从 `CHANGELOG.md` 提取当前版本的变更记录
- 生成包含下载链接的 Release Body
- 生成包含 Linux x64、Windows x64、macOS ARM64/Intel 的 `latest.json`
- 将 Release 从 draft 改为正式发布

### 发版检查清单

> **每次发版前必须确认以下事项：**

1. **版本号三处一致**：
   - `src-tauri/tauri.conf.json` → `"version": "x.y.z"`
   - `src-tauri/Cargo.toml` → `version = "x.y.z"`
   - Git tag → `vx.y.z`
2. **CHANGELOG.md** 包含 `## vx.y.z` 章节
3. **CI Check 通过**（push 到 dev/main 时自动运行）
4. **Updater Secrets 已配置**：
   - `TAURI_SIGNING_PRIVATE_KEY`
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
5. **macOS 签名与公证 Secrets 已配置**：
   - `APPLE_CERTIFICATE`（Developer ID Application `.p12` 的 Base64）
   - `APPLE_CERTIFICATE_PASSWORD`
   - `KEYCHAIN_PASSWORD`
   - `APPLE_ID`
   - `APPLE_PASSWORD`（app-specific password）
   - `APPLE_TEAM_ID`

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
| DEB | `Clippy_{version}_amd64_ubuntu22.deb` | Ubuntu 22.04+ 安装包 |
| AppImage | `Clippy_{version}_amd64_ubuntu22.AppImage` | Ubuntu 22.04+ 可执行文件 |
| NSIS | `Clippy_{version}_x64-setup.exe` | Windows 10/11 安装与自动更新 |
| MSI | `Clippy_{version}_x64.msi` | Windows 10/11 管理式安装 |
| DMG | `Clippy_{version}_aarch64.dmg` | Apple Silicon 安装包 |
| DMG | `Clippy_{version}_x64.dmg` | Intel Mac 安装包 |
| macOS updater | `Clippy_{version}_{arch}.app.tar.gz` | 签名、公证后的自动更新包 |
| Linux updater 固定名 | `Clippy_{version}_amd64.AppImage` | 与 ubuntu22 AppImage 相同，供 manifest 使用 |

### 自动更新

发布流程根据各平台的 Tauri v2 产物显式生成 `latest.json`：Linux 使用 AppImage、Windows 使用
NSIS、macOS 使用 `.app.tar.gz`。DEB、MSI 和 DMG 是人工安装入口，不写入默认 updater manifest。
签名文件内容直接嵌入 manifest，不能写成签名文件 URL。

## 常见 CI 失败原因

| 错误 | 原因 | 修复 |
|------|------|------|
| `cargo fmt -- --check` 失败 | 代码未格式化 | `cd src-tauri && cargo fmt` |
| `cargo clippy` 失败 | 代码有 warning | 按 clippy 提示修复 |
| Release 版本验证失败 | tag 版本 ≠ tauri.conf.json | 确保三处版本号一致 |
| vitest 失败 | 前端测试不通过 | `cd src && npx vitest run` 本地复现 |
| 默认依赖图出现 `pipewire v` | 可选 `linux-pipewire` 被误加入默认 feature | 保持 feature 显式 opt-in，不给 Ubuntu 22 默认包增加 PipeWire 构建依赖 |
| macOS 缺少 signing secret | Developer ID 或公证凭据未配置 | 按发版检查清单补齐全部 Apple Secrets；正式发布禁止退回 ad-hoc 签名 |
| NSIS/macOS `.sig` 缺失 | Tauri updater 私钥未配置或构建未生成 updater artifact | 检查 `TAURI_SIGNING_PRIVATE_KEY*` 与 `createUpdaterArtifacts` |

### 为什么默认包仍可在 Ubuntu 22.04 构建

Linux 默认目标固定使用 Jammy 可编译的截图依赖，不编译可选 `linux-pipewire` 增强后端；Windows
和 macOS 则通过 target-specific dependency 使用新版原生 `xcap`。CI 额外检查 Linux 默认依赖图，
一旦重新出现 `pipewire-rs` 就直接失败，从而防止发布包意外抬高 glibc 或 PipeWire 基线。
