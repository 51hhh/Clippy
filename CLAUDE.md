# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目简介

Clippy 是跨平台轻量剪贴板管理器，基于 Tauri v2 + Rust（后端）+ vanilla HTML/CSS/JS（主前端）+ React/TS（截图编辑功能岛）。当前架构见 `docs/architecture.md`，历史设计见 `docs/superpowers/specs/2026-04-24-clippy-clipboard-manager-design.md`。

已完成功能：剪贴板监听、SQLite 存储（含 FTS5 全文搜索）、悬浮面板、搜索、系统托盘、
X11/Wayland 分流自动粘贴、多显示器冻结截图、二维手动长截图、X11 受控自动滚动、Pin 工作区/内部无损图片修订、
可校验的本地批量归档交换、无限画布图片查看器、QR Code/Code 39/Code 128/EAN-13 扫码、Tesseract OCR、
可选结构化增强 OCR、翻译和设置面板。录屏的持久化、帧源、VP9 原型、控制链、结果/恢复库、异常
分段无损 remux、持久首帧缩略图及按需 WebM 库内播放已实现；产品入口仅在
显式 QA 构建的原生 X11、Wayland、Windows 与 macOS 12.3+ 会话开放，默认构建仍保持关闭。
内部录屏音频已建立 48 kHz 单音轨合同、共享时钟、线程内采集 worker，以及 feature 门控的
VP9 + Opus 可恢复双轨 session；它能生成 schema v2 的最终 WebM 和独立周期分段。Windows
QA 组合 feature 已把 WGC 与 WASAPI 系统声/默认麦克风接入该 session，并在录屏选区工具条由后端
下发可用模式；默认仍为无音频。macOS/Linux 音源、设备选择/混音和默认发布仍未接入，Windows
真机设备与长时漂移仍待验收。
macOS Intel/Apple Silicon QA 包已生成；Wayland 使用可信 Portal 父窗口和托盘控制，但两者的原生
真机录制仍待验收，不能记为发布可用。
增强 OCR 运行时需要显式配置，未随三平台安装包默认分发；交付边界见
`src-tauri/ocr-sidecar/README.md`。
智能擦除已完成 `PX-SMART-01` 可行性门控，但当前 LaMa 候选未通过强边缘质量、CPU 延迟和内存
阈值，因此尚无产品入口或模型依赖；证据见 `docs/reviews/2026-09-21-smart-erase-feasibility.md`。

## 开发环境搭建（Ubuntu）

```bash
# 1. Tauri v2 的系统依赖
sudo apt update && sudo apt install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev \
  pkg-config

# 2. 本项目额外依赖
#   libpipewire-0.3-dev  — ashpd 的 screencast feature（libspa-sys 需要 pkg-config）
#   libgbm-dev 等        — libwayshot-xcap 的 Wayland 截图链接依赖（-lgbm/-lEGL）
#   xvfb                 — scripts/smoke-dom.sh 与 ci-local.sh 的无头 DOM smoke
sudo apt install -y \
  libpipewire-0.3-dev \
  libgbm-dev libegl1-mesa-dev libdrm-dev libwayland-dev libxcb1-dev \
  xvfb

# 3. Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"

# 4. Node.js（最低 20.19，推荐当前 LTS）
curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
sudo apt install -y nodejs

# 5. Tauri CLI（cargo 侧；npm 侧的同版本 CLI 已作为 src/ 的 devDependency 锁进 lockfile，
#    `cd src && npm ci` 之后 `npx tauri` 即可用，两者版本保持一致）
cargo install tauri-cli --version "^2" --locked
```

## 常用命令

