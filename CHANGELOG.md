# Changelog

## 未发布

### ✨ 新功能

**建立 Linux、Windows 与 macOS 平台能力层**
- 后端按目标系统编译截图、自动粘贴、窗口层级、快捷键、OCR 安装和 tmux 实现，通过 typed IPC
  暴露结构化能力与 reason code，不再通过浏览器信息猜测平台。截图诊断会一并记录操作系统、会话、
  XWayland、各 Portal 接口版本以及可用/需授权/降级/不支持状态，但不记录剪贴板、token 或密钥。
- 设置页“关于”按同一份能力结构展示每项功能的可用、需授权、受限或不支持状态及本地化原因，
  同时列出桌面会话、XWayland 和具体 Portal 接口版本。
- Windows 使用原生前台窗口恢复与 `SendInput`，macOS 使用应用激活与 `CGEvent`；系统安全模型拒绝时
  保留复制结果并明确降级为 copy-only。macOS 另行处理屏幕录制、辅助功能、Spaces 与全屏辅助窗口。
- Windows 配置、Portal restore token 等私有文件使用受保护 DACL，只允许当前用户访问；滚动更新
  通过 `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)` 原子覆盖，避免第二次保存因目标已存在而失败。
- 非 GNOME Wayland 使用 XDG GlobalShortcuts Portal：按 XDG/xkb 语法提交建议组合，逐项核对 Portal
  实际接受的绑定；录制或修改快捷键会取消待确认请求并关闭旧 session。GNOME Wayland 继续使用
  Ubuntu 22 可用的 GSettings/D-Bus，Portal 不可用或被拒绝时保留可操作的手动绑定提示。
- 公共 Tauri 配置只保留跨平台字段，Linux、Windows、macOS 分别生成 deb/AppImage、NSIS/MSI、
  app/DMG；CI 在三个原生 runner 上执行 Rust 与前端门禁，并在无发布密钥模式下实际生成和核对
  Windows NSIS/MSI 与 macOS app/DMG，release 再汇总签名后的各平台 updater 产物。
- Windows 正式 release 强制导入带代码签名私钥的 PFX，以 SHA-256 和 RFC 3161 时间戳签名；上传前
  同时验证 NSIS/MSI 的 Authenticode 状态和证书指纹，缺少或过期证书不再发布“未知发布者”安装包。
- Ubuntu 22.04 重新成为 Linux 最低构建基线：默认依赖图使用 Jammy 可编译的截图实现，
  `pipewire-rs` 仅作为较新 Linux 可显式启用的增强 feature。

**贴图画布支持单文件可编辑 PNG 工程**
- 画布、交互和导出统一使用原图像素坐标；分数缩放下的清晰度补偿图只负责普通贴图显示，
  不再导致标注在保存时按补偿图/原图尺寸比例漂移。
- “保存可编辑 PNG”把最新合成图放进标准 IDAT，同时用真正压缩的 `clippy-project` iTXt
  保存 v2 工程（原图、尺寸/哈希、标注、调整与渲染器版本），保存后同一合成像素可立即粘贴；
  主窗口可用“打开图片”或 Ctrl+O 恢复并继续编辑。
- 可编辑文件明确提示包含未打码原图；“导出扁平 PNG”会去掉工程元数据，供模糊/马赛克后安全分享。
  编辑后的 Copy/Ctrl+C 始终复制最新合成像素，不携带工程块。
- 工程文件在 Rust 与 TypeScript 两层校验格式、PNG、尺寸、哈希、有限数值、对象 ID 和资源预算；
  普通 PNG、损坏工程、v1 和未来版本保留 IDAT 并按扁平图片打开。保存采用同目录临时文件原子提交，
  应用不会生成自己无法重开的工程。

