# Clippy 当前架构

## 技术边界

- 主窗口：vanilla HTML/CSS/ES modules，保留稳定的剪贴板高频交互。
- Pin、截图覆盖层（含标注）：React + TypeScript 功能岛。
- 系统资源：Rust/Tauri 拥有剪贴板、数据库、窗口、截图帧、Portal 会话、Pin 数据、密钥和网络请求。
- IPC：`src/js/api.ts` 是唯一 Tauri 调用边界，`ipc-types.ts` 对齐 Rust serde 字段。

## 后端模块

| 模块 | 职责 |
|---|---|
| `lib.rs` / `app/` | `lib.rs` 组装 Tauri builder、managed state 和 Wayland gsettings/D-Bus；`app/` 管理开发自启防护、WebKit 诊断、托盘、X11 快捷键和窗口事件；托盘菜单文案取自 `i18n.rs`，`config-changed` 同时刷新图标（主题）与菜单文案（语言） |
| `commands/` | 按 clipboard/settings/tmux/capture/OCR/URL 拆分薄 IPC 命令 |
| `clipboard_watcher.rs` + `clipboard_watcher/*` | 主轮询与去重协调；内容分类、写入重试和 tmux/inotify 监听各自隔离。**入库只有 watcher 这一条路径**——`writer.rs` 的三个写入口只管写系统剪贴板，写完敲 `wake::nudge()` 让轮询等待当场结束（`wake.rs` 的条件变量 + 待处理标记），所以自己复制的内容也是几毫秒内进历史，而不是最多等满 500 ms。别让写入方自己 `insert_clip`：watcher 哈希的是它自己从剪贴板 RGBA 重编的 PNG，字节对不上就会在 500 ms 后被再插一条 |
| `paste/mod.rs` | 自动粘贴协调器、后端选择、Copy-only fallback 和稳定状态契约 |
| `paste/portal.rs` / `x11.rs` / `token_store.rs` | Portal 会话与授权状态机、X11 窗口恢复与输入、私有 restore token 持久化 |
| `window_controller.rs` | 主窗口 work area、logical/physical 尺寸与位置约束；设置窗口的开窗入口（托盘与 IPC 共用一份几何与标题） |
| `i18n.rs` | 托盘菜单与 Rust 侧窗口标题的静态文案；语言解析与前端 `i18n.js` 同规则（显式值优先，`auto` 看 `LC_ALL`/`LC_MESSAGES`/`LANG`，其余回退英文） |
| `capture/` | 单一 CaptureSession、冻结帧、多显示器覆盖层、裁剪与动作；`window_probe.rs` 按堆叠顺序（扩展 `sort_windows_by_stacking` / X11 `_NET_CLIENT_LIST_STACKING`）下发速选候选。冻结帧像素由 `get_capture_frame` 以二进制 IPC 直传原始 RGBA（payload 只带几何），`StageTimings` 每次会话记一条分段耗时日志 |
| `capture/shell_extension.rs` | 自带 GNOME Shell 扩展的安装/卸载/状态、令牌校验与截图调用（整屏 `Screenshot` 与逐屏 `ScreenshotArea`，`include_str!` 内嵌 `gnome-extension/`）。GNOME Wayland 下既没有面向普通应用的窗口几何接口，Portal 截图也要求"聚焦的应用"才能弹授权对话框（快捷键触发时必然失败），两件事都只能以扩展身份进 gnome-shell 做；装了要注销一次生效，升级同样要（`ReloadExtension` 已废弃，故有 `stale` 状态），卸载即时。安装只由设置页显式触发，启动时只做 `reconcile_on_startup`，绝不擅自安装。`place_window` 是贴图缩放的每帧热路径，它的前置检查（是不是 GNOME Wayland、装没装、协议版本够不够、令牌文件内容）缓存在 `placement_token()` 里：成功的结果一直有效，失败的结果 30 秒后重试，安装/卸载会立刻作废缓存。详见 [docs/capture-linux.md](capture-linux.md) |
| `screenshot.rs` + `screenshot/*` | 原始截图帧契约与 PNG 编解码；Wayland 上按 Shell 扩展逐屏原生（`ScreenshotArea`，协议 v4）→ 同一扩展的整屏舞台图 → wlroots → Portal（非交互）→ GNOME → xcap 依次回退，返回的临时文件一律由 `TemporaryScreenshotFile` 兜底删除；几何测试隔离。取一次全屏冻结帧的时间绝大部分是 gnome-shell 自己编码 PNG（单屏约 550 ms、双屏整屏舞台图约 1050 ms），能动的只有"少拍像素 + 并行拍"，逐屏那条路正是为此（[capture-linux.md](capture-linux.md) §3.1、§3.3）。整屏舞台图在混合缩放的多屏上会把低缩放那块上采样，画面发糊，所以逐屏是首选、整屏只是兜底 |
| `pin/` | PinManager、内容来源、窗口尺寸、缩放/透明度/锁定和清理；`origins.rs` 的 `PinOriginRegistry`（挂在 `AppState.pin_origins`）记住"我们自己截下来复制走的图"原本在屏幕上的矩形，之后从历史里 Pin 同一张图时靠它贴回原处。键是**解码后像素**的 sha256（含宽高），不是 PNG 字节——图片经 arboard 走一圈是原始 RGBA，watcher 会重新编码，PNG 字节不稳定。登记方交出的是 `PinFingerprint`（已经算好的摘要 + 宽高），这样调用方能和剪贴板写入共用同一份解码像素，不必为登记再解一次或复制一份 16 MB；`lookup` 先只读 PNG 头比宽高，尺寸不匹配就直接返回，省掉整张解码 |
| `pin/` 的 `update_pin` 应答 | 缩放/不透明度是**每帧**都会走的路，所以应答是 `PinState`（label、内容尺寸、scale、opacity、locked、position），**不带 `image_base64`/`text`**——每帧重编一张全图 base64 纯属浪费，而且会让前端重建图片 object URL、造成闪烁。前端 `react/pin/update-order.ts::mergePinState` 把它合并进手里那份 payload |
| `translation/` | provider、超时/重试、request-id、内容选择、Secret Service；启用的服务按 `spawn_blocking` 并行，单服务失败作为数据返回；`direction.rs` 在文本已是目标语言时按备选语言换向；`tts.rs` 走 dictvoice 取回音频 |
| `storage.rs` + `storage/*` | SQLite/FTS5 初始化与搜索；维护清理、统计、URL 缓存、翻译记录和测试各自隔离 |
| `dbus.rs` | **全部阻塞式 D-Bus 调用的唯一入口。** ashpd 打开了 zbus 的 `tokio` feature，于是 `zbus::blocking` 内部用一个静态多线程 runtime 做 `block_on`，在 tokio async worker 线程上调必然 panic（`Cannot start a runtime from within a runtime`）。这里先跳到一条干净的 OS 线程再开连接，调用方不必关心自己跑在什么线程上；别处不要直接用 `zbus::blocking`。**session 连接是复用的**：`Connection::session()` 每次都要重做 SASL 握手 + `Hello`（1~2 ms），而贴图缩放每一帧都要发一次 `PlaceWindow`，所以连接缓存在一个 `OnceLock<Mutex<Option<Connection>>>` 里。失效判据是 `worth_reconnecting`——`Error::MethodError` 说明对端应答了（连接是好的，绝不能重连重试），其余错误才丢缓存重连一次。**存入缓存的判据（`worth_caching`）必须是同一个的反面**：只按"调用成功"判会把一条被业务错误拒绝、但本身完好的连接扔掉，于是每次失败的探测都要重新握手一遍 |
| `image_io.rs` / `dialogs.rs` | PNG 与剪贴板互转；按配置的目录与文件名模板落盘（`SaveTarget`）；`thumbnail_png` 给列表行缩图（`image` crate 的 `thumbnail()` 快路径，不是 Lanczos）；选目录对话框只在 `dialogs.rs` 调用插件 |