```bash
# 启动开发服务器（热重载前端 + Rust 后端）
cargo tauri dev
# 没装 cargo-tauri 时用 npm 侧的同版本 CLI（已锁在 src/package.json，不依赖 npx 缓存）
cd src && npx tauri dev

# 构建发布包（输出到 src-tauri/target/release/bundle/）
cargo tauri build

# 仅编译 Rust 后端（快速检查编译错误）
cd src-tauri && cargo check --all-targets

# 运行 Rust 单元测试
cd src-tauri && cargo test

# 运行单个测试
cd src-tauri && cargo test test_name

# 格式化 + lint
cd src-tauri && cargo fmt
cd src-tauri && cargo clippy --all-targets -- -D warnings

# 性能基线（criterion，门禁只编译不运行；数字与坑见 docs/bench-baseline.md）
cd src-tauri && cargo bench

# 完整本地门禁（含 DOM/Xvfb smoke 与前端生产构建）
# 仅覆盖当前宿主平台；Windows / macOS 只能由远程 Native Check 判定
./scripts/ci-local.sh
```

## 架构概览

```
前端 (src/)
├── index.html / settings.html         — 主窗口与设置
├── launcher.html                      — 键盘优先的受限动作启动器
├── pin.html / viewer.html             — Pin 与图片查看器功能岛入口
├── capture-overlay.html               — 冻结截图覆盖层入口
├── longshot-controller.html           — 长截图控制窗口入口
├── recording-control.html             — 录屏暂停、继续与停止控制窗口
├── recordings.html                    — 完整录屏与中断分段的结果/恢复库
├── js/
│   ├── api.ts / ipc-types.ts          — 类型化 IPC 边界与 serde 合同
│   ├── app.js / clipboard-list.js     — 主窗口路由与列表状态
│   ├── preview-panel.js / preview/    — 预览调度与分职责渲染器
│   ├── translation-*.js               — 翻译面板与设置
│   └── settings.js / theme.js         — 设置与主题
├── react/
│   ├── main/                          — 主窗口 React 局部面板与状态
│   ├── annotation/                    — 截图、Pin、查看器共享标注核心
│   ├── capture-overlay/               — 冻结画面选区、标注与选区翻译
│   ├── longshot-controller/           — 长截图控制窗口
│   ├── recording-library/             — 录屏缩略图、播放、导出、恢复合并、定位与受限删除
│   ├── launcher/                      — 动作搜索、参数、运行/取消与稳定错误状态
│   ├── viewer/                        — 无限画布图片查看器及工具
│   ├── pin/                           — 贴图、编辑与保存协议
│   └── shared/                        — i18n 与工具栏共享行为
└── styles/                            — base/components/settings/themes

Rust 后端 (src-tauri/src/)
├── lib.rs / main.rs                   — Tauri 初始化与入口
├── commands.rs / commands/            — AppState 与按功能 IPC 命令
├── actions/                            — 类型化动作注册表、受限 Launcher 与领域适配器
├── clipboard_watcher.rs / storage.rs  — 剪贴板监听与 SQLite/FTS5
├── archive.rs / storage/archive.rs    — `.clippy.zip` 校验、编解码与事务合并
├── paste/ / window_controller.rs      — X11/Portal 粘贴与窗口几何
├── capture/ / screenshot.rs           — CaptureSession 与平台截图
├── recording/                         — X11/WGC/ScreenCaptureKit 帧源、时间线、编码/remux、缩略图、恢复、结果库与控制窗门控
├── pin/ / pin_window.rs               — Pin command adapter、生命周期、可信输出与窗口适配
├── translation/ / ocr.rs / ocr/       — 翻译服务；OCR facade、运行时、探测、进程与协议
└── config.rs / models.rs              — 配置与共享模型
```

### 数据流

1. `ClipboardWatcher` 独立线程每 500ms 轮询系统剪贴板（arboard）；程序化写入不必等这一轮——`clipboard_watcher/writer.rs` 写成功后敲 `wake::nudge()`，轮询等待当场结束
2. SHA-256 哈希去重 → 重复内容递增持久 `use_order` 置顶
3. 写入 SQLite `clips` 表 + 同步 `clips_fts` FTS5 虚拟表
4. `app.emit("clip-added")` / `app.emit("clip-removed")` 通知前端
5. 前端 `api.ts` 监听事件 → `clipboard-list.js` 增量更新 DOM

