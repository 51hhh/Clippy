# Clippy 当前架构

## 技术边界

- 主窗口：vanilla HTML/CSS/ES modules，保留稳定的剪贴板高频交互。
- Pin、截图覆盖层、图片编辑器：React + TypeScript 功能岛。
- 系统资源：Rust/Tauri 拥有剪贴板、数据库、窗口、截图帧、Portal 会话、Pin 数据、密钥和网络请求。
- IPC：`src/js/api.ts` 是唯一 Tauri 调用边界，`ipc-types.ts` 对齐 Rust serde 字段。

## 后端模块

| 模块 | 职责 |
|---|---|
| `commands/` | 按 clipboard/settings/tmux/capture/OCR/URL 拆分薄 IPC 命令 |
| `paste/` | X11 活动窗口恢复、Wayland RemoteDesktop Portal、Copy-only fallback |
| `window_controller.rs` | 主窗口 work area、logical/physical 尺寸与位置约束 |
| `capture/` | 单一 CaptureSession、冻结帧、多显示器覆盖层、裁剪与动作 |
| `pin/` | PinManager、内容来源、窗口尺寸、缩放/透明度/锁定和清理 |
| `translation/` | provider、超时/重试、request-id、内容选择、Secret Service |
| `storage.rs` + `storage/*` | SQLite/FTS5 初始化与搜索；维护清理、统计、URL 缓存和测试各自隔离 |

## 前端模块

| 模块 | 职责 |
|---|---|
| `js/clipboard-list.js` | 列表状态、键盘动作、增量渲染 |
| `js/preview-panel.js` | 预览状态、检测优先级、延迟库与缓存 |
| `js/preview/*-renderers.js` | 代码、元数据、格式、加密、内容/OCR 渲染 |
| `js/translation-panel.js` | 主预览翻译状态与陈旧响应保护 |
| `react/capture-overlay/` | 窗口命中、选区移动/缩放、直接动作与选区翻译 |
| `react/capture/` | 对象标注、图像调整、撤销/重做和统一导出 |
| `react/pin/` | 首帧就绪、工具栏、拖动阈值和 rAF 更新合并 |

## 核心流程

```text
clipboard item -> preview -> translate/copy

shortcut -> frozen monitor frames -> selection
         -> copy/save/pin
         -> local OCR -> text translation
         -> editor -> copy/save/pin

clip/image/capture -> PinManager -> hidden window -> first frame ready
                   -> scale/opacity/lock/copy/save/edit -> destroy cleanup
```

## 自动粘贴状态

```text
X11     : capture _NET_ACTIVE_WINDOW -> hide Clippy -> restore/confirm -> Ctrl+V
Wayland : select keyboard + persist_mode=2 -> rolling restore token -> reused session
Fallback: permission/backend/injection failure -> clipboard remains populated, no key injection
```

`XDG_SESSION_TYPE` 优先于残留的 display 环境变量。Portal token 不进入普通配置；独立文件必须为 0600。首次 Portal 确认、撤权和桌面后端是否允许静默恢复仍属于真实桌面人工验收。

## 安全规则

- 敏感条目在 Rust 内容选择阶段拒绝翻译。
- 图片翻译只把本地 OCR 文本发送给 provider，不上传原图。
- API key 只进入系统 Secret Service，不提供明文 fallback。
- 用户文本使用 React 文本节点或 `textContent`；富文本仅使用严格 DOMPurify 配置。
- URL 元数据仅访问无凭据的 HTTP(S)，拒绝私有/保留 IP、私有 DNS 解析和重定向；请求有 5 秒超时与 1 MiB 上限。
- 翻译响应有超时与 1 MiB 上限；数学表达式不使用 `eval`/`Function`。

## 质量门禁

`./scripts/ci-local.sh` 依次执行 Rust fmt/check/clippy/test、锁文件安装、TypeScript、Vitest、DOM/Xvfb smoke 和 Vite build。Linux 发布目标仅为 deb/AppImage；updater 签名由 release CI secret 生成。
