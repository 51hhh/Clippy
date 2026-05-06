# Changelog

## v0.1.14

### ✨ 新功能
- **tmux copy-pipe 即时捕获**：使用 `copy-pipe-and-cancel` 将 tmux copy-mode 复制内容直接管道写入文件，彻底解决"延迟一拍"问题（之前 `after-copy-mode` hook 中 `save-buffer` 获取的是上一次的 buffer）
- **inotify 事件驱动监听**：替换 500ms 文件轮询为 inotify `CLOSE_WRITE|MOVED_TO|CREATE` 事件驱动，零延迟捕获
- **copy-pipe 绑定自动验证**：每 ~60s 检查绑定完整性，丢失时自动重建
- **i18n 补全**：设置面板 tmux 捕获和统计面板的中英文翻译

### 🐛 修复
- `teardown_tmux_hook` 中 Enter 键恢复值从 `cancel` 改为 `copy-selection-and-cancel`（修复关闭 tmux 捕获后 Enter 键不复制的 bug）
- 设置面板 tmux/统计区域中文不显示（i18n.js 缺失 11 个翻译 key）

### 🏗 架构
- 新增 `start_tmux_watcher()` 独立线程（inotify + libc::poll 1s 超时）
- 主线程与 tmux 线程共享 `tmux_last_hash: Arc<Mutex<String>>` 防止重复捕获
- `setup_tmux_hook()` 绑定 vi/emacs copy-mode 的 y/Enter/MouseDragEnd1Pane
- `after-copy-mode` hook 保留为兜底（`sleep 0.1` + `save-buffer`）
- 新增依赖：`inotify = "0.11"`、`libc = "0.2"`

### 🧪 测试
- **303 测试全部通过**（9 测试文件）
- 新增 i18n.test.js：9 个测试覆盖翻译完整性、参数插值、DOM 应用

---

## v0.1.13

### ✨ 新功能
- **数字键直选**：按 1-9/0 直接选中前 10 条并粘贴，无需方向键导航
- **敏感内容自动检测**：识别 16 种 Token 前缀（OpenAI sk-、GitHub ghp_/gho_、AWS AKIA、JWT eyJ、Slack xox 等）+ password/secret 关键字模式，🔒 标记 + 5 分钟自动清理（收藏条目豁免）
- **URL 智能预览**：纯 URL 文本自动渲染 OG 元数据卡片（favicon、标题、描述、站名），SQLite 7 天缓存，ureq HTTP 5s 超时
- **JSON 格式化预览**：自动检测 JSON 内容并格式化展示
- **剪贴板统计面板**：显示总条目数、收藏数、文本/图片/文件比例
- **智能编码检测**：自动识别 URL 编码、HTML 实体、Unicode 转义、Base64、Hex 编码并提供解码按钮
- **加密内容检测**：识别哈希值（MD5/SHA/CRC32）和加密文本（OpenSSL/PGP/SSH/Age/JWE）
- **29 种内容自动检测**：颜色值、时间戳、UUID、IP 地址、邮箱、MAC 地址、Cron 表达式、日期字符串、语义版本号、进制数、CSS 渐变、数据大小、正则表达式、坐标、MIME 类型、数学表达式、HTTP 状态码
- **编解码面板**：左侧面板（` 键切换），21 种操作 — Base64/URL/HTML/Unicode/Hex/ROT13/MD5/SHA/JSON/JWT/URL解析/时间戳/进制转换，Smart Detection 自动推荐、方向交换、输入输出交换、最近使用追踪

### ⚡ 性能优化
- **内存优化 P0**：WebKit CacheModel::DocumentViewer、clear_cache on hide、禁用 GPU/WebGL/WebAudio/Media/PageCache/SmoothScrolling
- **内存优化 P1**：preview-panel 延迟加载 hljs/marked/DOMPurify（首次预览时初始化）
- **内存优化 P2**：缩略图缓存 FIFO 上限 50、窗口 blur 时释放预览内容
- **Cargo release profile**：strip=true, lto=true, codegen-units=1, opt-level="s", panic="abort"

### 🔒 安全
- **深度安全加固**：FTS5 注入防护、SSRF URL 白名单、XSS 转义
- **isMathExpr 沙箱**：Function() 求值限制为纯数字+运算符，禁止标识符

### 🏗 架构
- 新增 `is_sensitive` 字段（SQLite 迁移 + 向后兼容）
- 新增 `url_meta_cache` 表（URL/title/description/favicon/site_name/fetched_at）
- 新增 `purge_expired_sensitive()` 定时清理（watcher 每 ~30s 检查一次）
- 新增 `fetch_url_meta` IPC 命令（ureq v3 + regex-lite OG 解析）
- 新增 `set_codec_visible` IPC 命令 + 动态窗口宽度计算（380/780/1180px）
- preview-panel 29 种检测管线（1900+ 行），__test__ 导出 33 个函数
- codec.js 模块（430 行），__test__ 导出 8 个函数
- 新增依赖：`ureq = "3"`、`regex-lite = "0.1"`

### 🧪 测试
- **294 测试全部通过**（8 测试文件）
- preview-detection: 193 测试覆盖 29 种检测类型
- codec: 48 测试覆盖 21 种编解码操作 + MD5 向量验证

### 🐛 修复
- isReadable 国际化：检测字符范围扩展支持 CJK
- base64url 解码 UTF-8 正确处理
- IPv6 严格验证
- hexToBytes 边界检查
- OpenSSL PBKDF2 密钥派生修复

### 🎨 UI
- 敏感条目列表行淡化显示 + 🔒 前缀
- URL 预览卡片样式（favicon/标题/描述/站名/链接）
- 编解码面板左侧布局 + Smart Detection 提示条

## v0.1.12

### ✨ 新功能
- 设置页 OCR 管理：开关 toggle、tesseract 依赖状态实时检测（绿/红指示灯）、一键安装按钮
- OCR 结果模式自定义下拉框：主题适配配色、键盘导航（Arrow/Enter/Escape）
- pkexec 安装取消检测：用户取消授权不再显示"安装失败"提示

### 🐛 修复
- dev 模式下自启动开关不再被 disabled，改为 i18n 文字提示

### 🏗 架构
- OCR 从 leptess 编译时链接改为 tesseract CLI 子进程，解决跨 Ubuntu 版本 SONAME 不兼容
- deb 包 recommends 添加 tesseract-ocr、tesseract-ocr-chi-sim

### 📝 文档
- README/README.zh-CN 添加 OCR 可选安装说明和自编译提示

## v0.1.11

### 🏗 架构
- OCR: 移除 leptess 编译时链接，改用 tesseract CLI 子进程调用
- deb depends 清空，recommends 添加 tesseract-ocr

## v0.1.10

### 🐛 修复
- hljs 缓存无限增长：添加 HLJS_CACHE_MAX=200 上限，超限时清理最早一半条目
- getClipDetail 异步竞态：回调中校验 _currentClipId，防止旧请求覆盖新内容
- removeClip 清空列表时未通知预览面板：空列表触发 _onFocusChange(null)
- 清理已删除的删除确认 CSS 残留（.danger-confirm / .clip-row-action-confirm）

### 📝 文档
- 重写中英文 README：精简结构，添加 4 张截图展示

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
