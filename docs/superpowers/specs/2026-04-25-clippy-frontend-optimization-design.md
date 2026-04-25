# Clippy 前端优化设计 — 窗口行为 + 设置页面 + i18n

日期: 2026-04-25

## 概述

修复 Clippy 前端的三类核心问题：窗口行为（浮动面板自动隐藏、任务栏隐藏、关闭≠退出）、设置页面（快捷键录制、主题同步、布局）、i18n（中英双语）。

## 模块 A：窗口行为修复

### A1. 任务栏隐藏

`tauri.conf.json` 主窗口配置添加 `"skipTaskbar": true`。主窗口作为浮动面板不应出现在任务栏/dock 中。

### A2. 关闭≠退出

在 `lib.rs` 的 `tauri::Builder` 链上添加 `.on_window_event()`：

- **main 窗口**：`CloseRequested` → `api.prevent_close()` + `window.hide()`
- **settings 窗口**：`CloseRequested` → `api.prevent_close()` + `window.hide()`
- 只有托盘菜单的 "Quit" 才真正退出应用（当前 `app_handle.exit(0)` 已实现）

### A3. 点击外部自动隐藏（主窗口）

监听 `WindowEvent::Focused(false)` 事件（仅 main 窗口）：

1. 启动 200ms 延迟定时器
2. 定时器到期后检查窗口是否仍处于 unfocused 状态
3. 若仍 unfocused，调用 `window.hide()`

延迟是为了避免误触——例如用户点击操作菜单（右键菜单）导致的瞬间失焦。

CopyQ 使用 500ms 延迟，我们用 200ms 因为 Tauri webview 内部的上下文菜单不会触发窗口级 Focused 事件。

### A4. 粘贴后行为

保持当前行为：`select_clip` IPC 命令中已经调用 `window.hide()`，选中条目后立即隐藏窗口。

## 模块 B：设置页面优化

### B1. Rust 端改动

#### 配置变更事件

`commands.rs` 中 `update_config` 命令执行后，通过 `app_handle.emit("config-changed", &new_config)` 广播事件。主窗口监听此事件以实时更新主题。

#### 快捷键录制支持

新增两个 IPC 命令：

- `pause_shortcuts` — 调用 `app.global_shortcut().unregister_all()` 暂停全局快捷键
- `resume_shortcuts` — 重新注册当前配置的快捷键

录制流程：前端点击 Record → 调用 `pause_shortcuts` → keydown 捕获 → 完成/取消后调用 `resume_shortcuts`。

#### ShortcutState 过滤

当前 `lib.rs:67` 和 `commands.rs:130` 的快捷键回调同时响应 Pressed 和 Released 事件。在回调中添加 `if event.state != ShortcutState::Pressed { return; }` 过滤。

注意：需要检查 `tauri-plugin-global-shortcut` v2 的 `on_shortcut` 回调签名。回调参数是 `(app, shortcut, event)` 其中 event 包含 `.state`。如果回调签名中没有直接提供 state，则需要检查实际 API，可能是 `event.state()` 或类似方法。

### B2. 前端改动

#### 设置页面布局

- settings 窗口保持有边框的正常窗口（当前 `WebviewWindowBuilder` 未设置 `decorations(false)` 所以已经是有边框的）
- 设置窗口 `resizable: false`，固定 500×400
- 布局保持当前结构但优化间距和分组

#### 快捷键录制流程

```
点击 Record → invoke("pause_shortcuts")
            → 监听 keydown 捕获组合键
            → 成功捕获或取消 → invoke("resume_shortcuts")
```

#### 主题实时同步

`app.js` 新增监听 `"config-changed"` 事件，收到后调用 `theme.applyTheme(payload.theme)`。

#### 表单默认值

设置页面加载时从后端读取配置并填充表单（当前已实现，保持不变）。

### B3. update_config 需要 app_handle

当前 `update_config` 命令签名中没有 `app_handle` 参数。需要添加 `app_handle: tauri::AppHandle` 参数以便调用 `emit`。

## 模块 C：i18n 系统