**GNOME Wayland 上的窗口速选（自带 Shell 扩展）**
- 起因：GNOME Wayland 下客户端拿不到任何窗口的屏幕坐标，速选只能看到 XWayland 客户端
  （本机实测一整个会话只有 0~1 个）。逐个实测排除了 `Shell.Introspect.GetWindows`（只有宽高、
  且限定 xdg-desktop-portal 调用）、`Shell.Screenshot`（白名单外一律拒绝）、`Shell.Eval`（已禁用）、
  `ext-foreign-toplevel-list-v1` / `wlr-foreign-toplevel-management`（协议不含几何，Mutter 也不实现）、
  AT-SPI `Component.GetExtents`（原生 Wayland 窗口位置恒为 (0,0)）。唯一持有这份数据的是
  gnome-shell 自己，所以 Clippy 附带一个只上报窗口矩形的小扩展
  （`gnome-extension/clippy-windows@clippy.local/`，`include_str!` 内嵌，deb/AppImage/dev 同一份）。
- **设置 → 截图** 新增服务卡片：安装 / 卸载 / 重新检测，状态用低透明度的绿（在跑）、
  橙（装好了待注销、或被系统关掉了全部扩展）、红（未安装）底色 + 同色左边框区分。
- 安装**只能由用户点击触发**，应用绝不擅自往用户的 GNOME 里塞扩展；启动时只做自检
  （内容过期就静默升级、目录被手工删掉就清掉 gsettings 孤儿条目）。
- 装完需要**注销一次**才生效（Shell 不热扫描新扩展目录），安装后明确提示；**卸载即时生效**，
  不需要注销。deb `purge` 时兜底清理各用户目录下的扩展。
- 令牌：`GetWindows` 要求出示扩展目录里的 0600 令牌文件内容（32 字节随机数）。窗口标题会泄露
  用户在做什么，所以不对本机所有进程开放；挡得住其他用户与沙箱应用（有 session bus 但读不到
  `$HOME`），挡不住同用户的普通进程——同用户之间本来就没有边界，这点在文档里写明，不假装解决。
- 首次在 GNOME Wayland 上遇到速选不可用时，覆盖层给一条能照着做的提示（去设置页安装 + 注销），
  只提示一次（`capture_probe_hint_shown`）：没有这个服务照样能自由框选，反复提示纯属打扰。
- 扩展同时负责**拍冻结帧**（协议 v2 的 `Screenshot(s token) -> s`，同一个令牌把关——
  一整屏画面至少和窗口标题一样敏感）：拍整个 stage，不含光标、不闪白、不往图片目录写文件，
  落在 `$XDG_RUNTIME_DIR/clippy-shots/` 下的 0600 临时文件，Clippy 读完即删
  （Rust 侧只接受这个目录里的 `.png` 路径）。于是 GNOME Wayland 上的后端顺序变成
  扩展 → wlroots → Portal（非交互）→ GNOME → xcap。服务卡片因此改名 "GNOME Screen Helper"，
  文案覆盖两项能力。
- 新增 **"Update pending"** 状态：`ReloadExtension` 实测已废弃（`NotSupported:
  ReloadExtension is deprecated and does not work`），所以 Clippy 升级后磁盘上是新版、
  跑着的还是上次登录加载的旧版。这时卡片不再谎称"已就绪"，而是提示注销一次；
  期间窗口速选照旧可用，扩展截图自动回退到别的后端。

**设置页分页**
- 拆成 常规 / 截图 / 翻译 / 关于 四页，配 roving tabindex 与方向键/Home/End 导航，
  记住上次停留的分页。面板只切 `hidden` 不销毁——各控制器在装配时就抓住了元素引用。

**贴图窗口置顶，并且落回原来的位置和大小**
- 扩展协议抬到 **v3**，新增 `PlaceWindow(s token, u pid, s marker, i x, i y, b reposition, b above) -> b`。
  GNOME Wayland 上客户端既不能给自己定位、也不能自己抬到最上层——`set_position` 与
  `set_always_on_top` 都是**静默空操作**，所以"贴图出现在屏幕中间、随手被别的窗口盖住"
  一直是这条限制的表现，不是代码漏了。唯一能做到的地方还是 gnome-shell 进程内部
  （`MetaWindow.move_frame` / `make_above`）。摆位与置顶都是**能则用**：两条路都失败只意味着
  位置或层级不理想，绝不让贴图本身失败。X11 上照旧走 Tauri 自己那套。
- 截图选区的桌面逻辑坐标随 `commit_capture_action` 的新 `origin` 参数传到后端，贴图就落在
  刚才框的那块上，大小也一致（只缩不放地钳进工作区）。窗口位置是 `origin − 12 逻辑像素`：
  要盖住原处的是内容区，而内容区相对窗口原点偏移一圈阴影留白。
