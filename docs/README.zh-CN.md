<p align="center">
  <img src="banner.png" alt="Clippy Banner" width="600">
</p>

<p align="center">
  <strong>轻量、极速的 Linux 剪贴板管理器</strong>
</p>

<p align="center">
  <a href="https://github.com/51hhh/Clippy/actions/workflows/build.yml">
    <img src="https://github.com/51hhh/Clippy/actions/workflows/build.yml/badge.svg" alt="CI">
  </a>
  <a href="https://github.com/51hhh/Clippy/releases/latest">
    <img src="https://img.shields.io/github/v/release/51hhh/Clippy?color=blue" alt="Release">
  </a>
  <a href="https://github.com/51hhh/Clippy/blob/main/LICENSE">
    <img src="https://img.shields.io/github/license/51hhh/Clippy" alt="License">
  </a>
  <a href="https://github.com/51hhh/Clippy/releases">
    <img src="https://img.shields.io/github/downloads/51hhh/Clippy/total" alt="Downloads">
  </a>
</p>

<p align="center">
  <a href="../README.md">English</a> | <a href="README.zh-CN.md">中文</a>
</p>

---

Clippy 安静地驻留在系统托盘，后台监听剪贴板变化，让你通过一个快捷键即可搜索和回溯所有复制过的内容——文本、HTML、图片。

基于 **Tauri v2 + Rust** 构建，没有 Electron，没有臃肿。

## 截图展示

<table>
  <tr>
    <td align="center"><img src="image1.png" width="300"><br><sub>剪贴板列表</sub></td>
    <td align="center"><img src="image4.png" width="500"><br><sub>代码预览 + 语法高亮</sub></td>
  </tr>
  <tr>
    <td align="center"><img src="image2.png" width="300"><br><sub>设置面板 + 主题切换</sub></td>
    <td align="center"><img src="image3.png" width="500"><br><sub>图片预览 + 设置窗口</sub></td>
  </tr>
</table>

## 功能特性

- **多类型剪贴板** — 自动捕获文本、HTML 和图片，SHA-256 哈希去重
- **tmux 集成** — 通过 `copy-pipe-and-cancel` 捕获 tmux copy-mode 复制内容，inotify 事件驱动即时检测，自动绑定验证
- **富文本预览** — 按 `Tab` 键展开预览面板：
  - 代码高亮 — 自动检测 21 种编程语言，highlight.js 渲染
  - Markdown 渲染 — 多特征评分检测，支持 GFM 语法
  - HTML 安全渲染 — DOMPurify 过滤后的富文本预览
  - 图片预览 — 内联 PNG 预览，显示分辨率信息
- **即时搜索** — SQLite FTS5 全文搜索，毫秒级响应
- **悬浮面板** — 无边框弹出窗口，全局快捷键唤起，失焦自动隐藏
- **键盘驱动** — 完整键盘导航，支持 Vim 风格按键 (WASD)
- **全局快捷键** — 同时支持 X11（`tauri-plugin-global-shortcut`）和 Wayland（XDG Portal / gsettings）
- **自动粘贴** — X11 恢复原目标窗口；Wayland 复用 RemoteDesktop Portal 持久会话，不可用时安全回退为仅复制
- **截图工作流** — 多显示器冻结选区、窗口命中、移动/缩放，可直接复制/保存/Pin/编辑；本地 OCR 后仅发送文字翻译
- **Pin 与图片编辑** — 文本、图片、截图统一贴图控件，支持对象标注、模糊/马赛克、撤销重做和图像调整；可保存“合成预览 + 原图/操作”的单文件可编辑 PNG，也可导出不含原图的安全扁平 PNG，并从主窗口 Ctrl+O 继续编辑
- **翻译集成** — LibreTranslate-compatible/OpenAI-compatible 服务、Secret Service 密钥、超时重试和敏感内容保护
- **6 款主题** — 亚麻、石墨、极地、晒纸、玫瑰、深夜
- **收藏夹** — 置顶重要条目，不受历史清理影响
- **自动更新** — 内置更新器，从 GitHub Releases 获取新版本
- **国际化** — 中文 / 英文

## 安装