## 前端模块

| 模块 | 职责 |
|---|---|
| `js/clipboard-list.js` | 列表 facade、数据加载、IPC 动作与增量渲染装配 |
| `js/clipboard/` | 导航状态机、展示格式化和单行 DOM/缩略图渲染 |
| `js/preview/classify.js` | **内容类型的唯一判定处**：有序规则表（guard 预筛 → detect → 渲染器），先匹配先赢 |
| `js/preview-panel.js` | 预览状态、异步判定尾段（Markdown/代码/富文本/纯文本）、延迟库与缓存 |
| `js/preview/*-renderers.js` | 代码、元数据、格式、加密、内容/OCR 渲染；badge 文案由命中的渲染器自己写 |
| `react/main/translationStore.ts` | 主预览翻译状态、多服务结果卡、单服务重试与陈旧响应保护 |
| `styles/components.css::.translation-host` | React 翻译面板的挂载壳：`.preview-panel` 的 flex 子项必须自己有 `min-height: 0` 与 `max-height`，否则 `.translation-panel` 的百分比 `max-height` 相对 auto 高度失效，翻译区会把预览内容挤没、自己被窗口裁掉。主窗口高度对所有面板组合恒定（`MAIN_WINDOW_HEIGHT`），翻译区靠这个上限和自身滚动落位——高度随预览变化会连带改变列表可见行数，列表跟着重排比翻译区挤一点更难用 |
| `js/translation-providers.ts` | 服务显示名、默认端点与能力标记（设置页/主面板/选区翻译共用） |
| `react/capture-overlay/` | 冻结画面上的全部截图交互：窗口速选、选区移动/缩放、贴着选区的完整工具条、标注、提交与选区翻译（只有这一个窗口） |
| `react/annotation/` | 与窗口无关的标注核心：16 个工具（选择/绘制/效果三组）、图像调整、撤销/重做、单画布渲染与 PNG 导出 |
| `js/settings/` | 主题、自动粘贴授权、快捷键录制与注册失败提示、OCR、统计、分页（`tabs.js`）与窗口速选服务卡片（`window-probe.js`）控制器 |
| `react/pin/` | 首帧就绪、工具栏、拖动阈值和 rAF 更新合并；`gestures.ts` 是"不可缩放、不可划选"的纯规则（滚轮缩放、Shift+滚轮调不透明度、ctrl/meta+滚轮一律忽略），由 `App.tsx` 里 `{ passive: false }` 的原生监听器执行——React 的 `onWheel` 是被动监听器，在里面 `preventDefault()` 拦不住 WebKit 的页面缩放 |