- **从剪贴板历史里再 Pin 同一张图也回原处**：`PinOriginRegistry`（`pin/origins.rs`，
  挂在 `AppState.pin_origins`，最多记 16 条）在"复制成功之后"登记来源矩形。键是
  **解码后像素**的 sha256（宽高一起进哈希），不是 PNG 字节——图片经 arboard 走一圈是原始 RGBA，
  watcher 会重新编码，PNG 字节不稳定，按字节比对必然认不出来。
  从别处复制来的图没有位置信息，仍旧居中显示，这是设计。
- ⚠️ 协议抬版之后**已登录的会话必然是 `stale`**（设置页显示 "Update pending"），
  此时摆位与置顶会退回 Tauri 那套、在 Wayland 上看起来像没生效。注销重登一次才生效。

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

**贴图窗口调不透明度的按键从 Ctrl+滚轮改成 Shift+滚轮**
- WebKitGTK 把触控板捏合合成成 ctrl+滚轮，留着这个绑定的话捏合会顺手把贴图调成半透明。
  现在 ctrl/meta+滚轮在贴图上是**彻底的空操作**（照旧 `preventDefault`，不落给页面缩放）。

### 🐛 修复

- **截图复制完去 Pin 剪贴板条目，贴出来的是上一张图**。根因是**前端焦点索引漂移**，
  与入库快慢无关（用户的库里那几张新图 id、哈希、字节数全都对，是焦点指错了行）：
  - 面板一失焦就 `releaseMemory()` 把列表清空，而 `releaseNavigation` 当时把 `focusedRow`
    归成 **0**——列表已经空了，0 是个不存在的行。接着 `clip-added` 到达，`prependClip` 把这个
    幻影焦点当真、按 `focusedRow + 1` "让"给新条目，于是重新打开面板时焦点停在**第二行**。
    Pin 的两个入口（全局快捷键与 Ctrl+P）都读 `getFocusedClip()`，列表行上又没有 Pin 按钮，
    所以从历史里 Pin 出来的必然是上一条。躲在后台连截两张，焦点就掉到第三行。
  - 两处一起改：`releaseNavigation` 现在报 `focusedRow: -1`（"没有焦点行"，重新加载时
    `normalizeAfterRefresh` 会收拢成 0，所以打开面板照样高亮最新那条）；`prependClip` 改成
    **按 id 跟踪焦点**而不是做索引加减——用户正看着的那一行不该在眼皮底下换成别的内容。
    顺带修掉第二个老毛病：同一张图再复制一次时 `insert_clip` 按哈希去重、只把它顶到最前，
    列表长度不变，原来的 +1 纯属把焦点推歪。
  - `js/clipboard-list.js` 与 `react/main/clipboardStore.ts` 两份实现同时改（前者目前运行时
    不生效，但只改一份的话渲染一旦切回去 bug 就复活），regression guard 钉住两边都不做索引加减。
  - 覆盖层工具条上的图钉那条路本来就是对的（用的是刚提交的那份 PNG 字节，标签也唯一），
    与这个 bug 无关。
- **顺带缩掉自己复制的内容进历史的延迟**（排查上面那条时先怀疑的是这里，实测不是根因，
  但改进本身是对的）：点对钩的 `Copy` 只把 PNG 写进系统剪贴板（`capture/mod.rs`），
  **入库是 watcher 下一次轮询才做的事**，而那是个 500 ms 的裸 `thread::sleep` 循环，
  没有任何唤醒口。现在 `writer.rs` 的三个写入口写成功后都敲一下 `wake::nudge()`，
  watcher 的等待换成带待处理标记的条件变量（`clipboard_watcher/wake.rs`），当场醒来把它收进去，
  窗口期从最多 500 ms 缩到几毫秒。
  **没有改成"写入方自己 `insert_clip`"**：watcher 的哈希算的是它自己从剪贴板 RGBA 重新编出来的
  那张 PNG，与我们手里的字节几乎不可能一致，写入方插一条之后 watcher 500 ms 后还会因为哈希不同
  再插一条；要让两边一致就得用 watcher 的编码器把整图再编一遍，那正是刚从提交热路径上省掉的
  开销。入库仍然只有 watcher 一条路径——哈希基准、去重、`clip-added` 全都不变。
  用布尔标记而不是裸 `notify_one()`，是因为敲的时候 watcher 可能正忙着编码上一张图，
  裸通知会被丢掉。
