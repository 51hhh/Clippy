# Changelog

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