## 内容类型只有一套标准

类型判定的**唯一**入口是 `js/preview/classify.js` 的有序规则表，结果只显示在预览面板的
`#preview-type-badge` 上。**列表行不显示类型**：后端 `content_type` 只有 text/html/image
三档（按剪贴板 flavor 定），而预览是按内容嗅探（YAML/JWT/TIMESTAMP…），两边同时显示必然
自相矛盾——同一条 HTML 片段会一边被标成 HTML、一边被标成 YAML。因此 `clipboard/formatters.js`
故意不提供类型格式化函数，列表行的 meta 只剩"大小 · 时间"。

表里 `guard` 只做廉价的长度/前缀预筛，`detect` 才承担语义，返回值原样透传给渲染器（哈希类型、
编码结果不重算）。顺序本身就是语义：JWT 必须早于可逆编码，否则三段 Base64 会被当成普通 Base64。
badge 文案跟着渲染器走而不是写在表里——文案和渲染方式本来就是一回事，分开写必然再次分叉。
表判不出来的留给 `preview-panel.js` 的异步尾段：Markdown、`hljs.highlightAuto`（relevance > 5
且排除 `xml`）、`html_content` 富文本、纯文本，它们要么依赖延迟加载的库要么要再拉一次 IPC。

哈希与可逆编码的边界靠"解码结果是否是合法 UTF-8"划，不靠顺序也不靠长度
（`detectors.js::decodeReadableBytes`）。纯 hex 的摘要同时也是合法 Base64 字符集，而
`atob`/hex 解出来的是"一字节一字符"的 Latin-1 串——按 Latin-1 判可读性时 0xA0-0xFF 全算
正常字符，随机字节里通常有七八成落在这个区间，比例阈值拦不住，MD5 于是被标成 BASE64 并显示成乱码。
反过来把 `hash` 提到 `encoding` 前面，或者像以前那样在 hex 分支里按长度黑名单排除
32/40/64/128，又会把正好 16/20/32/64 字节的 hex 编码文本误判成摘要。真正编码过的文本几乎
一定是合法 UTF-8，摘要的随机字节几乎一定不是，因此严格 UTF-8 解码同时解决两个方向；
顺带修掉了 UTF-8 内容被按 Latin-1 显示成乱码的老问题（图片魔数仍按原始字节判）。

## 主窗口键盘状态机

