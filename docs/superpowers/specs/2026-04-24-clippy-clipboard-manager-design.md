# Clippy — 跨平台轻量剪贴板管理器设计文档（MVP）

## 概述

Clippy 是一款基于 Tauri v2 + Rust 的跨平台剪贴板管理器，支持 Windows / macOS / Linux。核心体验：全局快捷键呼出轻量悬浮面板，快速搜索和选择历史剪贴板内容。

**MVP 范围**：剪贴板监听、SQLite 存储（含 FTS5 全文搜索）、悬浮面板、搜索、全局快捷键、多主题。
**后续迭代**：系统托盘、开机自启、设置面板。

## 技术栈

- **后端**：Rust（Tauri v2 框架）
- **前端**：HTML + CSS + vanilla JS（无框架，极轻量）
- **存储**：SQLite + FTS5 全文搜索（via rusqlite）
- **构建/打包**：Tauri CLI（输出 deb/appimage/nsis）

## 架构

```
┌──────────────────────────────────────────────────┐
│                    Clippy                          │
│                                                    │
│  ┌────────────────────┐    ┌───────────────────┐  │
│  │   Rust Backend      │    │  Frontend (UI)    │  │
│  │                     │    │                   │  │
│  │  ClipboardWatcher ──┼────▶  悬浮面板          │  │
│  │       │             │    │  - 搜索框         │  │
│  │  StorageEngine      │    │  - 历史列表       │  │
│  │  (SQLite + FTS5)    │    │  - 收藏切换       │  │
│  │       │             │    │  - 内容预览       │  │
│  │  ConfigManager      │    └────────▲──────────┘  │
│  └────────────────────┘         Tauri IPC          │
│                                                    │
│  系统集成：全局快捷键                                │
└──────────────────────────────────────────────────┘
```

## 项目结构

```
clippy/
├── src/                              # 前端（纯 HTML/CSS/JS，与后端可分离）
│   ├── index.html
│   ├── styles/
│   │   ├── base.css                  # 重置 + 基础样式
│   │   ├── themes.css                # CSS 变量定义（多主题色系）
│   │   └── components.css            # 组件样式
│   ├── js/
│   │   ├── app.js                    # 入口，初始化 + 事件绑定
│   │   ├── api.js                    # 封装所有 Tauri IPC 调用（唯一与后端耦合点）
│   │   ├── clipboard-list.js         # 列表渲染 + 无限滚动
│   │   ├── search.js                 # 搜索框逻辑 + debounce
│   │   └── theme.js                  # 主题切换逻辑
│   └── assets/                       # 图标等静态资源
├── src-tauri/
│   ├── src/
│   │   ├── main.rs                   # 入口，调用 lib::run()
│   │   ├── lib.rs                    # Tauri Builder 注册插件和命令
│   │   ├── clipboard_watcher.rs      # 剪贴板监听线程
│   │   ├── storage.rs                # SQLite + FTS5 存储引擎
│   │   ├── config.rs                 # JSON 配置读写
│   │   ├── commands.rs               # Tauri IPC 命令定义
│   │   └── models.rs                 # 数据结构（ClipItem 等）
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── capabilities/default.json     # Tauri v2 权限配置
│   └── icons/
├── docs/
└── CLAUDE.md
```

### 前后端分离设计

- `src/js/api.js` 是唯一调用 `window.__TAURI__` 的文件，其他 JS 模块通过它间接与后端通信
- 前端可用浏览器直接打开 index.html 做 UI 开发（api.js 提供 mock 降级）
- CSS 主题通过 CSS 自定义属性（变量）实现，切换主题只需更改 `<html data-theme="xxx">` 属性

## 模块设计

### 1. ClipboardWatcher（剪贴板监听）

- 独立线程运行，定时轮询系统剪贴板（间隔 ~500ms）
- 使用 `arboard` crate 读取剪贴板内容
- 通过 SHA-256 内容哈希去重，避免重复记录
- 支持的内容类型：
  - **纯文本**：直接存储字符串
  - **富文本/HTML**：存储 HTML 原文 + 纯文本降级版本
  - **图片**：存储为 PNG 格式的二进制数据