- **全局 Pin 快捷键不再退回前端列表缓存**：原来是 `getFocusedClip() || getLatestClip()`，
  而 `getLatestClip()` 读的是内存缓存 `_allClips[0]`，读库兜底只在列表整个空掉时才生效。
  现在没有明确焦点行时**一律问后端要最新一条**（`js/pin-target.js`）。
- **删掉没有调用方的 `pin_screenshot_image`**：独立的截图编辑器窗口下线后它就没人调了，
  留着等于白留一个"从任意 base64 开窗"的入口。
- **装上扩展之后按截图快捷键直接 panic**（`Cannot start a runtime from within a runtime`）：
  ashpd 打开了 zbus 的 `tokio` feature，于是 `zbus::blocking` 内部用一个静态多线程 runtime 做
  `block_on`，而 tokio 不允许在已进入 runtime 的线程上再起一个。实测 async worker 线程上必炸、
  `spawn_blocking` 线程侥幸能过——于是"装扩展前一切正常、装完就崩"：没装时 `probe()` /
  `hint_needed()` 在碰 D-Bus 之前就返回了。新增 `dbus.rs` 作为阻塞 D-Bus 的唯一入口
  （先跳一条干净的 OS 线程再开连接），扩展与 `org.gnome.Shell.Screenshot` 两处共六个调用点
  全部改走它；`dbus::tests::blocking_calls_survive_inside_an_async_task` 正反两个方向都钉住。
- **按截图快捷键弹出来的是系统的截图界面**：xdg-desktop-portal 的非交互截图在第一次使用前要弹
  一次系统授权对话框，而 gnome-shell 只允许**当前聚焦的应用**弹它（实测
  `AccessDenied: Only the focused app is allowed to show a system access dialog`）。截图由全局
  快捷键触发，此刻 Clippy 没有聚焦窗口，于是这条路必然失败、并回退到 `interactive = true`——
  在 GNOME 上那就是 GNOME 自己的截图 UI。现在 GNOME Wayland 优先走自带扩展拍帧，
  交互式回退整条删掉：宁可干净地失败，也不能让用户按 Clippy 的快捷键看到系统截图界面。
  （顺带记一笔：Portal 的权限按 app id 存，而 app id 来自 systemd scope，dev 下是 `code`，
  所以"我记得授权过"跟当前进程能不能用是两件事。）
- **截图往用户的图片目录里堆文件**：xdg-desktop-portal-gnome 把每张非交互截图写成
  `~/图片/Screenshot-N.png` 并把处置权交给调用方，而 Clippy 只读不删，攒出过几十个残留文件。
  现在 Portal 与扩展两条路的返回文件都由 `TemporaryScreenshotFile` 兜住，
  解码成功与否都会在作用域结束时删除。
- **"截屏是黑的"**：冻结帧本来就是正常的，黑的是覆盖层窗口自己——Wayland 不允许客户端摆放窗口，
  Tauri 的 `position()`/`set_size()` 被 GNOME 静默忽略。改为配置底层 GTK 窗口，
  由合成器 `fullscreen_on_monitor` 铺满按最大重叠面积选出的显示器。
- **截图完成后整屏白 2 秒才出画面**：覆盖层建窗就 `show()`，于是加载 webview、取 payload、
  解 PNG 的整段时间里用户盯着的是 webview 默认底色（白）铺满整屏。改为隐藏建窗 +
  前端画完首帧再调 `mark_capture_overlay_ready` 显示，窗口与 webview 底色同时设为不透明黑；
  前端出错时也立刻显示（否则错误提示没机会露面），并有 2.5 秒超时兜底，避免加载失败留下
  一个看不见又按不了 Esc 的会话。焦点给光标所在那块覆盖层，不再是"谁先画完谁拿"。
