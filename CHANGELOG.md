# Changelog

## v0.1.9

### ✨ 新功能
- 收藏列表独立数据源：收藏和全部列表使用独立数组，避免误删和性能问题
- 国际化：预览面板硬编码字符串接入 i18n（富文本/图片加载失败）

### ⚡ 性能优化
- cleanup_old_entries 消除 N+1 查询：批量 SELECT + 批量 FTS 删除
- render() 差量更新：ID 序列不变时复用 DOM 行，只更新焦点/按钮状态
- 图片缩略图内存缓存（_thumbCache Map）
- hljs 语言检测结果缓存（_hljsCache Map，按 content_hash）

### 🎨 UI 改进
- 行内容竖直居中（align-items: center）
- 省略图标（⋯）选中时不再收回，操作按钮覆盖显示
- 收藏模式操作按钮移至左侧，←/A 展开，与全部模式对称

## v0.1.8

### ✨ 新功能
- 富文本预览面板：支持 HTML 富文本、Markdown 渲染、代码语法高亮（21 种语言）
- 智能内容检测：评分制 Markdown 识别 + hljs 代码检测（阈值 5）
- HTML 剪贴板支持：完整的 HTML 复制/粘贴管道
- 预览面板按需加载 html_content（`get_clip_detail` IPC），列表不再传输大体积 HTML
- 6 套主题适配的语法高亮配色（light/dark/nord/solarized-light/rose/midnight）

### 🔒 安全
- CSP 收紧：移除 img-src/media-src 中的 `https: http:`
- 预览面板拦截所有 `<a>` 链接点击，防止 webview 导航
- DOMPurify 白名单净化 HTML，40+ 允许标签，禁止 script/iframe/form
- renderCode 和 marked renderer 输出均经 DOMPurify 处理
- inline style 清理扩展：移除 position/z-index/opacity

### 🐛 修复
- 鼠标悬浮切换行时同步更新预览面板
- restoreRender 后同步更新预览面板
- HTML 条目无纯文本时自动 strip tags 作为 FTS 搜索回退
- HTML 行预览显示 `[富文本]` 而非原始标签
- select_clip HTML 分支 alt_text 为空时回退空串

### 🔧 技术变更
- 新增 `get_clip_detail` IPC 命令（按需加载含 html_content 的完整条目）
- get_clips SQL 不再返回 html_content，减少列表加载数据量
- 窗口尺寸提取为常量（WINDOW_WIDTH_DEFAULT/PREVIEW/HEIGHT）
- clipboard_watcher 新增 `strip_html_tags` 辅助函数

## v0.1.6

### ✨ 新功能
- 新增开机自启动功能，设置页添加 toggle 开关，切换即时生效
- deb 安装包更新检测：自动识别安装类型，deb 用户显示手动下载引导而非自动更新

### 🐛 修复
- 修复 deb 安装后 GNOME 桌面出现双图标的问题（通过 postinst 脚本创建 .desktop 符号链接）

### 🔧 技术变更
- 集成 tauri-plugin-autostart v2，支持 Linux 系统自启动
- 新增 deb 包 postinst/postrm 脚本，处理 GTK App ID 与 .desktop 文件名映射
- 新增 `get_install_type` IPC 命令，通过 APPIMAGE 环境变量判断安装类型

## v0.1.5

### 🐛 修复
- 更新弹窗 changelog 区域改为自适应高度，内容多时自动伸展可滚动
- Release 构建时传入 changelog 到 tauri-action，修复 updater latest.json notes 为空的问题

### 📄 文档
- 新增 CI/CD 文档（docs/CI.md），记录完整构建发布流程和发版检查清单

## v0.1.4

### 🎨 UI 图标全面升级
- 全套 Unicode/emoji 图标替换为 Lucide 风格 SVG 矢量图标
- 新增 `icons.js` 集中管理图标模块（search/clipboard/copy/star/starFill/trash/more）
- 收藏行星标从 CSS `content: "★"` 改为 SVG mask 方案，跟随主题色
- i18n action 标签移除 Unicode 图标前缀，图标与文字彻底分离

### 🐛 体验修复
- 操作按钮组改为绝对定位覆盖模式，不再推挤文本导致换行
- Esc 键分级处理：先收起操作按钮组 → 再关闭搜索框 → 最后关闭面板
- 搜索框打开时 Esc 不再直接关闭面板，而是先走三段式退出逻辑

### 🔧 技术变更
- i18n JSON 文件补齐 `empty.favorites`、`action.confirm`、`search.escHint` 三个缺失键
- CSS 按钮/触发器移除 `font-size`（SVG 图标不需要字体大小控制）
- 前端测试模板同步更新为 SVG 图标

## v0.1.3

### ✨ 新功能
- 选中条目自动粘贴：选中后写入剪贴板并通过 XTest 模拟 Ctrl+V 粘贴到前一活动窗口
- ESC 一键退出：面板内任何状态下按 ESC 立即隐藏面板

### 🐛 体验优化
- 面板打开时默认选中第一行（最新条目），无需额外操作
- 键盘导航期间鼠标悬浮不再抢夺焦点（需实际移动鼠标才激活行高亮）

### 🔧 技术变更
- 新增 `enigo` 依赖（x11rb 后端），用于 XTest 按键模拟
- ESC 隐藏面板改用 `@tauri-apps/api/window` 替代已废弃的 `window.__TAURI__`
- `releaseMemory()` / `restoreRender()` 重置焦点状态，确保面板每次打开焦点一致

## v0.1.1

### ⚡ 性能优化（运行时缓存）
- SQLite 启用 WAL 日志模式，提升并发读写性能
- 查询语句改用 `prepare_cached`，避免重复编译 SQL
- 剪贴板插入采用 UPSERT（ON CONFLICT）替代 try/catch 两步写入
- 历史清理改为批量 DELETE（IN 子句），减少逐条删除开销
- Linux 下设置 `MALLOC_ARENA_MAX=2`，降低 glibc 内存碎片
- 前端列表分页加载（PAGE_SIZE=30 + 滚动追加），替代一次性加载 200 条
- 行移动 / 列切换 / 展开收起 / 鼠标悬停改为 CSS class 差量更新，不再全量 render
- 新增 / 删除条目差量 DOM 操作（prepend / remove + idx 重编号）
- 窗口失焦时释放列表内存，聚焦时按需恢复（dirty flag 机制）

## v0.1.0

### ✨ 新功能
- 剪贴板实时监听（500ms 轮询，SHA-256 去重）
- SQLite 持久化存储 + FTS5 全文搜索
- 悬浮剪贴板面板（无边框置顶窗口，键盘导航）
- 系统托盘菜单（Open Clipboard / Settings / Quit）
- 全局快捷键动态注册（X11: tauri-plugin-global-shortcut; Wayland: gsettings + D-Bus）
- 收藏功能（收藏条目不受历史清理影响）
- 设置面板：快捷键录制、6 主题切换、历史上限、语言选择
- 主题化托盘图标（SVG 实时渲染，跟随主题色）
- 国际化支持（英文 / 中文 / 跟随系统）
- 召唤式搜索条 + segment tabs（全部/收藏）