- 检测到新内容后写入 StorageEngine，并通过 Tauri 事件通知前端刷新

### 2. StorageEngine（存储引擎）

#### 数据库 Schema

```sql
CREATE TABLE clips (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    content_type TEXT NOT NULL,  -- 'text' | 'html' | 'image'
    text_content TEXT,           -- 纯文本内容（text/html 类型）
    html_content TEXT,           -- HTML 原文（html 类型）
    image_data   BLOB,           -- 图片二进制数据（image 类型）
    content_hash TEXT NOT NULL UNIQUE,  -- SHA-256 哈希，用于去重
    is_favorite  INTEGER DEFAULT 0,     -- 是否收藏
    created_at   INTEGER NOT NULL,      -- Unix 时间戳
    byte_size    INTEGER NOT NULL       -- 内容大小（字节）
);

CREATE VIRTUAL TABLE clips_fts USING fts5(
    text_content,
    content='clips',
    content_rowid='id'
);

CREATE INDEX idx_clips_created_at ON clips(created_at DESC);
CREATE INDEX idx_clips_favorite ON clips(is_favorite, created_at DESC);
```

#### 核心操作

- **插入**：写入 clips 表 + 同步更新 FTS 索引；若超出历史上限，删除最早的非收藏条目
- **搜索**：通过 FTS5 的 `MATCH` 语法实现全文搜索，支持中文（unicode61 tokenizer）
- **收藏**：切换 `is_favorite` 字段；收藏条目不受历史上限清理影响
- **删除**：支持单条删除和清空全部历史
- **历史上限清理**：插入时检查非收藏条目数量，超出上限则删除最旧的

#### 存储模式

- **持久化模式（默认）**：SQLite 数据库文件存放在 Tauri 的 app data 目录
- **内存模式**：使用 SQLite `:memory:`，应用退出后数据清空

### 3. ConfigManager（配置管理）

配置文件使用 JSON 格式，存放在 Tauri app config 目录：

```json
{
    "max_history": 100,
    "storage_mode": "persistent",
    "global_shortcut": "CmdOrCtrl+Shift+V",
    "theme": "light"
}
```

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| max_history | int | 100 | 历史上限，0 = 无限制 |
| storage_mode | string | "persistent" | "persistent" 或 "memory" |
| global_shortcut | string | "CmdOrCtrl+Shift+V" | 全局快捷键 |
| theme | string | "light" | "light" / "dark" / "ocean" / "forest" |

### 4. Frontend（前端 UI）

#### 主题系统

通过 CSS 自定义属性定义多套颜色系列主题：

```css
[data-theme="light"]   { --bg-primary: #ffffff; --text-primary: #1a1a1a; --accent: #4a90d9; ... }
[data-theme="dark"]    { --bg-primary: #1e1e1e; --text-primary: #e0e0e0; --accent: #5b9fe6; ... }
[data-theme="ocean"]   { --bg-primary: #0d1b2a; --text-primary: #e0e1dd; --accent: #48cae4; ... }
[data-theme="forest"]  { --bg-primary: #1b2e1b; --text-primary: #d8e2dc; --accent: #52b788; ... }
```

切换主题只需设置 `document.documentElement.dataset.theme = "dark"`。

#### 悬浮面板

- **尺寸**：宽 380px，高 500px
- **位置**：屏幕居中弹出
- **行为**：CmdOrCtrl+Shift+V 呼出/隐藏，失焦自动隐藏，选择条目后自动隐藏并写入剪贴板

#### 面板布局

```
┌──────────────────────────────┐
│  🔍 搜索框                    │
├──────────────────────────────┤
│  [All] [Favorites]           │
├──────────────────────────────┤
│  ┌────────────────────────┐  │
│  │ 📋 This is some text...│⋯│  │
│  │    2 min ago            │  │
│  ├────────────────────────┤  │
│  │ 🖼️ [image thumbnail]   │⋯│  │
│  │    15 min ago       ⭐  │  │
│  ├────────────────────────┤  │
│  │ 📄 <html>rich text...  │⋯│  │
│  │    1 hour ago           │  │
│  └────────────────────────┘  │
│       ↕ 滚动加载更多          │
└──────────────────────────────┘
```