- **dev 构建下截图首帧要等一两秒**：`[profile.dev.package]` 给 `image`/`png`/`fdeflate`/
  `miniz_oxide` 等开 `opt-level = 3`。2560×1600 冻结帧的 PNG 编码从 1408 ms 降到 414 ms
  （本机实测），自己的代码仍不优化，调试体验不变。
- **截图里的画面缩放不对**：xcap 的 `Monitor::width()` 在 1920×1200/scale 1.3333 的桌面上返回 2880×1800，
  既不是逻辑尺寸也不是物理尺寸；改按 `round(帧像素 / scale_factor)` 归一化。
- **窗口速选框歪了**：`xcap::Window` 给的是 X screen 原始像素的**客户端**矩形，
  混了坐标空间（实测一个 QQ 窗口被报成 2598 像素宽）；现在先按 `X 像素 / 逻辑像素` 折算，
  再减掉 `_GTK_FRAME_EXTENTS`（CSD 阴影），小于 20 逻辑像素的候选丢弃。
- **窗口速选按"面积小的赢"来判遮挡，答案与肉眼相反**：一个大窗口压在小窗口上时选到的是
  看不见的那个。改为真正的堆叠顺序——扩展侧 `sort_windows_by_stacking().reverse()`，
  X11 侧读 `_NET_CLIENT_LIST_STACKING`（协议自下而上，同样反转），枚举不到的窗口沉到最底；
  前端 `windowAt` 取第一个命中的候选，被完全遮住的窗口自然选不到。
- **可见的窗口被当成最小化而整个消失**：xcap 的 `is_minimized()` 把一个正常显示的 QQ 窗口
  报成已最小化，于是唯一的候选被丢掉。改为读 ICCCM `WM_STATE`（首值 `3 == IconicState`），
  用的是模块本来就持有的那条 X 连接。
- 顺手删掉随编辑器窗口一起失去入口的三个命令（`copy_screenshot_image`、`save_screenshot_image`、
  `save_screenshot_image_as`）：它们接受任意 base64 就写文件/剪贴板，留着是没人用的攻击面。
- **在贴图上用触控板捏合会把整个页面缩放掉**（内容溢出窗口、工具栏错位）：React 17+ 把
  `wheel` 注册成**被动**监听器，所以写在 `onWheel` 里的 `preventDefault()` 是空操作，
  而 WebKit 把捏合合成成 ctrl+滚轮、默认行为就是页面缩放。改为自己用 `{ passive: false }`
  注册原生监听器并无条件 `preventDefault`；同时吃掉 WebKit 专有的 `gesturestart/change/end`
  与 Ctrl+`+`/`-`/`=`/`_`/`0` 这几个页面缩放快捷键（`+`/`-` 顺手转成贴图自己的缩放）。
  后端再加一道锁：`pin_window.rs` 在建窗时把 WebKitGTK 的 `zoom_level` 钉在 1.0，
  `zoom-level` 通知里发现被改动就改回去（比较留容差，严格比较会让回调自己触发自己）。
  滚轮缩放贴图不受影响——那是我们自己处理的手势，不是页面缩放。
- **拖动贴图有时被识别成"选中图片"，整块刷成橙色**（Ubuntu 的系统强调色）：
  拦掉 `selectstart` 与 `dragstart`（文本贴图的 `<pre>`、输入框与 `contenteditable` 例外，
  否则文本贴图就没法划选了），图片加 `-webkit-user-drag: none`，并把 `::selection` 底色兜底成透明。
  **窗口照旧可以拖动、也照旧可以点击获得焦点**——只是不再选中内容。
- **截完图按全局 Pin 键，贴出来的还是上一张**（前两轮都没修到根上）。真正的触发条件是
  **右侧预览或左侧编解码侧栏开着**：这时失焦不隐藏主窗口（`window_events.rs::
  should_hide_on_focus_loss`），前端也就不会 `releaseMemory()`（`app.js::onWindowBlur`），
  于是整份列表连焦点一起活过整个截图流程。而 `prependClip` 是**按条目**跟踪焦点的
  （用户正看着的那行不该在眼皮底下换成别的内容，这是对的），新截图插到第 0 行后焦点跟着
  老条目挪到第 1 行——按 Pin 贴的就是上一张，再截一张就掉到第 2 行。
  两道独立的修法：
  - `resolvePinTarget` 新增第三个参数 `panelFocused`，只有 `document.hasFocus()` 为真时
    才信焦点行；全局快捷键不会抢焦点，所以面板没焦点就一定不是"用户正看着的行"，
    一律问后端要最新一条。漏传参数时退化成"问后端"，慢一点但绝不会贴错。
  - `onWindowFocus` 两条分支都 `restoreRender()`。以前只有"不脏"那条复位，
    而 `refresh()` 里的 `normalizeAfterRefresh` 只做钳位、不复位，于是面板不可见期间
    来了新条目时，打开面板高亮的是第二行，按回车/Ctrl+P 命中的也是上一条。
