# Clippy 当前架构

## 技术边界

- 主窗口：vanilla HTML/CSS/ES modules，保留稳定的剪贴板高频交互。
- Pin、截图覆盖层、图片编辑器：React + TypeScript 功能岛。
- 系统资源：Rust/Tauri 拥有剪贴板、数据库、窗口、截图帧、Portal 会话、Pin 数据、密钥和网络请求。
- IPC：`src/js/api.ts` 是唯一 Tauri 调用边界，`ipc-types.ts` 对齐 Rust serde 字段。

## 后端模块

| 模块 | 职责 |
|---|---|
| `lib.rs` / `app/` | `lib.rs` 组装 Tauri builder、managed state 和 Wayland gsettings/D-Bus；`app/` 管理开发自启防护、WebKit 诊断、托盘、X11 快捷键和窗口事件 |
| `commands/` | 按 clipboard/settings/tmux/capture/OCR/URL 拆分薄 IPC 命令 |
| `clipboard_watcher.rs` + `clipboard_watcher/*` | 主轮询与去重协调；内容分类、写入重试和 tmux/inotify 监听各自隔离 |
| `paste/mod.rs` | 自动粘贴协调器、后端选择、Copy-only fallback 和稳定状态契约 |
| `paste/portal.rs` / `x11.rs` / `token_store.rs` | Portal 会话与授权状态机、X11 窗口恢复与输入、私有 restore token 持久化 |
| `window_controller.rs` | 主窗口 work area、logical/physical 尺寸与位置约束 |
| `capture/` | 单一 CaptureSession、冻结帧、多显示器覆盖层、裁剪与动作 |
| `screenshot.rs` + `screenshot/*` | 原始截图帧契约与 PNG 编解码；Wayland/Portal/GNOME/xcap fallback 和几何测试隔离 |
| `pin/` | PinManager、内容来源、窗口尺寸、缩放/透明度/锁定和清理 |
| `translation/` | provider、超时/重试、request-id、内容选择、Secret Service；启用的服务按 `spawn_blocking` 并行，单服务失败作为数据返回；`direction.rs` 在文本已是目标语言时按备选语言换向；`tts.rs` 走 dictvoice 取回音频 |
| `storage.rs` + `storage/*` | SQLite/FTS5 初始化与搜索；维护清理、统计、URL 缓存、翻译记录和测试各自隔离 |

## 前端模块

| 模块 | 职责 |
|---|---|
| `js/clipboard-list.js` | 列表 facade、数据加载、IPC 动作与增量渲染装配 |
| `js/clipboard/` | 导航状态机、展示格式化和单行 DOM/缩略图渲染 |
| `js/preview-panel.js` | 预览状态、检测优先级、延迟库与缓存 |
| `js/preview/*-renderers.js` | 代码、元数据、格式、加密、内容/OCR 渲染 |
| `react/main/translationStore.ts` | 主预览翻译状态、多服务结果卡、单服务重试与陈旧响应保护 |
| `js/translation-providers.ts` | 服务显示名、默认端点与能力标记（设置页/主面板/选区翻译共用） |
| `react/capture-overlay/` | 窗口命中、选区移动/缩放、直接动作与选区翻译 |
| `react/capture/` | 对象标注、图像调整、撤销/重做和统一导出；视口、PNG 管线及待处理截图加载器独立管理 |
| `js/settings/` | 主题、自动粘贴授权、快捷键录制、OCR 和统计控制器 |
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

设置窗口关闭时，快捷键录制控制器先等待 `resume_shortcuts` 完成再关闭；Rust `AppState` 以原子标志和转换锁提供窗口销毁后的幂等恢复兜底。截图编辑器的待处理截图由最新请求代次门控，后端读取不消费缓存，窗口销毁或显式清理时统一释放。

`XDG_SESSION_TYPE` 优先于残留的 display 环境变量。Portal token 不进入普通配置；独立文件必须为 0600。首次 Portal 确认、撤权和桌面后端是否允许静默恢复仍属于真实桌面人工验收。
截图 Portal 的交互模式由截图用户动作显式开启；后台或未来自动任务应传入非交互模式，避免隐式弹出桌面授权。

## 安全规则

- 敏感条目在 Rust 内容选择阶段拒绝翻译；朗读条目文本走同一条内容选择路径，因此同样被拒绝。
- 朗读音频由 Rust 取回后以 data URL 播放，webview 不直接请求 dictvoice；文本长度上限 200 字符。
- 图片翻译只把本地 OCR 文本发送给 provider，不上传原图。
- API key 只进入系统 Secret Service，不提供明文 fallback。
- 成功的译文与原文落在同一个 SQLite 库（`translation_history`，全库上限 500 条）：条目删除、历史清空和上限清理都会一并删除它的译文，设置里另有"清空已保存的译文"入口。敏感条目从不进入翻译，因此也不会产生记录。
- 用户文本使用 React 文本节点或 `textContent`；富文本仅使用严格 DOMPurify 配置。
- URL 元数据仅访问无凭据的 HTTP(S)，拒绝私有/保留 IP、私有 DNS 解析和重定向；请求有 5 秒超时与 1 MiB 上限。
- 翻译响应有超时与 1 MiB 上限；数学表达式不使用 `eval`/`Function`。

## 质量门禁

`./scripts/ci-local.sh` 依次执行 Rust fmt/check/clippy/test、锁文件安装、TypeScript、Vitest、DOM/Xvfb smoke 和 Vite build。Linux 发布目标仅为 deb/AppImage；updater 签名由 release CI secret 生成。