键盘归属按**焦点位置**单点解析（`js/keyboard-router.js::resolveKeyboardMode`），先匹配先赢：
`codec > search > translation > list`。不按"面板是否可见"判定，因为键盘操作下焦点不会自己跑出侧栏，
"只有 `` ` `` 能切回列表"才成立；而鼠标点回中间列表能立刻把键盘交还列表。

| 模式 | 谁拥有键盘 | 被路由拦下的键 | 其余按键 |
|---|---|---|---|
| `codec` | 焦点在 `#codec-panel` 内（`` ` `` 打开左侧栏） | `` ` ``、`Esc` 关面板并把焦点还给列表 | 全部交给面板自己（字母/数字打进输入框，不驱动列表） |
| `codec` 内层 | 自定义下拉展开中 | `Esc` 只收起下拉并 `stopPropagation()`，不外泄给上表 | 收起后再按一次 `Esc` 才关侧栏 |
| `search` | 搜索框聚焦 | `Esc` 逐级退出（清空 → 收起 → 隐藏窗口） | 交给 input，`` ` `` 能正常打出反引号 |
| `translation` | 焦点在 `#translation-react-root` 内（`Shift+Tab` 显式送入） | `Ctrl+Enter` 翻译；`Esc`/`Tab` 只把焦点交回列表（预览留着）；`` ` `` 切侧栏 | 放行原生滚动与按钮语义 |
| `list` | 默认（预览开着也一样） | `w/a/s/d`、方向键、`1-9/0`、`Enter`、`Space`、`Ctrl+P`、`Ctrl+Enter`、`Tab`/`Esc` | 带修饰键的其它组合一律放行 |

方向键与 `ws` 在 `list` 模式下**始终**驱动列表：Tab 打开预览不再把焦点塞进翻译面板，
焦点要进翻译区必须显式 `Shift+Tab`。焦点撤离翻译面板时先 `blur()` 再 focus `#list-panel`，
不留"谁也不拥有"的中间态。

**焦点归属跟着条目走，不跟着索引走。** `focusedRow` 只有两种含义：`-1` 是"没有焦点行"，
`0..N-1` 指向 `visibleItems()` 里那一条。所以列表内容被释放（面板关闭、`releaseMemory`）时
必须报 `-1` 而不是 `0`——空列表上的 0 是个不存在的行；而 `prependClip` 收到新条目时按**id**
找回原来那一行，不做 `focusedRow ± 1`。两条规则缺一个的后果都是同一个用户可见症状：
面板关着时到达的新条目把焦点挤到第二行，而 Pin 的两个入口（全局快捷键、`Ctrl+P`）都读
`getFocusedClip()`、列表行上又没有 Pin 按钮，于是"截图复制完去 Pin，贴出来的是上一张图"。
`js/clipboard-list.js` 与 `react/main/clipboardStore.ts` 各有一份 `prependClip`，前者目前运行时
不生效，但两份必须同时改（`regression-guards.test.js` 钉住）。

**但"跟着条目走"只对握着焦点的面板成立。** 全局快捷键不会抢焦点，所以它触发时
`document.hasFocus()` 为假就说明"焦点行"是上一轮会话留下的残影，不是用户正看着的行——
`pin-target.js::resolvePinTarget` 因此把面板是否有焦点作为第三个参数，为假时一律问后端要
最新一条。这一步不能省：侧栏（预览/编解码）开着时失焦**不隐藏**主窗口
（`window_events.rs::should_hide_on_focus_loss` 与 `app.js::onWindowBlur`），
于是 `releaseMemory()` 不会跑、列表连焦点一起活过整个截图流程，`prependClip` 按 id
把焦点跟着老条目挪到第 1 行，按 Pin 贴出来的就是上一张。
同理 `onWindowFocus` 的**两条分支**都要 `restoreRender()`——重新聚焦算一轮新会话，
而 `refresh()` 里的 `normalizeAfterRefresh` 只做钳位、不复位。

**列表行只取缩略图。** 库里存的是原图，行里那格是 48 CSS px，所以两个列表渲染器都走
`get_clip_thumbnail`（后端缩到最长边 128 px 并按 id 缓存），只有预览面板取
`get_clip_image` 原图。退回原图后功能照样对，只是每开一次面板就把十几 MB 送进 webview
并做十几次全尺寸 PNG 解码，全落在 webview 那一个线程上。