### C1. 架构

```
src/
├── i18n/
│   ├── i18n.js       — 核心翻译模块（~50 行）
│   ├── en.json       — 英文翻译文件
│   └── zh-CN.json    — 中文翻译文件
```

### C2. 翻译键设计

JSON 翻译文件按页面和功能分组：

```json
{
  "search.placeholder": "Search clipboard history...",
  "tabs.all": "All",
  "tabs.favorites": "Favorites",
  "empty.icon": "📋",
  "empty.text": "No clipboard items yet",
  "action.favorite": "Favorite",
  "action.unfavorite": "Unfavorite",
  "action.copy": "Copy",
  "action.delete": "Delete",
  "settings.title": "Settings",
  "settings.shortcut.label": "Global Shortcut",
  "settings.shortcut.placeholder": "Click to record...",
  "settings.shortcut.record": "Record",
  "settings.shortcut.stop": "Stop",
  "settings.shortcut.reset": "Reset",
  "settings.shortcut.hint": "Click \"Record\" then press your desired key combination",
  "settings.shortcut.conflict": "This shortcut may conflict with an existing shortcut",
  "settings.theme.label": "Theme",
  "settings.history.label": "History Limit",
  "settings.history.hint": "0 = unlimited. Favorites are never deleted.",
  "settings.language.label": "Language",
  "settings.save": "Save",
  "settings.cancel": "Cancel",
  "settings.saved": "Settings saved!",
  "settings.saveFailed": "Save failed: {error}",
  "time.justNow": "just now",
  "time.minutesAgo": "{n} min ago",
  "time.hoursAgo": "{n} hr ago",
  "time.yesterday": "yesterday",
  "time.daysAgo": "{n} days ago"
}
```

### C3. HTML 绑定

HTML 元素使用 `data-i18n` 属性标记翻译键：

```html
<span data-i18n="tabs.all">All</span>
<input data-i18n="search.placeholder" data-i18n-attr="placeholder" placeholder="Search...">
```

`i18n.js` 模块提供：
- `init()` — 检测语言、加载翻译文件、扫描 DOM 并替换
- `t(key, params?)` — 程序式翻译（用于 JS 中动态生成的文本）
- `applyToDOM()` — 重新扫描并替换所有 `[data-i18n]` 元素

### C4. 语言检测与配置

1. 优先使用 `AppConfig.language`（用户手动选择）
2. 若为空或 `"auto"`，使用 `navigator.language` 检测
3. 回退到 `"en"`

### C5. AppConfig 扩展

`models.rs` 的 `AppConfig` 新增字段：
```rust
pub language: String,  // "auto" | "en" | "zh-CN"
```

默认值 `"auto"`。设置页面新增语言选择下拉框。

## 文件变更清单

### Rust 端
- `src-tauri/tauri.conf.json` — 添加 `skipTaskbar`
- `src-tauri/src/lib.rs` — 添加 `on_window_event`（CloseRequested + Focused）、ShortcutState 过滤
- `src-tauri/src/commands.rs` — `update_config` 添加 emit、新增 `pause_shortcuts` / `resume_shortcuts`
- `src-tauri/src/models.rs` — `AppConfig` 新增 `language` 字段

### 前端
- `src/i18n/i18n.js` — 新建，翻译核心模块
- `src/i18n/en.json` — 新建，英文翻译
- `src/i18n/zh-CN.json` — 新建，中文翻译
- `src/index.html` — 添加 `data-i18n` 属性
- `src/settings.html` — 添加 `data-i18n` 属性、语言选择控件
- `src/js/app.js` — 添加 i18n init、config-changed 事件监听
- `src/js/settings.js` — 快捷键录制 pause/resume、i18n
- `src/js/clipboard-list.js` — 动态文本使用 `t()` 翻译
- `src/js/theme.js` — 无变更（已正确实现）
- `src/js/search.js` — 无变更

## 不做的事情

- 不更改默认快捷键（保持 Super+V）
- 不添加开机自启功能
- 不修改 SQLite 存储逻辑
- 不添加新的内容类型支持