右侧的 `⋯` 为多功能按钮。

#### 多功能按钮交互

取代传统右键菜单，每条剪贴板条目右侧显示多功能按钮（⋯）：

1. **默认状态**：显示条目内容预览 + 多功能按钮图标
2. **激活状态**：点击 ⋯ 按钮（或键盘触发）后：
   - 条目内容区域虚化（backdrop blur）
   - 上层覆盖显示操作选盘菜单
   - 操作项：⭐ Favorite / 🗑 Delete / 📋 Copy / 📌 Paste / 📝 Paste as plain text
3. **支持鼠标和纯键盘操作**（具体键盘映射在实现阶段细化）
4. 点击操作项或按 Escape 退出激活状态

#### 列表交互

- **搜索**：输入即搜（debounce 200ms），调用后端 FTS 搜索，高亮匹配关键词
- **选择条目**：点击条目内容区域 → 写入系统剪贴板 → 隐藏面板
- **键盘导航**：↑/↓ 浏览列表，Enter 选中，Escape 关闭面板
- **列表切换**：点击 [All] / [Favorites] 切换视图
- **滚动加载**：每次加载 20 条，滚动到底部自动加载更多

#### 条目显示

- **文本**：显示前 2 行预览，超出截断并加省略号
- **HTML**：显示纯文本降级版本的预览
- **图片**：显示 64x64 缩略图预览
- 每条显示相对时间（"2 min ago"、"yesterday" 等，英文 UI）

### 5. 系统集成（MVP）

#### 全局快捷键

- 使用 `tauri-plugin-global-shortcut`
- 默认快捷键：`CmdOrCtrl+Shift+V`
- 按下快捷键：面板可见则隐藏，面板隐藏则显示

## Tauri IPC 命令

后端暴露给前端的 Tauri 命令：

```rust
#[tauri::command] fn get_clips(query: Option<String>, favorites_only: bool, offset: u32, limit: u32) -> Vec<ClipItem>
#[tauri::command] fn delete_clip(id: i64) -> Result<()>
#[tauri::command] fn toggle_favorite(id: i64) -> Result<bool>
#[tauri::command] fn clear_history() -> Result<()>
#[tauri::command] fn select_clip(id: i64) -> Result<()>  // 写入剪贴板并隐藏面板
#[tauri::command] fn get_config() -> AppConfig
#[tauri::command] fn update_config(config: AppConfig) -> Result<()>
```

后端推送给前端的事件：

```rust
app.emit("clip-added", &new_clip);      // 新条目添加
app.emit("clip-removed", &clip_id);     // 条目被删除（上限清理）
```

## 关键依赖（Cargo.toml）

| crate | 用途 |
|-------|------|
| tauri (v2) | 应用框架 |
| rusqlite (features: bundled) | SQLite 绑定 |
| arboard | 跨平台剪贴板访问 |
| serde + serde_json | 序列化 |
| sha2 | 内容哈希去重 |
| tauri-plugin-global-shortcut | 全局快捷键 |

## 平台特殊考量

| 平台 | 注意事项 |
|------|----------|
| Linux | 需处理 X11/Wayland 两种剪贴板协议；arboard 已封装，但 Wayland 下全局快捷键可能受限 |
| macOS | 需要辅助功能权限才能监听剪贴板；签名和公证流程 |
| Windows | 无特殊限制；注意 UWP 应用的剪贴板隔离 |

## 非目标（MVP 不做）

- 系统托盘（后续迭代）
- 开机自启（后续迭代）
- 设置面板 UI（后续迭代，MVP 通过配置文件修改）
- 多设备同步
- 云存储
- 剪贴板内容编辑
- 脚本/自动化处理
- 多语言国际化（v1 仅英文 UI）