"内层先吃掉、吃掉就不外泄"是这套状态机的通用规则：嵌套控件消费了某个键就必须
`stopPropagation()`，否则一次 `Esc` 会同时收下拉 + 关侧栏，输入框里的内容跟着一起没了。
反过来，路由拦下一个键之前也要确认动作真的发生了（`Shift+Tab` 只在焦点确实进了翻译面板时才
`preventDefault`；翻译进行中按钮 disabled、无选中条目时面板 render 成 null，这时把键还给浏览器）。

codec 侧栏的操作可以显式收藏（`codec-favorites` 分组，localStorage `clippy-codec-favorites`），
星星按钮用 `js/icons.js` 的描边/实心两个图标切换状态，与列表行的收藏按钮同一套图标。
收藏是跨版本的外部输入：`init()` 会剔掉下拉里已不存在的操作并回写自愈，且不把值拼进 CSS
选择器（带引号的脏值会让 `querySelector` 抛错，`codec.init()` 中断则整个主窗口初始化不完）。
面板里的操作名、分组标题、按钮提示和输入框占位符全部挂 `data-i18n`（专有名词如 Base64/JWT/SHA 不翻译），
下拉触发按钮的文案与收藏分组是 JS 写入的，`applyToDOM` 碰不到，因此语言切换后由 `codec.refreshLabels()` 补齐。

一次操作产出多个不同种类的值时（时间戳的 Local/UTC/ISO、进制的四种写法、URL 各段、JWT 的
Header/Payload），`_runOp` 返回 `{ fields: [{ label, value, group? }] }` 而不是拼好的多行文本，
输出区渲染成一行一对按钮：点键只复制键，点值只复制值，整段复制仍走工具条上的 📋。
所以 `#codec-output` 是 `<div>` 而不是 `<pre>`（`<pre>` 只容纳短语内容，装不了按钮行），
单值结果靠 CSS 的 `white-space: pre-wrap` 保持原样。"复制全部"和 ⇅ 读模块内的 `_outputText`
而不是 `textContent`——多字段 DOM 里键值之间没有分隔符，拼出来的文本不可用。
字段键名是文案，换语言后由 `refreshLabels()` 重算一遍（JWT 的 Header/Payload、UTC/ISO 8601
等规范里的专有字段名不翻译）。

失焦自动隐藏与这套状态机配套：`app/window_events.rs::should_hide_on_focus_loss(preview, codec)`
在预览或 codec 侧栏打开时豁免隐藏（纯函数 + 单测）。主窗口里也不允许出现原生 `<select>`——
WebKitGTK 的原生下拉是独立 GTK 弹窗，一打开 webview 就失焦，看着像窗口崩掉；
所有下拉都用 `js/custom-select.js`（支持分组标题与动态选项 `refresh()`），
`entrypoints-smoke.test.js` 锁定 `index.html` 里 `<select>` 数量为 0。

## 标注工具

分组即覆盖层工具条上排的分组（`capture-overlay/tools.tsx::TOOL_GROUPS`，成员由
`annotation-tools.test.js` 锁定；工具 id 与标注核心的 `TOOL_DRAFTS ∪ MANUAL_TOOLS` 对齐，
差别只有 `crop → select`）。

| 分组 | 工具 | 说明 |
|---|---|---|
| 选择 | select、object、eraser | select 就是框选/移动/缩放选区（选区自己是裁剪框，因此没有独立的 crop 工具）；选中/拖动已有标注；橡皮一次点击删一个标注（保持撤销粒度） |
| 绘制 | pen、marker、rect、ellipse、line、arrow、measure、text | 四种拖拽形态（折线/矩形/线段/文本）复用同一套包围盒、命中与移动逻辑；marker 半透明且笔宽更粗，ellipse 只在轮廓附近命中，measure 标注原图像素长度 |
| 效果 | highlight、blur、mosaic、spotlight、magnifier | blur/mosaic/spotlight/magnifier 需要读取或压暗底图，因此始终先于矢量标注绘制，magnifier 从原图重采样使预览与导出清晰度一致；highlight 只是半透明矢量色块，按用途归在这一组，绘制顺序仍跟随矢量标注 |

## 核心流程