### ⚡ 性能

**截图从按下快捷键到覆盖层出现：1449 ms → 901~957 ms**（本机实测，dev 构建，
单屏 2560×1600 物理 / 1920×1200 逻辑，GNOME 50.1 Wayland）

- 先量后改。新增 `StageTimings`：每次会话在 `CaptureManager::reveal` 打一条分段汇总日志
  （冻结帧 / 窗口候选 / 建窗与 webview / 后端交付 / 前端绘制），更细的分解用
  `cargo test --lib capture_stage_timings -- --ignored --nocapture`。
  结论之一是**报障猜的"获取窗口导致变慢"并不成立**：窗口枚举全程 3~4 ms，
  1449 ms 里占千分之三。
- **冻结帧像素不再走 JSON。** payload 去掉 `pngBase64`，改由新命令 `get_capture_frame`
  以二进制 IPC 直传原始 RGBA，前端 `new ImageData` + `putImageData` 铺进离屏 canvas。
  省掉的是同一张图被处理四次：Rust 编 PNG（215 ms）→ base64（3 MB 字符串）→
  webview `atob` → WebKit 解 PNG。后端交付 225 ms → ~0 ms，前端解码绘制 453 ms → 108~156 ms。
  字节数反而更大（16 MB vs 2.2 MB），但两头都是零编码。
- 隐藏自己的窗口之后那 140 ms 合成器落定等待改成**按需**：快捷键截图的常态是面板本来就没开着
  （`hide_sources` 返回空），这时白等纯粹是加在感知延迟上的。面板开着时照旧等，
  少了它会把 Clippy 自己的面板烧进冻结帧。
- 剩下的 550 ms 冻结帧**是地板**：gnome-shell 在自己进程里把 stage 编码成 PNG 才交回路径，
  而 Shell 的 typelib 里 JS 能碰到的像素导出路径全都要么落 PNG、要么给一个不透明的
  `Clutter.Content`，扩展这一侧没有拿到原始像素的路。已在文档里写明，别再重复调研。
- `env_logger` 默认过滤器设为 `clippy_lib=info,warn`。此前默认只放行 `error`，
  于是所有 `log::info!`/`log::warn!`（包括"覆盖层超时未报告首帧"）都是写给空气的，
  排障线索一条都看不到；`RUST_LOG` 仍然优先。

**贴图缩放的每帧开销**（滚轮缩放走 `update_pin` → `resize_pin_window` → `keep_pin_above`
→ `PlaceWindow`，而 `update_pin` 是同步命令、**跑在主线程上**，这条路上的任何阻塞都直接卡 UI）

- **session D-Bus 连接改成复用**（`dbus.rs`）。`Connection::session()` 每次都要重做一轮
  SASL 握手 + `Hello`（1~2 ms），而缩放时每帧要发两次（`GetVersion` + `PlaceWindow`）。
  连接缓存在 `OnceLock<Mutex<Option<Connection>>>` 里（`zbus::Connection` 是 Arc 支撑的
  Send+Sync 句柄，克隆很便宜）。失效判据是 `worth_reconnecting`：`Error::MethodError`
  说明对端**应答了**，连接是好的，绝不能重连重试；其余错误才丢缓存重连一次。
  **存入缓存的判据（`worth_caching`）必须是它的反面**：只按 `result.is_ok()` 判会把一条被
  业务错误拒绝、但本身完好的连接扔掉，于是"扩展没装 / 版本不对"这类每次都失败的探测，
  每一次都要重新握手一遍，而这条路上有跑在主线程的调用方。
