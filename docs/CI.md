# CI/CD 文档

## 概览

本项目使用 GitHub Actions 进行持续集成、真机 QA 产物交付和正式发布。共有三个 workflow：

| Workflow | 文件 | 触发条件 | 用途 |
|----------|------|----------|------|
| CI Check | `.github/workflows/build.yml` | push/PR 到 `dev`/`main` | 代码质量检查 |
| Native QA Packages | `.github/workflows/native-qa.yml` | 手动触发 | 四架构 QA 包与 Ubuntu 24 运行证据 |
| Release  | `.github/workflows/release.yml` | push `v*.*.*` 标签 | 构建四个 updater 目标 + 发布 |

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
| 原生平台检查 | Rust check/clippy/test | Windows 与 macOS 原生 runner 编译、平台 API 与测试门禁 |

### 环境
- **Runner**: Linux 使用 `ubuntu-22.04` 作为最低构建基线；原生编译门禁使用 `windows-latest` 与 `macos-latest`
- **系统依赖**: 默认 Linux 依赖图不包含 `pipewire-rs`，因此 Jammy 无需安装新版 PipeWire 开发头文件；
  仍需 `libgbm-dev/libegl-dev/libdrm-dev/libwayland-dev/libxcb1-dev`，以及 DOM smoke 使用的
  `xvfb`/`x11-utils`
- **Rust**: stable（含 clippy + rustfmt 组件）
- **Node.js**: 24
- **缓存**: `Swatinem/rust-cache@v2`（加速 Rust 编译）

平台无关的前端测试、类型检查和构建只在 Jammy 执行一次；Windows/macOS runner 专注 Rust 的目标条件
编译、lint 和单元测试。安装包构建移入独立 Native QA workflow，避免每次 push 重复生成约 190 MB
测试产物，也避免普通 CI 在界面上与正式发布混淆。

## Native QA Packages（native-qa.yml）

在 GitHub Actions 中选择待测 ref 并手动运行。workflow 会显式合并公共配置、对应平台配置和关闭 updater
附加产物的 `tauri.ci.conf.json`，保留 14 天并上传绑定完整 commit SHA 的四套安装包：

- Ubuntu 22 构建的 x64 deb 与已移除宿主 Wayland ABI 库的 AppImage；
- 临时自签名并核对 Authenticode 的 Windows x64 NSIS 与 MSI；
- ad-hoc 签名的 macOS Apple Silicon DMG；
- ad-hoc 签名的 macOS Intel DMG。

Linux 产物先装入 tar 以保留 AppImage 执行权限。每套产物均含 `QA-BUILD.txt` 与 `SHA256SUMS.txt`；同一
run 还会上传 `qa-record-templates-<SHA>`，其中是绑定该 SHA、应用版本和九个目标环境的结构化记录模板。
Linux 包由 Ubuntu 22.04 构建后，独立的 Ubuntu 24.04 runner 下载同一 tar、校验 SHA-256，并强制执行
AppImage X11 窗口几何、首帧和单实例 smoke；smoke 缺依赖或缺产物会失败，不会静默跳过。

这些安装包明确是 `unsigned-qa-only`、`self-signed-qa-only` 或 `ad-hoc-qa-only`，只用于功能测试。
Windows QA 包可验证 Authenticode 摘要与 signer 一致，但不具备公共 CA 信任，也不能用于 Tauri updater
签名验收。当前正式 release 的 macOS 包同样采用 Ad-Hoc 签名，能验证
产物完整性与架构，但不具备 Developer ID、公证或 Gatekeeper 公共信任；发布说明必须明确这一限制。

### 原生 runner 证据

本地分支推送并完成 CI 后，用完整 commit SHA 校验三个必需 job，避免把旧 workflow、其它分支或只有
Linux 的 run 误记为跨平台通过：

```bash
node scripts/verify-native-ci.mjs \
  --repo 51hhh/Clippy \
  --sha <40位commit SHA> \
  --output native-ci-evidence.md
```

校验器只读取 GitHub check-runs API；仅当 `Check (ubuntu-22.04)`、
`Native Check (windows-latest)` 和 `Native Check (macos-latest)` 对该 SHA 都是
`completed/success` 时返回 0。输出的 Markdown 可直接归档到任务验证记录。公开仓库无需 token；受限
环境或私有仓库可通过 `GITHUB_TOKEN` / `GH_TOKEN` 提供只读权限，脚本不会打印 token。