### 反向写入（select_clip）

用户选中条目 → 写入系统剪贴板 → 通过 `skip_hash` 机制让 watcher 跳过此次变更，避免重复存储。

### 窗口管理

- **main** 窗口：无边框悬浮面板（380×500），`visible: false` 启动，全局快捷键切换显隐
- **settings** 窗口：按需创建（从托盘菜单或 IPC `show_settings` 命令），不在 `tauri.conf.json` 中预声明

### 共享状态

`AppState` 通过 `app.manage()` 注入，是以下领域所有者的组合根：

| 领域 | 所有者 |
|---|---|
| 剪贴板与持久化 | `StorageEngine`、`ClipboardWatcher`、`AppConfig` |
| 主窗口 | `window_controller`、`app/window_events` 的转换与位置状态 |
| 截图与长截图 | `CaptureManager`、`CaptureModeGate`、`LongshotLifecycle`、`LongshotControllerRegistry` |
| 录屏 | `RecordingLifecycle`、`RecordingManager`、`RecordingControlRegistry` |
| Pin 与查看器 | `PinManager`、`PinWorkspacePersistence`、`PinOriginRegistry`、`ViewerManager` |
| 自动化与服务 | `PasteManager`、`TranslationService`、快捷键/Portal worker |

具体字段以 `src-tauri/src/commands.rs` 和 owner 类型为准。命令 adapter 只借用 owner 并做参数转换，
不要把领域状态机重新放回 `AppState` 或 `commands.rs`。

### 前端约定

- 主界面无框架，纯 HTML/CSS/JS + ES Module `<script type="module">`
- React/TS 功能岛是 `capture-overlay/`、`longshot-controller/`、`viewer/`、`pin/` 与主窗口局部
  `main/`；`annotation/` 和 `shared/` 只提供跨岛共享核心，不持有页面生命周期
- 使用 Vite 作为开发服务器和构建工具（`src/vite.config.mjs`）
- 业务模块只从 `api.ts` 公共 facade 导入；只有 `scripts/frontend-api-boundary.mjs` 登记的
  `api/*.ts` 领域模块允许直接访问 Tauri IPC
- 所有用户纯文本通过 React 文本节点或 `textContent` 写入 DOM；只有
  `scripts/check-html-sinks.mjs` 登记的 sink 能写 `innerHTML`，且必须先经 DOMPurify 严格清洗
- `scripts/check-html-sinks.mjs` 固定富文本 sink 数量，并禁止未登记模块导入 `@tauri-apps/*`；
  `tsconfig.js.json` 与 Promise 门禁增量覆盖 `js/preview/`、`js/settings/`
- HTML 实体解码使用隔离 `DOMParser`，禁止 `Function`/`eval` 动态执行
- 翻译 API key 只写系统 Secret Service；Wayland restore token 使用单独 0600 文件，不进入 AppConfig

## 关键设计约束

- 剪贴板监听采用轮询（~500ms），不使用系统通知机制；但**自己写入**时会唤醒轮询（`clipboard_watcher/wake.rs`），否则"截图复制完立刻 Pin"会取到上一条
- 内容去重基于 SHA-256 哈希（`content_hash` 字段 UNIQUE 约束）
- 收藏条目不受历史上限清理影响（`cleanup_old_entries` 跳过 `is_favorite = 1`）
- SQLite 数据库位于 Tauri app data 目录，`config.storage_mode` 可切换为 `"memory"`
- FTS 索引需手动同步：插入时同步写 `clips_fts`，删除时用 FTS5 `'delete'` 命令清理
- 公共 `tauri.conf.json` 不含平台专属 bundle；Linux、Windows、macOS 覆盖配置分别生成
  deb/AppImage、NSIS/MSI 与 Intel/Apple Silicon app/DMG
- 前端通过 Vite 构建输出到 `dist/`，Tauri 从 `frontendDist: "../dist"` 加载静态文件

## 语言约定

- 代码注释、commit message、文档使用**中文**
- 前端 UI 文本使用**英文**