- **`place_window` 的前置检查加缓存**（`capture/shell_extension.rs`）。以前每帧都要
  读 `metadata.json`、比一遍 gsettings `enabled-extensions` 那串 13 KB 字符串、发一次
  `GetVersion` 协商协议、再读一次令牌文件。现在结果缓存在 `placement_token()`：
  成功一直有效，失败 30 秒后重试（用户可能刚在设置页装上），`install()`/`uninstall()`
  以及 `PlaceWindow` 自己报错都会立刻作废缓存。
- **`update_pin` 的应答不再带图片。** 新增 `PinState`（label、内容尺寸、scale、opacity、
  locked、position），**故意不含 `imageBase64`/`text`**：每帧把整张图重编一遍 base64
  纯属浪费，而且会让前端重建图片 object URL、造成闪烁。前端
  `react/pin/update-order.ts::mergePinState` 把应答合并进手里那份 payload
  （直接整份替换会把内容与 `canSave` 抹成 undefined，`pin-gestures.test.js` 钉住了这点）。

**打开面板时的图片条目**

- **列表行不再取原图。** 新增 `get_clip_thumbnail` 命令：后端用 `image` crate 的
  `thumbnail()` 快路径把图缩到最长边 128 px 再交出去（行里那格是 48 CSS px，
  2× 屏上 96 物理像素）。此前两个列表渲染器都调 `get_clip_image`，于是为了画 48×48
  要把库里存的原图（一张全屏截图 2560×1600 / 几 MB）整份送进 webview 再全尺寸解码一次，
  一次开面板十几个图片条目就是几十 MB IPC 加十几次 PNG 解码，全落在 webview 那一个线程上。
  缩完一条几 KB，base64 那 33% 的膨胀也就无所谓了，不必为它改二进制 IPC。
  预览面板照旧取原图（缩略图会糊），这条由回归断言钉住。
- 缩略图结果按 id 缓存（进程级 FIFO，64 条，`ThumbnailCache`）：缩一次要解一遍原图，
  而同一条记录会反复出现在每次打开的列表里。`clips.id` 自增不复用、内容不可变，
  所以这个缓存永不失效。
- `get_clip_image` 改成 `async`：同步命令跑在 GTK 主线程上，而它要读一个几 MB 的 blob
  再 base64 编一遍。函数体里没有 `.await`，`MutexGuard` 不跨让点。

**其它**

- **截图点对钩时少解一遍全屏 PNG。** `Copy` 分支以前解码两次：一次给剪贴板、一次给来源登记。
  现在解一次，两边共用同一份像素。登记方收的是新的 `PinFingerprint`（宽高 + 已经算好的摘要），
  所以既不用再解码、也不用复制那 16 MB 像素，同时保持"复制成功之后才登记"的顺序
  （复制失败就登记，只会让将来某张碰巧一样的图错位）。
- **`PinOriginRegistry::lookup` 先比宽高再解码。** 尺寸单独存一份，只读 PNG 文件头就能判定；
  没有任何登记项的尺寸对得上时直接返回，省掉整张解码。
  `the_size_prefilter_never_hides_a_real_match` 钉住这个预筛不会漏掉真匹配。
- **覆盖层的 payload 与底图改成并行请求。** 两个 effect 都只依赖窗口 label，铺画布放在
  第三个 effect 里等两边到齐。像素是这两次里慢的那个（16 MB），串起来等于把它的往返
  白加在覆盖层出现之前。
- `image_io.rs` 解码后用 `into_rgba8` 而不是 `to_rgba8`：PNG 常见就是 RGBA8，
  前者原地接管缓冲区，后者要再拷一份 16 MB。
- **删掉两处白算的图片 skip hash**（`pin/commands.rs::copy_pin` 与
  `commands/clipboard.rs` 的图片分支）。watcher 哈希的是**它自己**从剪贴板 RGBA
  重新编出来的 PNG，和写入方手上那串字节几乎不可能相同，所以这两次全图 sha256
  永远匹配不上、纯属白算。文本那几路能生效是因为两边哈希的是同一串字节。
  后果只是这张图会被顶到历史最前面——`insert_clip` 按哈希去重，不会多存一份；
  两处都写了注释说明，免得有人"修好"它。
  （**没有**改成按像素哈希：watcher 手上确实有 RGBA，但哈希 16 MB 比哈希 2 MB 的 PNG
  贵约 8 倍，而且换哈希基准会让库里已有的图片全部对不上、悄悄破坏去重。）