Native QA 之后的权限、焦点、输入注入、混合 DPI、Spaces 和 Wayland compositor 行为按
[`native-qa.md`](native-qa.md) 生成结构化真机记录；CI 绿色不能替代这些场景。

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

### 流程

```
check-version → release-preflight ─┬→ build-linux ──────┐
                                   ├→ build-windows ────┤
                                   ├→ build-macos ARM ──┼→ update-release
                                   └→ build-macos Intel ┘
```

#### 1. check-version
- 校验并提取 SemVer tag（去掉 `v` 前缀）
- **验证 tag、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` 三处版本完全一致**
- 验证 `CHANGELOG.md` 存在精确的 `## vX.Y.Z` 标题
- 验证 tag commit 可从 `main` 或 `dev` 到达；不满足时停止发布，不再只给 warning

#### 2. release-preflight
- 使用完整 tag SHA 核对 `CI Check` 的 Ubuntu、Windows、macOS 三项原生门禁均为成功
- updater 私钥缺失时停止；Windows PFX 成对配置，未配置时选择自签名模式
- macOS Intel/ARM 固定启用并采用 Ad-Hoc 签名，不依赖 Apple Developer 凭据

#### 3. build-linux
- 安装系统依赖 + Rust + Node.js
- 在 Ubuntu 22.04 上使用 `tauri-apps/tauri-action@v1` 构建
- 产出：deb 包 + AppImage + updater 签名
- 上传到 `release-linux-x64` workflow artifact，不提前创建 GitHub Release

#### 4. build-windows
- 在 `windows-latest` x64 runner 上生成 NSIS 与 MSI
- NSIS 安装程序作为 Windows updater 入口
- 优先使用仓库 PFX；未配置时在 runner 的个人证书库生成临时自签名代码签名证书，不写入 Root 信任库
- 自签名模式只在临时 runner 的 `LocalMachine\TrustedPeople` 中信任该证书公钥，使 Authenticode 必须
  严格返回 `Valid`；验证后同时删除个人证书与临时信任。两种模式都核对 signer thumbprint，仍拒绝
  无签名、哈希不匹配和错误 signer，并在 Release Notes 标注
  SmartScreen/未知发布者风险

#### 5. build-macos
- 在 `macos-latest` ARM64 runner 上分别编译 `aarch64-apple-darwin` 与 `x86_64-apple-darwin`
- 通过 `signingIdentity: "-"` 使用 Tauri 官方支持的 Ad-Hoc 签名，并验证 `codesign --strict` 与
  `Signature=adhoc`
- 上传前用 `lipo` 核对可执行文件与矩阵目标架构一致；产物不做 Developer ID 签名或 Apple 公证
- 产出两个架构的 DMG，以及 updater 使用的 `.app.tar.gz` 和签名

#### 6. update-release
- 等待四个目标构建完成并汇总原生产物
- 所有目标成功后才创建 GitHub Release draft
- 从 `CHANGELOG.md` 提取当前版本的变更记录
- 生成包含下载链接的 Release Body
- 生成固定包含 Linux x64、Windows x64、macOS ARM64/Intel 的 `latest.json`
- 将 Release 从 draft 改为正式发布

### 发版检查清单

> **每次发版前必须确认以下事项：**

1. **版本号三处一致**：
   - `src-tauri/tauri.conf.json` → `"version": "x.y.z"`
   - `src-tauri/Cargo.toml` → `version = "x.y.z"`
   - Git tag → `vx.y.z`
2. **CHANGELOG.md** 包含 `## vx.y.z` 章节
3. **同一 commit SHA 的 CI Check 通过**（release preflight 会再次调用 GitHub check-runs API）
4. **Updater Secrets 已配置**：
   - `TAURI_SIGNING_PRIVATE_KEY`
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
5. **macOS 签名限制已确认**：
   - Intel 与 Apple Silicon 均固定构建，不需要 Apple Secrets。
   - `signingIdentity: "-"` 只提供 Ad-Hoc 代码签名，不提供 Developer ID 身份、公证或 Gatekeeper 信任。
   - 首次打开时用户可能需要在“系统设置 → 隐私与安全性”中手动允许；发布说明必须保留该警告。