前往 [Releases](https://github.com/51hhh/Clippy/releases/latest) 下载最新的 `.deb` 或 `.AppImage`。

```bash
# Debian / Ubuntu
sudo dpkg -i clippy_*.deb

# AppImage
chmod +x Clippy_*.AppImage && ./Clippy_*.AppImage
```

**OCR（可选）：** 图片文字识别需要 tesseract。可在 设置 → OCR → 点击"一键安装"按钮，或手动安装：

```bash
sudo apt install tesseract-ocr tesseract-ocr-chi-sim
```

未安装 tesseract 时，Clippy 正常运行，设置页会显示安装状态和一键安装按钮。

## 从源码构建

前置要求：Rust 工具链、Node.js ≥ 20.19、Tauri v2 系统依赖。

```bash
sudo apt install -y \
  libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev pkg-config

cargo install tauri-cli --version "^2"

git clone https://github.com/51hhh/Clippy.git && cd Clippy
cd src && npm install && cd ..
cargo tauri build
```

构建产物：`src-tauri/target/release/bundle/`

> **提示**：若预构建的 deb 在你的系统上无法运行（如系统库版本不匹配），从源码编译可确保完全兼容。

## 技术栈

| 层 | 技术 |
|----|-----|
| 框架 | [Tauri v2](https://v2.tauri.app/) |
| 后端 | Rust — arboard, rusqlite, sha2, image, ashpd |
| 前端 | 原生 HTML / CSS / JS (ES Modules) |
| 预览 | highlight.js · marked · DOMPurify |
| 数据库 | SQLite + FTS5 |
| 构建 | Vite（多页面） |

## 架构

当前模块所有权、截图/Pin/翻译流程和平台边界见 [architecture.md](architecture.md)。

参考项目（Flashot/Translator）的截图、翻译和授权设计取舍见 [reference-project-guidance.md](reference-project-guidance.md)。

```mermaid
flowchart LR
    CB["系统剪贴板"]
    CW["ClipboardWatcher\n(文本 → HTML → 图片)"]
    DB[("SQLite + FTS5")]
    BE["Rust 后端\n(Tauri IPC)"]
    FE["前端\n(列表 + 预览)"]
    TRAY["系统托盘"]

    CB -- "500ms 轮询" --> CW
    CW -- "SHA-256 去重" --> DB
    CW -- "事件通知" --> FE
    FE -- "invoke" --> BE
    BE -- "查询 / 写入" --> DB
    TRAY -- "切换 / 设置" --> BE
    FE -- "select_clip" --> CB
```

## 项目结构

```
src/                          # 前端
├── index.html                # 主面板（列表 + 预览）
├── settings.html             # 设置窗口
├── js/
│   ├── api.ts                # 类型化 Tauri IPC 封装（唯一边界）
│   ├── ipc-types.ts          # Rust serde 数据合同
│   ├── app.js                # 入口 + 键盘路由
│   ├── clipboard-list.js     # 列表状态机 + 差量渲染
│   ├── preview-panel.js      # 预览状态与检测调度
│   ├── preview/              # 代码、元数据、格式、内容和加密渲染器
│   ├── search-bar.js         # 搜索 UI
│   └── settings.js           # 设置逻辑
├── styles/                   # CSS
└── i18n/                     # 中文、英文

src-tauri/src/                # Rust 后端
├── lib.rs                    # 应用初始化 + 插件注册
├── commands.rs               # AppState 与兼容 re-export
├── commands/                 # 剪贴板、设置、tmux、截图、OCR、URL 命令
├── clipboard_watcher.rs      # 剪贴板轮询线程
├── storage.rs                # SQLite/FTS5 核心与搜索
├── storage/                  # 清理维护、统计、URL 缓存和存储测试
├── config.rs                 # JSON 配置
├── models.rs                 # 数据模型
├── gsettings_shortcuts.rs    # Wayland 快捷键
├── tray_icon.rs              # 主题自适应托盘图标
├── paste/                    # X11、Wayland Portal、仅复制协调器
├── capture/                  # CaptureSession、多显示器覆盖层与选区动作
├── pin/                      # 统一 PinManager 与窗口生命周期
└── translation/              # 服务适配器、错误、request-id 和密钥环
```

## 开发

```bash
cargo tauri dev                              # 开发服务器（热重载）
cd src-tauri && cargo check --all-targets     # 编译检查
cd src-tauri && cargo test                   # 单元测试
cd src-tauri && cargo clippy --all-targets -- -D warnings  # Lint
cd src && npx vitest run                     # 前端测试
./scripts/ci-local.sh                        # 完整门禁 + DOM/Xvfb smoke
```

## 贡献

欢迎贡献！请先开 Issue 讨论你想做的改动。

1. Fork → 创建分支 (`git checkout -b feat/my-feature`)
2. 提交更改 (`git commit -m 'feat: add feature'`)
3. 推送 → Pull Request

## 致谢

- [Tauri](https://tauri.app/) — 更小、更快、更安全的桌面应用框架
- [arboard](https://github.com/1Password/arboard) — 跨平台剪贴板库
- [rusqlite](https://github.com/rusqlite/rusqlite) — Rust 的 SQLite 绑定
- [ashpd](https://github.com/bilelmoussaoui/ashpd) — XDG Portal 绑定
- [highlight.js](https://highlightjs.org/) — 语法高亮
- [marked](https://marked.js.org/) — Markdown 解析器
- [DOMPurify](https://github.com/cure53/DOMPurify) — HTML 净化器

## 许可证

[MIT](../LICENSE)