- 顺手改掉一句错的注释：`get_capture_frame` 是**零编码**，但不是零拷贝——
  `InvokeResponseBody::Raw` 要 `Vec<u8>`，而帧还得留在会话里给选区翻译用，
  所以那一次 16 MB memcpy 去不掉（实测 2 ms 上下）。`docs/capture-linux.md`
  §3.1 的耗时表里"后端交付底图"从 `~0 ms` 改为 `~2 ms`。

### 📄 文档

- `docs/capture-linux.md` 新增 §3.1「从快捷键到覆盖层出现，时间花在哪」（优化前后的分段对照表、
  523 ms 的 gnome-shell 拍照为什么是地板、还剩下的那个可选优化为什么没做）与
  §3.2「冻结帧像素怎么送进覆盖层」（RGBA8 契约、为什么原始字节必须自己核对长度、
  为什么别改回 base64、payload 与底图为什么必须并行请求）。
- `docs/capture-linux.md` §1 新增「`PlaceWindow` 是每帧热路径，而且跑在主线程上」：
  前置检查缓存的失效规则、D-Bus 连接复用、`update_pin` 应答为什么不带图片。
  `docs/architecture.md` 的 `dbus.rs` / `capture/shell_extension.rs` / `pin/` 三行同步补上
  连接复用、摆位探测缓存与 `PinFingerprint`，并新增一行写 `update_pin` 的应答契约。
- 新增 [docs/capture-linux.md](docs/capture-linux.md)：**每个窗口的大小能不能拿到**的系统 API 调研结论
  （X11/xcb 能；GNOME Wayland 只能枚举 XWayland 客户端；`wlr-foreign-toplevel-management` 只有标题没有几何；
  `org.gnome.Shell.Screenshot` 对普通应用 `AccessDenied`），三套坐标空间的换算规则、
  覆盖层摆放为什么必须交给合成器、以及快速选区的交互约定。
- `docs/capture-linux.md` 补齐自带 Shell 扩展这条路：被排除的接口逐条列出实测结论、
  扩展为什么只装用户目录、装了必须注销而卸载即时、令牌的威胁模型（挡得住谁、挡不住谁）、
  堆叠顺序为什么不能用面积近似，以及"速选一个被部分遮住的窗口会包含遮挡者像素"这个已知取舍。
- `docs/capture-linux.md` 补上贴图窗口这一路：§1 新增「同一条限制也卡住贴图窗口」
  （`set_always_on_top` 同样是空操作、`show → set_focus → PlaceWindow` 的顺序为什么不能改、
  尺寸为什么留在客户端、`window_marker` 是两侧共享的查找键）、§2.1 的四个方法与协议 v3
  及"抬版必须三处一起抬 + 注销才生效"、威胁模型里 `PlaceWindow` 的 `pid` 是限定作用域而非安全边界、
  §4 新增选区坐标要加 `logicalX/logicalY` 与「窗口位置 = 原始矩形 − `SHADOW_GUTTER`」两条换算规则、
  §6 新增贴图的人工验收清单（摆位/尺寸、置顶、捏合、拖动不选中，以及验收前必须先注销重登）。
- `docs/capture-linux.md` 新增 §3「冻结帧走哪个后端」：五个后端的先后顺序及其理由、
  Portal 授权对话框只允许聚焦应用弹出这条实测结论、app id 来自 systemd scope、
  Portal 会往图片目录写文件、以及 `ReloadExtension` 已废弃因此存在 `stale` 状态。

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

- **恢复 Ubuntu 22.04 构建**：Linux 默认目标固定使用 Jammy 可编译的截图依赖，不再把需要新版
  PipeWire 头文件的 `xcap 0.9` Linux 后端放入默认依赖图；较新 Linux 仍可从源码显式启用
  `linux-pipewire` 增强。发布产物使用 `ubuntu22` 后缀，并保留 updater 使用的无后缀 AppImage。
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