6. **Windows 代码签名模式已确认**：
   - Tauri updater 的 `.sig` 只验证更新包完整性，不是 Windows Authenticode 签名。
   - `WINDOWS_CERTIFICATE`（含代码签名私钥的 `.pfx` Base64）
   - `WINDOWS_CERTIFICATE_PASSWORD`（PFX 导出密码）
   - 两项都没有时，release 生成临时自签名代码签名证书；它只能验证文件签名完整性，不建立公共信任，
     SmartScreen 仍可能显示未知发布者。发布说明必须保留该警告。
   - release 会检查证书私钥、代码签名 EKU 和有效期，以 SHA-256 + RFC 3161 时间戳签名，并在上传前
     验证 NSIS/MSI 的 Authenticode 摘要及 signer thumbprint；临时自签名公钥只在 runner 的
     `LocalMachine\TrustedPeople` 中短暂存在，使校验必须严格返回 `Valid`，随后立即删除。证书不进入
     Root 信任库，任何其它状态都会阻止发布 Windows 产物。

### 发版命令

```bash
# 1. 更新版本号（tauri.conf.json + Cargo.toml）
# 2. 更新 CHANGELOG.md
# 3. 显式暂存本次版本文件和 CHANGELOG，避免带入无关工作区文件
git add src-tauri/tauri.conf.json src-tauri/Cargo.toml CHANGELOG.md
git commit -m "release: vX.Y.Z"

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
| DMG | `Clippy_{version}_aarch64.dmg` | Ad-Hoc 签名的 Apple Silicon 安装包 |
| DMG | `Clippy_{version}_x64.dmg` | Ad-Hoc 签名的 Intel Mac 安装包 |
| macOS updater | `Clippy_{version}_{arch}.app.tar.gz` | Ad-Hoc 签名的自动更新包 |
| Linux updater 固定名 | `Clippy_{version}_amd64.AppImage` | 与 ubuntu22 AppImage 相同，供 manifest 使用 |

### 自动更新

发布流程根据本次真实存在的 Tauri v2 产物显式生成 `latest.json`：Linux 使用 AppImage、Windows 使用
NSIS，macOS 使用两个 `.app.tar.gz`。DEB、MSI 和 DMG 是人工安装入口，不写入 updater manifest；
任何一个目标缺少产物或签名都会阻止整次发布，避免生成死链接。
签名文件内容直接嵌入 manifest，不能写成签名文件 URL。

`scripts/generate-updater-manifest.mjs` 是 manifest 的唯一生成入口。它把四个 `OS-ARCH` key、平台
Tauri bundle target、标准化 artifact 名和签名文件名绑定在同一份受测契约中，并拒绝缺签名、错误
SemVer、非 UTC RFC 3339 时间或非 HTTPS 下载地址。release workflow 只负责汇总构建产物和调用该
生成器，不再在 YAML 中维护第二份平台映射。

## 常见 CI 失败原因

| 错误 | 原因 | 修复 |
|------|------|------|
| `cargo fmt -- --check` 失败 | 代码未格式化 | `cd src-tauri && cargo fmt` |
| `cargo clippy` 失败 | 代码有 warning | 按 clippy 提示修复 |
| Release 版本验证失败 | tag 不是 SemVer、三处版本不一致、CHANGELOG 缺章节或 tag 不在发布分支 | 修正版本、变更日志或 tag 来源后重新创建 tag |
| vitest 失败 | 前端测试不通过 | `cd src && npx vitest run` 本地复现 |
| 默认依赖图出现 `pipewire v` | 可选 `linux-pipewire` 被误加入默认 feature | 保持 feature 显式 opt-in，不给 Ubuntu 22 默认包增加 PipeWire 构建依赖 |
| macOS Ad-Hoc 校验失败 | `.app` 没有严格签名、不是 `Signature=adhoc` 或架构不匹配 | 检查 macOS 覆盖配置、目标 triple 与 Tauri bundle 输出 |
| NSIS/macOS `.sig` 缺失 | Tauri updater 私钥未配置或构建未生成 updater artifact | 检查 `TAURI_SIGNING_PRIVATE_KEY*` 与 `createUpdaterArtifacts` |
| Windows 签名门禁失败 | PFX 只配置一项、证书无私钥/代码签名 EKU、已过期，或安装包签名无效 | 成对修复 `WINDOWS_CERTIFICATE*`，或全部移除以使用明确标注的临时自签名模式 |

### 为什么默认包仍可在 Ubuntu 22.04 构建

Linux 默认目标固定使用 Jammy 可编译的截图依赖，不编译可选 `linux-pipewire` 增强后端；Windows
和 macOS 则通过 target-specific dependency 使用新版原生 `xcap`。CI 额外检查 Linux 默认依赖图，
一旦重新出现 `pipewire-rs` 就直接失败，从而防止发布包意外抬高 glibc 或 PipeWire 基线。
