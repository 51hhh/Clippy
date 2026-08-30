# Changelog

## 未发布

### 🔄 变更

**截图改成"一个窗口走完全程"（参考 [flashot](https://github.com/poneding/flashot)，MIT）**
- 独立的截图编辑器窗口删除：选区、标注、图像调整、提交现在都发生在冻结画面覆盖层里。
  设置里的 "After Selecting"（`capture_commit_action`）随之删除——没有"转到编辑器"这个分支了。
- 默认就是选区模式：**点一下空地即整屏**，鼠标悬停在窗口上点一下取那个窗口，拖拽取自由区域。
- 点击或拖拽**不再直接结束**：完整工具条贴在选区旁边（16 个标注工具 + 颜色/线宽 + 撤销/重做 +
  图像调整 + 翻译/保存/贴图/对钩/取消），选区仍可拖动与八方向缩放。铺满全屏时内部拖拽回到重新框选。
- 点对钩直接把**裁剪 + 标注后的 PNG 复制进剪贴板**（`commit_capture_action`）：
  裁剪在前端画布上完成，后端不再按选区裁第二遍——那会丢掉画布上的标注。
  选区翻译仍走后端裁剪，OCR 要的是原始像素。

### 🐛 修复

- **"截屏是黑的"**：冻结帧本来就是正常的，黑的是覆盖层窗口自己——Wayland 不允许客户端摆放窗口，
  Tauri 的 `position()`/`set_size()` 被 GNOME 静默忽略。改为配置底层 GTK 窗口，
  由合成器 `fullscreen_on_monitor` 铺满按最大重叠面积选出的显示器。
- **截图里的画面缩放不对**：xcap 的 `Monitor::width()` 在 1920×1200/scale 1.3333 的桌面上返回 2880×1800，
  既不是逻辑尺寸也不是物理尺寸；改按 `round(帧像素 / scale_factor)` 归一化。
- **窗口速选框歪了**：`xcap::Window` 给的是 X screen 原始像素的**客户端**矩形，
  混了坐标空间（实测一个 QQ 窗口被报成 2598 像素宽）；现在先按 `X 像素 / 逻辑像素` 折算，
  再减掉 `_GTK_FRAME_EXTENTS`（CSD 阴影），小于 20 逻辑像素的候选丢弃。
- 顺手删掉随编辑器窗口一起失去入口的三个命令（`copy_screenshot_image`、`save_screenshot_image`、
  `save_screenshot_image_as`）：它们接受任意 base64 就写文件/剪贴板，留着是没人用的攻击面。

### 📄 文档

- 新增 [docs/capture-linux.md](docs/capture-linux.md)：**每个窗口的大小能不能拿到**的系统 API 调研结论
  （X11/xcb 能；GNOME Wayland 只能枚举 XWayland 客户端；`wlr-foreign-toplevel-management` 只有标题没有几何；
  `org.gnome.Shell.Screenshot` 对普通应用 `AccessDenied`），三套坐标空间的换算规则、
  覆盖层摆放为什么必须交给合成器、以及快速选区的交互约定。

## v0.1.17

自 v0.1.16 起共 119 个提交。这一版把 Clippy 从"剪贴板历史"扩成"剪贴板 + 截图 + 翻译"三条主线，
并按功能边界重写了后端与前端的模块划分。

### ✨ 新功能

**自动粘贴（一次授权）**
- X11：记录原活动窗口 → 隐藏 Clippy → 恢复并确认焦点 → 注入 `Ctrl+V`。
- Wayland：RemoteDesktop Portal 会话复用 `persist_mode=2` 与 0600 restore token，不用每次重新授权。
- 任一环节失败只回退到"内容已在剪贴板"，绝不盲注按键。

**截图工作流**
- 快捷键唤起多显示器冻结帧覆盖层：窗口命中速选、选区移动、八方向缩放。
- 选区提交后默认直接进图片编辑器（`capture_commit_action`，松手时按住 `Alt` 可临时留在工具条）；工具条支持 Copy / Save / Pin / Edit / OCR / 选区翻译。
- 编辑器窗口按截图尺寸开窗（物理像素按 `scale_factor` 折算 + chrome 占位，夹在最小尺寸与工作区之间），小选区可上采样到 3 倍，不再是大窗里的一小块。
- 图片编辑器 16 个标注工具，按选择 / 绘制 / 效果三组：裁剪、对象选择、橡皮、钢笔、荧光笔、矩形、椭圆、直线、箭头、测量、文字、高亮块、模糊、马赛克、聚光灯、放大镜；含图像调整与撤销/重做。
- 截图保存目录与文件名模板可配置（`{prefix} {date} {time} {unix} {seq}`），另存为走系统对话框，同名追加序号不覆盖。

**贴图（Pin）**
- 剪贴板条目、截图结果共用 `PinManager` 与 React 控件：首帧就绪后显示、缩放、透明度、锁定位置、复制、保存、转入编辑器，销毁时统一清理资源。

**翻译**
- 六个服务：LibreTranslate-compatible、OpenAI-compatible、DeepL、Google、Bing、有道；每个服务都有官方 API 与未配置凭据时的非官方 web 回退两条路径，端点一律过白名单校验。
- 启用的服务并行翻译（各自 `spawn_blocking`），单服务失败只影响自己那张结果卡并可单独重试。
- 文本本来就是目标语言时按备选语言换向，不再出现"英文翻成英文"；实际使用的目标语言随结果返回。
- 译文写入 SQLite（条目 + 服务 + 目标语言 + 原文哈希，全库上限 500 条），重选条目时回填并标记 "Saved earlier"；设置页有"清空已保存的译文"入口。
- 朗读原文与译文（dictvoice）：音频由 Rust 取回后以 data URL 播放，webview 不直接请求第三方主机。
- 图片翻译只把本地 OCR 文本发出去，不上传原图。

**编解码侧栏（`` ` `` 打开）**
- 22 个操作：Base64、URL、HTML 实体、Unicode、Hex、JWT 解析、URL 解析、时间戳与日期互转、进制转换、MD5、ROT13 等。
- 显式收藏（星星两态图标，写 `localStorage`），常用操作由用户自己定，不用 MRU 冲掉。
- 一次产出多个类别的操作（时间戳的 Local/UTC/ISO、进制四种写法、URL 各段、JWT 的 Header/Payload）渲染成键值对按钮：点键只复制键，点值只复制值，整段复制仍走工具条。

**预览与内容识别**
- 内容类型判定收口到 `js/preview/classify.js` 的有序规则表，结果只显示在预览面板 badge 上；列表行不再显示类型，不会再出现"主栏 HTML、侧栏 YAML"的自相矛盾。
- 覆盖 URL 卡片、JSON、JWT、可逆编码、哈希、加密内容、颜色、时间戳、UUID、IP、邮箱、MAC、cron、日期、semver、进制、渐变、数据大小、正则、坐标、MIME、数学表达式、HTTP 状态码，判不出来再走 Markdown / 代码高亮 / 富文本 / 纯文本。

**其它**
- 托盘菜单与原生窗口标题按界面语言本地化（`src-tauri/src/i18n.rs`，解析规则与前端一致）。
- 主窗口记忆显示位置；快捷键可在设置页录制，注册失败按动作给出可操作提示，并能检测桌面级占用与 Clippy 内部自冲突。

### 🐛 修复

- **hex 摘要被标成 BASE64**：`atob`/hex 解出的是 Latin-1 字节串，旧的可读性判断把 `0xA0-0xFF` 全算成正常字符，随机字节过得去阈值，于是 MD5 被 `encoding` 规则抢走并显示成乱码。改为先严格校验 UTF-8（`TextDecoder({ fatal: true })`）再判可打印比例；hex 分支按长度排除摘要的黑名单一并删掉，正好 16/20/32/64 字节的 hex 编码文本不再被误判成摘要。顺带修掉 UTF-8 内容被按 Latin-1 显示成乱码。
- **GNOME 自定义快捷键被覆盖**：条目路径不再写死 `custom0/1/2`（先到先得，很可能是用户自己的快捷键），改为按 command 认领并原地复用，认不出来才取未占用编号。
- **一个键位被占用连坐另外两个**：X11 改为逐个动作 `register`，不再用全有或全无的 `register_multiple`；注册、保存和录制结束恢复三条路径都按动作记账，全部失败才把状态退回"已暂停"。
- **codec 收藏把 `localStorage` 的值拼进 CSS 选择器**：带引号的脏值让 `querySelector` 抛错，而 `codec.init()` 排在列表初始化之前，抛一次异常整个主窗口就初始化不完。改为遍历比对，并剔掉已不存在的操作后回写自愈。
- **一次 `Esc` 同时收下拉和关侧栏**，输入框内容跟着一起没了：内层控件消费掉的键必须 `stopPropagation()`。
- **`Shift+Tab` 在翻译面板聚焦不上时是死胡同**（按钮 disabled 或面板 render 成 null 时照样 `preventDefault`）：改成聚焦成功才拦，否则把键还给浏览器。
- **删除与插入缺事务**：`insert_clip` / `delete_clip` / `clear_history` / `delete_entries` 的 FTS 清理、主表删除与译文删除各自独立执行，中途失败会留下"搜得到但已不存在"的 FTS 幽灵行或删不掉的译文（后者违反"删条目会一并删掉译文"的隐私承诺）。四处统一用事务包住。
- **开预览会改变列表可见行数**：撤销"预览打开时窗口加高"，主窗口高度对所有面板组合恒定 500，翻译区靠 `max-height` 与自身滚动落位。
- **翻译区遮挡预览**：`#translation-react-root` 补 `.translation-host`（flex 列 + `min-height: 0` + `max-height`），百分比 `max-height` 才有确定的包含块。
- **主窗口打开原生下拉像是崩了**：WebKitGTK 的原生 `<select>` 弹窗是独立 GTK 窗口，一打开 webview 就失焦、无边框窗口随即隐藏。主窗口最后一个 `<select>` 换成 `custom-select`，并锁定 `<select>` 数量为 0。
- **侧栏打开后字母键仍在驱动列表**：键盘归属改为按焦点位置单点解析（`codec > search > translation > list`）；方向键与 `ws` 在列表模式下始终归列表，进翻译区必须显式 `Shift+Tab`；预览或 codec 打开时豁免失焦隐藏。
- **快速切换 codec 操作时旧结果覆盖新结果**：执行加代次门控。
- **快捷键注册失败被静默吞掉**：失败记账在后端，`get_shortcut_failures` 可随时读取，启动期早于设置页监听的失败也能显示。
- **翻译历史回填过于频繁**：加防抖与面板可见性门控，列表连按上下键只查停下的那条。
- **Portal restore token 被误删**：引入授权阶段状态机，只有 token 确实被 Portal 消费过才在失败后删除，否则下次又要重新弹授权。
- **翻译错误不可行动**：4xx 响应限读 4 KiB 正文用于归类，把"缺少/无效 key"从不透明的 `http_status` 里还原出来；5xx 正文一律不读，网关错误页不会被误判成凭据问题。
- **AppImage 签名步骤在 release runner 上必挂**：`finalize-appimage.sh` 写死 `cargo tauri signer sign`，而 runner 上只有 tauri-action 自带的 CLI 和 `src/` 锁定的 npm CLI，没有 cargo-tauri，重封装成功后紧接着 `no such command: tauri`。改为优先用仓库锁定的 npm CLI、其次 cargo-tauri，并在签名后校验 `.sig` 真的产出（CLI 有 exit 0 但不出签名的路径，那会让 updater 静默拿不到签名）。
- **构建钩子依赖固定 cwd**：`beforeDevCommand` / `beforeBuildCommand` 写死 `cd ../src`，用 `npx tauri` 从仓库根启动时直接 `can't cd to ../src`。改成两种 cwd 都成立。
- 截图动作失败后释放会话、按代次清理待编辑缓存、动作完成后恢复源窗口；Pin 丢弃乱序 IPC 响应；收藏面板差量行节点重排；陈旧列表请求不再覆盖新状态；显式文本复制语义统一；设置窗口关闭时恢复全局快捷键；AppImage 图标链接可移植。

### 🏗 架构

- 前端 IPC 收口到严格类型化的 `src/js/api.ts` + `ipc-types.ts`，结构测试守卫其它模块不得直接碰 Tauri；主列表与翻译面板迁到 React/TS 功能岛（`src/react/main/`），Pin 与截图编辑器同样是隔离功能岛。
- Rust IPC 命令按剪贴板、设置、tmux、截图、OCR、URL 元数据、编辑器拆分；存储维护/统计/URL 缓存、自动粘贴的 Portal/X11/token、截图入口与平台 fallback、剪贴板 watcher 的主轮询/内容分类/写入重试/tmux 监听各自独立。
- 错误全面类型化：`PasteError` / `PinError` / `CaptureError` 与既有 `StorageError` / `TranslationError` 一起提供稳定 `code()`，日志标识统一为 `domain.code`，对前端返回的文案逐字不变。
- 日志区分真实故障与预期路径：Wayland 首次未授权、翻译请求被新请求取代、快捷键连按撞上进行中的截图会话降为 info。
- 统一的阻塞 HTTP 层：超时、禁重定向、1 MiB 上限、单次重试与错误归一只有一处实现。
- 服务显示名与默认端点抽到 `src/js/translation-providers.ts`，设置页 / 主面板 / 选区翻译共用。
- 配置 v1 → v2 迁移：单个 `translation_provider/endpoint/model` 改成 `translation_services` 列表，迁移后立刻回写；端点与当年默认值相同则清空，不把旧默认地址永久钉住。

### 🔒 安全

- 翻译 API key 只写系统 Secret Service，没有明文回退；双字段服务（有道）用 `{provider}` 与 `{provider}.secret` 两条记录，只填一半在设置页就拒绝。凭据结构不实现 `Debug`/`Serialize`，不会进日志或跨 IPC。
- 敏感条目在 Rust 内容选择阶段拒绝翻译，朗读走同一条路径因此同样被拒绝。
- 本地数据目录 `0700`，配置、数据库、WAL/SHM 与 Portal restore token `0600`，启动时修复旧文件的宽松权限；restore token 不进普通配置。
- 用户文本只经 React 文本节点或 `textContent` 进 DOM，富文本只走严格 DOMPurify；数学预览改用受限递归下降解析器，HTML 实体解码不再动态执行。
- URL 元数据只访问无凭据 HTTP(S)，拒绝私有/保留 IP 与私有 DNS 解析、关闭重定向，5 秒超时与 1 MiB 上限。
- Portal 交互式截图只由用户动作开启；授权失败会关闭会话；GNOME Shell 临时截图使用私有权限并在错误路径清理。
- 依赖：DOMPurify 3.4.13，Vite/Vitest/jsdom 升到无已知漏洞版本，移除未使用的 sharp。

### ⚡ 性能

- criterion 基线覆盖截图编解码、剪贴板扫描与搜索（`src-tauri/benches/`，经 `bench_support.rs` 调用生产代码，不复制实现）；门禁只编译不运行，数字见 `docs/bench-baseline.md`。
- 截图内存占用与拖拽卡顿优化；`[profile.bench]` 关掉 release 的 fat LTO。

### 🧪 测试与门禁

- Rust 237 项测试；前端 Vitest 35 files / 605 tests；`./scripts/ci-local.sh` 依次跑 fmt / check / clippy / test、锁文件安装、TypeScript、Vitest、DOM/Xvfb smoke、Canvas 导出像素 smoke、主窗口布局像素 smoke 和 Vite build（11 通过 / 0 失败 / 1 跳过，跳过项是需显式开启的 AppImage 可视 smoke）。
- 两个真实浏览器像素 smoke：Canvas 导出与主窗口布局几何（jsdom 没有布局引擎，量不出遮挡）。
- 结构守卫把"改错了不会报错、只会悄悄退化"的问题钉住：构建钩子的 cwd 无关性、主窗口 `<select>` 为 0、codec 操作必须挂 `data-i18n`、列表行不写类型标签、源码里写死的 i18n key 必须存在于两个 locale、release notes 的下载后缀必须等于构建矩阵的 label（否则发布页挂死链）。
- `@tauri-apps/cli` 进 `src/` 的 devDependencies 并锁进 lockfile，`cargo tauri` 与 `npx tauri` 同版本，构建不再依赖 npx 缓存。

### ⚠️ 升级说明

- **不再提供 Ubuntu 22.04 构建**：截图用的 `xcap 0.9` 必然拉入 `pipewire`/`libspa 0.9`，而 `libspa`
  无条件访问 `spa_video_info_raw.flags`（pipewire ≥ 0.3.65 才有该字段），jammy 仓库只有 0.3.48，
  bindgen 生成的结构体缺字段，编译直接失败。用第三方 PPA 换新头文件能骗过编译，但产出的二进制
  与 22.04 实机上的 libpipewire ABI 不一致，比明确不支持更危险。22.04 用户请留在 v0.1.16 或自行编译。
  发布产物只剩 `ubuntu24` 后缀（以及 updater 用的无后缀件）。
- 配置会自动从 v1 迁移到 v2（翻译服务列表）并回写，无需手工编辑；旧的单服务配置保留原有端点与模型。
- 翻译需要自己配置服务凭据；未配置凭据时部分服务走非官方 web 端点，设置页对这些服务标注"随时可能失效"。
- OCR 需单独安装 tesseract：`sudo apt install tesseract-ocr tesseract-ocr-chi-sim`。
- Wayland 下自动粘贴首次仍需确认一次 Portal 授权；桌面是否允许静默恢复取决于桌面环境。

## v0.1.16

### ✨ 新功能
- **搜索体验优化**：短输入（<3字符）走 LIKE 子串匹配，长输入走 FTS5 prefix + LIKE 合并，覆盖 `text_content` 和 `ocr_text`
- **FTS 索引自动修复**：初始化时执行一次 FTS rebuild，修复旧库索引缺失
- **select_clip 置顶**：点击列表条目时自动更新 `created_at` 并移动到首位

### 🐛 修复
- **剪贴板搜索单字母无结果**：旧实现用 FTS5 phrase query 精确匹配，短输入无法命中；改为 LIKE 模糊搜索
- **重复复制不置顶**：watcher 的 `last_hash` 在 skip 检查前更新，导致 select_clip 后外部重复制被跳过；移动 `last_hash` 更新到 skip 检查之后
- **select_clip 不移动到首位**：`select_clip` 仅写剪贴板不更新 `created_at`，添加 `touch_clip()` + `clip-added` 事件通知前端
- **前端重复条目**：`prependClip` 未检查已有条目，重复内容会叠加；添加 `findIndex` + `splice` 去重逻辑

### 🧪 测试
- **Rust 22 测试 + 前端 306 测试全部通过**
- 新增 8 个后端搜索测试（单字母、前缀、中文、OCR、特殊字符、收藏过滤、FTS 修复、touch_clip）

## v0.1.15

### 🐛 修复
- **AppImage Wayland 兼容性修复**：移除 linuxdeploy-plugin-gtk 强制注入的 `GDK_BACKEND=x11`，恢复 Wayland 下页面渲染和托盘图标功能
- **Wayland 检测逻辑统一**：`is_wayland()` 同时检查 `WAYLAND_DISPLAY` 和 `XDG_SESSION_TYPE`，确保 XWayland 混合环境正确识别

### 🏗 架构
- Linux-only 依赖（webkit2gtk、zbus、enigo、inotify）移至 `[target.'cfg(target_os = "linux")'.dependencies]` 平台守卫
- `enableGTKAppId: true` 确保 Wayland 下 GTK app_id 正确设置

### 🧪 测试
- **Rust 15 测试 + 前端 303 测试全部通过**

---

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