```text
clipboard item -> preview -> translate/copy

shortcut -> frozen monitor frames -> overlay window (hidden) -> geometry payload + raw RGBA frame
         -> click empty space = whole screen / click a window = that window / drag = free area
         -> toolbar beside the selection: annotate, adjust, still re-frame
         -> check mark -> canvas PNG (crop + annotations) -> commit_capture_action -> copy/save/pin
         -> translate -> backend crop -> local OCR -> text translation

clip/image/capture -> PinManager -> hidden window -> first frame ready
                   -> show + focus + PlaceWindow(original rect, above) or Tauri fallback
                   -> scale/opacity/lock/copy/save -> destroy cleanup
```

贴图窗口落在**原始矩形**上而不是屏幕中间：截图选区的桌面逻辑坐标随 `commit_capture_action`
一路传到 `PinOrigin`，窗口位置是 `origin − SHADOW_GUTTER`（内容区相对窗口原点偏移 12 像素），
尺寸按 `origin_content_size` 只缩不放地钳进工作区。没有来源信息的图（从别处复制来的）
仍居中，这是设计而不是退化。GNOME Wayland 上 `set_position` 与 `set_always_on_top` 都是静默
空操作，摆位和置顶只能借道 Shell 扩展的 `PlaceWindow`（协议 v3 起提供），因此顺序必须是
`show()` → `set_focus()` → `PlaceWindow`（`MetaWindow` 要窗口映射后才存在），缩放之后还要再摆一次
（改尺寸会把窗口带回普通层）；尺寸始终留在客户端。两条路都失败只是位置/层级不理想，
**绝不让贴图本身失败**。细节见 [capture-linux.md](capture-linux.md) §1、§4。

截图只有覆盖层一个窗口，不再有独立的编辑器窗口。三态由选区推导，不额外存状态：
没有选区是 idle（悬停高亮可速选），按住不放是 dragging（框选/移动/缩放选区，或画一笔标注），
有选区是 editing（工具条贴在选区旁，选区仍可拖动与缩放）。点对钩时前端用
`react/annotation/pngPipeline.ts::exportPngBase64` 把"裁剪 + 图像调整 + 矢量标注"渲染成一张 PNG，
再交给 `commit_capture_action` 落地——**后端不再按选区裁第二遍**，否则画布上的标注会被丢掉。
只有选区翻译仍走后端裁剪：OCR 要的是原始像素而不是带标注的画布。

铺满全屏的选区没有"移动"余量，所以 `geometry.ts::coversBounds` 让它内部的拖拽回到重新框选
（否则点一下取了整屏之后就再也框不出小区域）；缩放手柄不受当前工具影响，标注工具激活时仍可改框。
工具条落点由 `toolbarPlacement` 计算：优先贴选区下方，放不下翻到上方，都放不下压在视口底部。
窗口速选依赖 `xcap::Window::all()`（本质是 X11/xcb 枚举），枚举失败或返回空时后端记 `log::info`、
覆盖层显示 "Window picking unavailable in this session"，让 Wayland 下的退化可见。
覆盖层窗口本身必须由合成器摆放，坐标与窗口几何的换算细节见 [capture-linux.md](capture-linux.md)。
底图不走 JSON：`get_capture_frame` 用二进制 IPC 直传原始 RGBA，前端 `putImageData` 铺进离屏 canvas
（`react/annotation/frameImage.ts`），因此标注渲染层的底图类型是
`FrameImage = HTMLImageElement | HTMLCanvasElement`。像素改回 PNG + base64 会让覆盖层出现的时间翻倍。

## 自动粘贴状态

```text
X11     : capture _NET_ACTIVE_WINDOW -> hide Clippy -> restore/confirm -> Ctrl+V
Wayland : select keyboard + persist_mode=2 -> rolling restore token -> reused session
Fallback: permission/backend/injection failure -> clipboard remains populated, no key injection
```

快捷键注册失败（Wayland 桌面不受管、X11 组合被占用）由后端记账，`get_shortcut_failures` 可随时读取，因此启动阶段早于设置页监听的失败也能显示；设置页对同一动作只保留最新一条，保存成功后重新拉取。注册、保存后更新和录制结束的恢复三条路径都记账，且都按动作归因：X11 逐个 `register`（不用全有或全无的 `register_multiple`），GNOME 恢复逐个写 binding 后只重启一次 gsd，因此一个键位被占用不会连坐另外两个。全部失败才把状态退回"已暂停"，部分成功保持"已恢复"，否则录制期的暂停会被跳过。Clippy 内部三个快捷键互相冲突由前端判定（它能读到未保存的录制值），桌面级冲突由 Rust 判定，X11 无法枚举时明确报告"无法检查"而不是"无冲突"。

GNOME 自定义快捷键条目路径按 command 认领而不是写死 `custom0/1/2`：这些编号先到先得，用户自己建的快捷键很可能已经占用，直接覆盖 name/command/binding 会静默销毁它。启动时读一次 `custom-keybindings`，认出带 Clippy D-Bus 方法的条目就原地复用，认不出来的再取未占用编号，结果在进程内缓存（`gsettings_shortcuts::plan_slots`）。

设置窗口关闭时，快捷键录制控制器先等待 `resume_shortcuts` 完成再关闭；Rust `AppState` 以原子标志和转换锁提供窗口销毁后的幂等恢复兜底。

`XDG_SESSION_TYPE` 优先于残留的 display 环境变量。Portal token 不进入普通配置；独立文件必须为 0600。首次 Portal 确认、撤权和桌面后端是否允许静默恢复仍属于真实桌面人工验收。
截图 Portal 的交互模式由截图用户动作显式开启；后台或未来自动任务应传入非交互模式，避免隐式弹出桌面授权。

## 安全规则

- 敏感条目在 Rust 内容选择阶段拒绝翻译；朗读条目文本走同一条内容选择路径，因此同样被拒绝。
- 朗读音频由 Rust 取回后以 data URL 播放，webview 不直接请求 dictvoice；文本长度上限 200 字符。
- 图片翻译只把本地 OCR 文本发送给 provider，不上传原图。
- API key 只进入系统 Secret Service，不提供明文 fallback。
- 成功的译文与原文落在同一个 SQLite 库（`translation_history`，全库上限 500 条）：条目删除、历史清空和上限清理都会一并删除它的译文，设置里另有"清空已保存的译文"入口。敏感条目从不进入翻译，因此也不会产生记录。
- 上面这条"删条目必然删译文"以及 `clips` 与 `clips_fts` 的一致性由事务保证：`insert_clip`、`delete_clip`、`clear_history`、`delete_entries` 都用 `unchecked_transaction()` 包住多条语句（`StorageEngine` 只持有 `&self`，并发已由外层 `Arc<Mutex<_>>` 串行化）。没有事务时中途失败会留下"搜得到但已不存在"的 FTS 幽灵行或删不掉的译文，而 `rebuild_fts_once` 只在 schema 版本变化时跑，索引不会自己长回来。
- 截图保存目录与文件名模板可配置（留空即内置默认 `~/Pictures/Clippy`）：模板只生成文件名，路径分隔符与前导点被清洗，写不到目录之外；同名时追加序号，不覆盖已有文件。
- 用户文本使用 React 文本节点或 `textContent`；富文本仅使用严格 DOMPurify 配置。
- URL 元数据仅访问无凭据的 HTTP(S)，拒绝私有/保留 IP、私有 DNS 解析和重定向；请求有 5 秒超时与 1 MiB 上限。
- 翻译响应有超时与 1 MiB 上限；数学表达式不使用 `eval`/`Function`。
- 非 2xx 响应只在 4xx 时读取最多 4 KiB 正文用于错误归类（把"缺少/无效 key"从不透明的 `http_status` 里区分出来），5xx 正文一律不读，网关错误页不会被误判成凭据问题。

## 质量门禁

`./scripts/ci-local.sh` 依次执行 Rust fmt/check/clippy/test、锁文件安装、TypeScript、Vitest、DOM/Xvfb smoke、Canvas 导出像素 smoke、主窗口布局像素 smoke 和 Vite build。两个像素 smoke 都需要 firefox 加 ffmpeg 或 python3-pil 读取截图像素，缺少时整步跳过（不算通过）。布局 smoke 直接 `?raw` 引入产品 `index.html` 的结构（headless Firefox 的 `--screenshot` 不等待顶层 `await`，异步 fixture 只会拍到空白页），断言失败时把原因画进红色浮层。criterion 基准（`src-tauri/benches/`，通过 `bench_support.rs` 调生产代码）被 `--all-targets` 编译但不运行，数字与运行方式见 [bench-baseline.md](bench-baseline.md)。Linux 发布目标仅为 deb/AppImage；updater 签名由 release CI secret 生成。
