# Linux 截图：窗口几何、坐标空间与覆盖层摆放

这份文档记录截图链路上四件反复踩坑、且**光看代码看不出来**的事：覆盖层窗口为什么不能自己摆位置、
每个窗口的大小到底能不能拿到、冻结帧该走哪个后端、以及三套坐标空间怎么换算。改动
`src-tauri/src/capture/`、`src-tauri/src/screenshot/` 或 `src/react/capture-overlay/` 之前先读这里。

## 1. 覆盖层窗口必须由合成器摆放

**不要用 Tauri 的 `position()` / `set_position()` / `set_size()` 来摆覆盖层。**
Wayland 协议里客户端无权决定自己窗口的位置（`xdg_surface` 只描述内容，摆放是合成器的事），
GNOME 会静默忽略这些调用。表现就是"截屏是黑的"：冻结帧其实是正常的（Rust 侧实测平均亮度 43.7，
全黑像素 0%），但覆盖层窗口落在错误的显示器上、或者尺寸不对，用户看到的是一块没有画面的窗口。

正确做法在 `capture/overlay_windows.rs`：拿到底层 GTK 窗口后设置
Splashscreen 类型提示、去装饰、不进任务栏、置顶、`stick`，然后
`gtk_window.fullscreen_on_monitor(&screen, index)` —— 由**合成器**把窗口铺满指定显示器。
显示器编号用 `OverlayRect` 与 GDK 几何求**最大重叠面积**来选（`best_monitor_index`），
不按索引猜：显示器顺序在 xcap、GDK 和 RandR 之间并不一致。

没走 gtk-layer-shell 是有意的：GNOME 不实现 `wlr-layer-shell`。在 wlroots 系合成器
（sway/Hyprland）上 layer-shell 能给出更可靠的"覆盖整个输出、独占键盘"的语义，
那是未来的升级路径，但它换不来 GNOME 上的任何好处。

非 Linux 平台仍走 `set_position` / `set_size` 兜底。

### 同一条限制也卡住贴图窗口

贴图（Pin）窗口不是覆盖层，不能 `fullscreen_on_monitor`——它就是一个要落在指定位置的小窗。
但 Wayland 的限制一样管着它，而且还多一条：**`set_always_on_top` 也是静默空操作**。
所以 `pin/window.rs` 里那两行 `always_on_top(true)` 和 `set_position(...)` 在 GNOME Wayland 上
从来没生效过——贴图出现在屏幕中间、随手被别的窗口盖住，根源是这个，不是代码漏了。

唯一能做到的地方还是 gnome-shell 进程内部：`MetaWindow.move_frame(false, x, y)` 与
`make_above()`。所以 `pin/window.rs::keep_pin_above` 先走扩展的 `PlaceWindow`（§2.1），
失败才退回 Tauri 自己那套（X11 上它本来就管用）。两条路都失败只意味着位置或层级不理想，
**绝不能让贴图本身失败**。

摆放时机也不能改：`MetaWindow` 只在窗口真的映射之后才存在，所以顺序必须是
`show()` → `set_focus()` → `PlaceWindow`（见 `reveal_pin_window`）。缩放之后要再调一次，
因为改尺寸会顺带把窗口带回普通层。

**于是 `PlaceWindow` 是每帧热路径，而且跑在主线程上**（`update_pin` 是同步命令，
Tauri 只把 `async fn` 挪到 async runtime），所以这条路上不能有多余开销：

- 前置检查（是不是 GNOME Wayland、扩展装没装、跑着的协议版本够不够、令牌文件内容）
  缓存在 `shell_extension::placement_token()`。以前每帧要读两个文件（`metadata.json` 与
  gsettings 的 `enabled-extensions` 13 KB 字符串比较）、发一次 `GetVersion`、再读一次令牌文件。
  成功的结果一直有效；失败的结果 30 秒后重试（`PLACEMENT_PROBE_NEGATIVE_TTL`，
  用户可能刚在设置页装上）；`install()` / `uninstall()` 立刻作废缓存，
  `PlaceWindow` 自己报错也作废（扩展可能刚被禁用）。
- session D-Bus 连接由 `dbus.rs` 复用，见那里的"连接是复用的"。以前每帧两次
  `Connection::session()`，每次一轮 SASL 握手 + `Hello`。
- `update_pin` 的应答不带图片（`PinState`），否则每帧要把整张图重编一遍 base64。

尺寸仍走客户端：`set_size` / `inner_size` 在 Wayland 上是有效的（那是内容尺寸，客户端说了算），
而且贴图窗口是 `resizable(false)`，让 Shell 去 `move_resize_frame` 反而可能被 GTK 拒绝。
**只有位置和层级借道扩展。**

扩展找窗口靠的是"pid + 窗口标题"。贴图窗口无装饰、不进任务栏，标题在界面上看不到，
所以拿它当查找键：`pin/model.rs::window_marker(label)` = `"Clippy Pin {label}"`。
改这个格式两侧要一起改。

### 显示时机：隐藏建窗 + 前端报告首帧

覆盖层**建窗时是隐藏的**，由前端画完第一帧后调 `mark_capture_overlay_ready` 才显示。
原因：webview 的默认底色是白的，而覆盖层铺满整屏——建窗就 `show()` 的话，加载 webview、
取 payload 与底图的整段时间用户盯着的是**一整屏白色**（实测约 2 秒，与系统截图那一下的
闪白是两件事）。另外窗口与 webview 的底色都显式设成不透明黑（`background_color`），
这样任何一帧还没画完的画面都不会是白的。

两条保险：

- `overlay_windows::READY_FALLBACK_MS`（2500 ms）超时兜底。前端加载失败或 JS 抛异常时
  没人报告首帧，窗口会一直隐藏着占用会话——用户既看不到它也按不了 Esc。超时后强制显示并记
  `log::warn`。
- 前端出错（payload 取不到、底图字节数不对）时也立刻报告 ready，否则错误提示根本没机会露面。

焦点归属由 `CaptureManager::reveal` 决定，不是"谁先画完谁拿"：光标所在的那块覆盖层独占焦点
（合成器可能拒绝第二次 `set_focus`，所以不能先给错的那块再让），拿不到光标位置时退化为先到先得，
保证总有一块能接 Esc。

`[profile.dev.package]` 给 `image`/`png`/`fdeflate`/`miniz_oxide` 等开了 `opt-level = 3`。
底图已经不再经过 Rust 的 PNG 编码（见 §3.2），但这条设置照样要留着：扩展交回来的那张
全屏 PNG 得由 Rust 解码，选区翻译的裁剪帧也要编码成 PNG 喂给 OCR。自己的代码仍是
`opt-level = 0`，调试体验不变。

耗时分解见 §3.1——真正的大头是 gnome-shell 自己拍照那 550 ms，不是窗口枚举。

## 2. 能不能拿到每个窗口的大小

能，但**只在 X11 协议可达时**。调研结论按平台分：

| 环境 | 可用接口 | 能拿到几何吗 |
|---|---|---|
| X11 | `XQueryTree` / `XGetWindowAttributes` / `_NET_CLIENT_LIST_STACKING`（xcb），即 `xcap::Window::all()` 走的路 | 能，含位置与大小 |
| GNOME Wayland | 同上，但 X server 是 XWayland | 只能拿到 **XWayland 客户端**；原生 Wayland 应用列不出来（本机实测一整个会话只有 0~1 个） |
| GNOME Wayland（Shell 的现成接口） | 见下表，逐个实测排除 | 不能 |
| GNOME Wayland（**自带 Shell 扩展**） | `org.gnome.Shell.Extensions.ClippyWindows` | **能**，含位置、大小、标题、堆叠顺序 |
| wlroots（sway/Hyprland） | `wlr-foreign-toplevel-management` | 只有标题 / app_id / 状态，**协议不含几何**；GNOME 也不实现它 |
| xdg-desktop-portal | ScreenCast / Screenshot | 只给画面，不给窗口列表 |

GNOME Wayland 上被逐个排除掉的现成接口（都在 GNOME Shell 50.1 / Ubuntu 上实测过，
别再重复一遍这些弯路）：

| 接口 | 实测结论 |
|---|---|
| `org.gnome.Shell.Introspect.GetWindows` | 只下发 `width`/`height`，**没有 x/y**；而且调用方被 `DBusSenderChecker` 限定为两个 xdg-desktop-portal 实现，别的进程连调都调不到 |
| `org.gnome.Shell.Screenshot` | 整个接口白名单外一律 "is not allowed" |
| `org.gnome.Shell.Eval` | 早已默认禁用 |
| `ext-foreign-toplevel-list-v1` / `wlr-foreign-toplevel-management` | 协议里根本没有几何字段，且 Mutter 两个都不实现 |
| AT-SPI `Component.GetExtents` | 能枚举全部窗口、尺寸也对，但**原生 Wayland 窗口位置一律返回 (0,0)**，只有 XWayland 窗口有真坐标 |

也就是说：Wayland 的安全模型里没有面向普通应用的"列出别人的窗口并读它的矩形"接口，这不是缺实现，
是刻意不给。唯一持有这份数据的地方是 gnome-shell 进程自己——它自带的截图 UI 就在用
`global.get_window_actors()` + `get_frame_rect()`。所以要在 GNOME Wayland 上做窗口速选，
只有一条路：**以扩展的身份进到那个进程里去取**。

无论如何，**窗口速选都是能则用的增强，不是必需功能**：`probe_windows` 枚举失败或拿到空列表时
后端记 `log::info`，覆盖层显示 "Window picking unavailable in this session — drag to select an area"
（i18n key `capture.windowPickingUnavailable`），拖拽框选与"点一下取整屏"完全不受影响。

### 2.1 自带的 GNOME Shell 扩展

源码在 `gnome-extension/clippy-windows@clippy.local/`，通过 `include_str!` 编进二进制
（见 `capture/shell_extension.rs`），因此 deb / AppImage / dev 三种运行方式用的是同一份内容。

- **接口**：`org.gnome.Shell.Extensions.ClippyWindows`，对象路径
  `/org/gnome/Shell/Extensions/ClippyWindows`，四个方法 —— `GetVersion() -> u`（不校验令牌，
  只用来探活与协议协商）、`GetWindows(s token) -> s`（返回 JSON 数组）、
  `Screenshot(s token) -> s`（拍整个 stage，返回 PNG 路径，见 §3），以及
  `PlaceWindow(s token, u pid, s marker, i x, i y, b reposition, b above) -> b`
  （把调用方自己的某个窗口摆到指定位置并/或置顶，用于贴图窗口，见 §1）。
  当前协议版本 **v3**（`PlaceWindow` 从 v3 起存在；低于 v3 的扩展照样能截图与窗口速选，
  只是贴图回不到原位、也压不住别的窗口——退化，不是故障）。
- **不需要坐标换算**。扩展给的是**逻辑像素**，而且 `get_frame_rect()` 已经排除了 CSD 阴影。
  §4 的 `x11_pixel_ratio` 和 `_GTK_FRAME_EXTENTS` 裁边都只服务于 X11 那条路，
  不要把它们套到扩展的结果上。
- **装了必须注销一次**。Shell 不会热扫描新的扩展目录（实测 `EnableExtension` 直接返回 `false`），
  唯一的热加载入口 `InstallRemoteExtension` 要求先发布到 extensions.gnome.org。
  所以设置页安装后明确提示"注销后生效"，`InstallOutcome.needs_logout` 就是这个用途。
  反过来 **卸载是即时的**：`disable()` 当场把 D-Bus 对象反注册掉，不需要注销。
- **升级同样要注销一次，而且这事必须显示出来**。`org.gnome.Shell.Extensions.ReloadExtension`
  实测已废弃（直接返回 `GDBus.Error:org.freedesktop.DBus.Error.NotSupported:
  ReloadExtension is deprecated and does not work`），没有任何热重载手段。于是"磁盘上是新版、
  跑着的是上次登录加载的旧版"是一个真实且长期存在的状态：`ShellExtensionStatus.stale`
  （`running < EMBEDDED_PROTOCOL_VERSION`）就是它，设置页据此显示 "Update pending"
  而不是"已就绪"，`request_screenshot` 也据此在协议低于 `SCREENSHOT_PROTOCOL_VERSION`
  时明确报错、让上层回退，而不是去调一个不存在的方法（`place_window` 同理，门槛是
  `PLACEMENT_PROTOCOL_VERSION`）。
  **加新方法时记得同时抬 `PROTOCOL_VERSION`（extension.js）、`EMBEDDED_PROTOCOL_VERSION`
  （shell_extension.rs）与 metadata.json 的 `version`；抬完之后已经登录的会话必然是
  `stale`，设置页会显示 "Update pending"，用户注销一次才生效——这是预期行为，不是 bug。**
- **只装到用户目录**（`$XDG_DATA_HOME/gnome-shell/extensions/`），不进 deb 的
  `/usr/share/gnome-shell/extensions/`。四个原因：root 的 postrm 清不掉每个用户的目录与 dconf；
  AppImage 根本没有安装钩子；只有 Ubuntu 会自动启用系统目录里的扩展（上游 GNOME 不会）；
  同一个 uuid 出现在两个位置会互相遮蔽。
- **安装只能由用户在设置页显式点击触发**，应用绝不擅自往用户的 GNOME 里塞扩展。
  启动时只做 `reconcile_on_startup()`：内容过期就静默改写、目录还在但 uuid 被移出
  `enabled-extensions` 就补回去、目录被手工删掉就清掉孤儿条目——三件事都不会"帮用户装上"。

#### 令牌与威胁模型

窗口标题会泄露用户正在做什么，所以 `GetWindows` 不对本机所有进程开放：调用方必须出示
Clippy 写在扩展目录里的令牌文件内容（32 字节随机数的 hex，`0600`，由
`private_files::write_private` 落盘），不匹配就抛 `AccessDenied`。`Screenshot` 用同一个令牌
把关——一整屏画面至少和窗口标题一样敏感。

这条边界挡得住谁、挡不住谁，说清楚：

- **挡得住**：其他用户（文件是 0600）；以及沙箱应用（Flatpak/Snap）——它们通常有 session bus
  但没有 `$HOME` 的读权限，因此拿不到令牌。
- **挡不住**：同一个用户的普通进程。它读得到那个文件，也就调得通。同用户之间本来就没有安全边界
  （那样的进程直接读 Clippy 的数据库更省事），所以这是有意接受的，不假装解决。
- **服务端不可伪造**：`org.gnome.Shell` 这个 bus name 只有 gnome-shell 能占，
  没有第三方进程能冒充它来骗走令牌。
- 被否掉的替代方案：按 sender pid 查 `/proc/<pid>/exe` 白名单。AppImage 挂在随机的
  `/tmp/.mount_XXXX` 下、dev 跑的是 `target/debug`，白名单没法维护。
- **`PlaceWindow` 的 `pid` 参数是限定作用域，不是安全边界**。`wrapJSObject` 的同步方法拿不到
  sender 凭据，所以调用方报什么 pid 扩展核实不了——它的作用是"只动调用方自己的窗口"这层意图，
  真正的边界还是令牌，而持有令牌的进程本来就能截整屏。挪一个窗口的位置比那个轻得多。

两侧的契约（uuid、令牌文件名、`MIN_TOKEN_LENGTH`、协议版本、接口名与对象路径）由
`shell_extension.rs` 的 `embedded_extension_matches_the_uuid_and_token_contract` 单测钉住：
它直接在内嵌的 JS 文本里 grep 这些字面量，因此两边不会悄悄漂移。扩展的语法与清单由
`scripts/check-gnome-extension.sh` 在门禁里检查——写错一个字要等到用户注销重登、
gnome-shell 加载失败才暴露，而报错只进 journal。

#### 一次性提示

首次在 GNOME Wayland 上截图且扩展没在应答时，覆盖层给一条能照着做的提示
（`capture.windowProbeHint`：去设置 → 截图安装并注销一次），而不是只说"用不了"。
判据是扩展有没有应答 D-Bus，不是文件在不在——装完还没注销同样不可用，这时恰恰最需要提示。
提示只出现一次，由 `AppConfig.capture_probe_hint_shown` 记住（`capture/mod.rs::take_probe_hint`
取一次就消耗掉并落盘）：没有这个服务照样能自由框选，反复提示纯属打扰。

两个必须做的修正：

- **客户端矩形 ≠ 用户看到的窗口。** GTK 的 CSD 把阴影算在窗口里，`_GTK_FRAME_EXTENTS`
  （回退 `_NET_FRAME_EXTENTS`）给出四边要减掉的边距，不减就会得到一个比窗口大一圈、
  边缘全是透明阴影的候选区。见 `window_probe.rs::trim_frame_extents`。
- **小窗口不做候选。** 小于 `MIN_CANDIDATE_SIZE = 20` 逻辑像素的矩形点不准，也挡不住误命中，直接丢掉。

## 3. 冻结帧走哪个后端

`screenshot/backends.rs::capture_all_monitors` 在 Wayland 上按这个顺序试，**顺序是结论，不是偏好**：

1. **自带 GNOME Shell 扩展**（`Screenshot`）。GNOME Wayland 上的首选。
2. **wlroots**（`libwayshot-xcap`）。sway/Hyprland 用这条。
3. **xdg-desktop-portal**（非交互 `Screenshot`）。非 GNOME 的桌面用这条。
4. **`org.gnome.Shell.Screenshot`**（不带扩展时的 D-Bus 直调，白名单外一律拒绝，基本只是聊胜于无）。
5. **xcap**。最后兜底。

为什么扩展要排在 Portal 前面——这是一个被真实 bug 逼出来的顺序，别改回去：

- **Portal 的非交互截图在"快捷键触发"这个场景下根本不可能成功。**
  xdg-desktop-portal 第一次给某个 app id 截图前要弹一次系统授权对话框，而 gnome-shell
  只允许**当前聚焦的应用**弹它。截图是全局快捷键触发的，此时 Clippy 没有聚焦窗口
  （`overlay_windows::hide_sources` 刚把它们藏起来），于是必然拿到
  `GDBus.Error:org.freedesktop.DBus.Error.AccessDenied: Only the focused app is allowed to
  show a system access dialog`。这条实测结论写在 `request_portal_screenshot` 的文档注释里，
  不要再去查一遍。
  顺带一提，权限是按 app id 存的，而 app id 来自 systemd scope（dev 跑在
  `app-code-*.scope` 时 app id 就是 `code`），所以"我记得授权过"跟当前进程能不能用是两件事。
- **`interactive = true` 的回退已经删掉了。** 它在 GNOME 上会拉起 **GNOME 自己的截图 UI**，
  用户按 Clippy 的快捷键却看到系统截图界面，这比干净地失败糟糕得多。
- **xdg-desktop-portal-gnome 会把每张非交互截图落进用户的图片目录**
  （`~/图片/Screenshot-N.png`），返回的文件归调用方处置。所以 Portal 这条路和扩展这条路
  都用 `TemporaryScreenshotFile` 包住返回路径，无论解码成功还是失败，出了作用域一定删掉。
  历史上漏了这一步，攒出过几十个残留文件。

**所有阻塞 D-Bus 调用都必须走 `dbus.rs`。** ashpd 打开了 zbus 的 `tokio` feature，于是
`zbus::blocking` 内部用一个静态多线程 runtime 做 `block_on`。实测三种线程：tokio async
worker（`#[tauri::command] async fn` 的函数体）上**必然 panic**
（`Cannot start a runtime from within a runtime`），`spawn_blocking` 线程侥幸能过，普通线程正常。
截图链路两边都有调用方——`request_screenshot` 在 `spawn_blocking` 里，而 `hint_needed()` 与
`probe()` 是被 async 命令体直接调的——所以靠"记得别在 async 里调"守不住，统一由 `dbus.rs`
先跳一条干净的 OS 线程。`dbus::tests::blocking_calls_survive_inside_an_async_task` 把两个方向都钉住了。

扩展这条路的其它约定：

- 扩展调 `Shell.Screenshot.screenshot(false, stream, cb)`，拍的是**整个 stage**（多屏合成一张图），
  因此沿用现成的 `split_portal_screenshot` 切成每屏；几何仍旧自己枚举，扩展只负责画面。
- 不含光标（冻结帧是覆盖层的底图，烧进一个光标只会碍事），不闪白
  （那道闪光是 gnome-shell 自己的 ScreenshotService 加的，不是 `Shell.Screenshot` 的一部分），
  不写图片目录。
- 文件落在 `$XDG_RUNTIME_DIR/clippy-shots/` 下，`Gio.FileCreateFlags.PRIVATE`（0600）。
  Rust 侧 `validate_screenshot_path` 只接受父目录正好是这个目录、扩展名是 `png` 的路径——
  读完就要删，不能删错地方。

### 3.1 从快捷键到覆盖层出现，时间花在哪

"截图要三五秒"这种报障只能靠分段计时定位：链路每一环都返回 Ok，问题是加起来太久。
`CaptureManager::reveal` 每次会话都打一条汇总日志（`RUST_LOG=clippy_lib=info` 起效，
见下面那条坑），逐段更细的数字用 `cargo test --lib capture_stage_timings -- --ignored --nocapture`。

本机实测（dev 构建，单屏 2560×1600 物理 / 1920×1200 逻辑，GNOME 50.1 Wayland）：

| 段 | 优化前 | 优化后 | 说明 |
|---|---|---|---|
| 冻结帧 | 558 ms | 550 ms | 其中扩展的 `Screenshot` D-Bus 往返 523 ms、读文件 1.4 ms、Rust 解 PNG 32 ms |
| 窗口候选 | 4 ms | 3 ms | `shell_extension::probe()` 4.2 ms + 相交计算。**报障猜的"获取窗口导致变慢"不成立** |
| 建窗 + webview 冷启动 | 209 ms | 243 ms | 覆盖层是隐藏建窗的，这段用户看不到白屏 |
| 后端交付底图 | 225 ms | ~2 ms | 优化前是 PNG 编码 215 ms + base64 7 ms；现在只剩一次 16 MB memcpy（`InvokeResponseBody::Raw` 要 `Vec<u8>`，帧还得留在会话里给选区翻译用），见 §3.2 |
| 前端解码 + 绘制 | 453 ms | 108~156 ms | 优化前含 3 MB JSON 传输 + `atob` + WebKit 解 PNG；现在是一次 `putImageData` |
| **总计** | **1449 ms** | **901~957 ms** | 两次独立测量 |

那 523 ms 的 `Screenshot` 往返**是地板，不要再去优化它**：gnome-shell 在自己进程里把
stage 编码成 PNG 才把文件路径交回来。`strings /usr/lib/gnome-shell/Shell-*.typelib` 里
JS 能碰到的像素导出只有 `screenshot` / `screenshot_area` / `screenshot_window` /
`composite_to_stream` / `screenshot_stage_to_content`——全都要么落 PNG，要么给一个不透明的
`Clutter.Content`，扩展这一侧没有拿到原始像素的路。

还剩下的可选优化（**没做**，因为有真实风险）：把覆盖层窗口挪到拍照**之前**创建，
让 243 ms 的 webview 冷启动和 550 ms 的拍照重叠。代价是建窗时还没有冻结帧，几何只能另找
一处来源——而"两处几何来源"正是历史上截出黑图那一类 bug 的成因（见 §4）。真要做的话
先把几何统一到一个函数里。

顺带一条坑：`env_logger::init()` 默认只放行 `error`，于是所有 `log::info!`/`log::warn!`
（包括"覆盖层超时未报告首帧"）都是写给空气的。`lib.rs` 现在把默认过滤器设成
`clippy_lib=info,warn`，`RUST_LOG` 仍然优先。

### 3.2 冻结帧像素怎么送进覆盖层

**像素不走 JSON。** `get_capture_overlay` 只给几何与窗口候选，底图由
`get_capture_frame` 单独交付：后端 `tauri::ipc::Response` 直接回原始字节，前端拿到
`ArrayBuffer` 后 `new ImageData(...)` + `putImageData` 铺进一块离屏 canvas
（`react/annotation/frameImage.ts`），那块 canvas 就是 `drawImage` 的源。

**两个请求同时发出。** 覆盖层的 `payload` effect 与 `frame` effect 都只依赖窗口 label，
互不等待；铺画布放在第三个 effect 里（依赖 `[frameBuffer, payload]`），等两边都到齐才做。
像素是这两次里慢的那个（16 MB），串起来等于把它的往返白加在覆盖层出现之前——
而覆盖层出现之前用户什么都看不到。**不要为了"拿到尺寸再要像素"把它们改回串行。**

契约：**RGBA8、行优先、无 padding，尺寸取 payload 的 `pixelWidth`/`pixelHeight`。**
原始字节没有自校验的文件头，所以前端必须核对 `length === 4 × w × h`，不对就报错——
拿错位的像素铺满全屏比干净地失败糟糕得多（`rgbaToFrameCanvas` 的尺寸校验和
`capture-overlay-app.test.js` 里那条截断帧的用例钉住了这一点）。

曾经的做法是 payload 里带一个 `pngBase64`，代价是同一张图要被处理四次：Rust 编 PNG
（215 ms）→ base64（3 MB 字符串）→ webview `atob` → WebKit 解 PNG。原始 RGBA 虽然字节数
更大（16 MB vs 2.2 MB），但两头都是零编码，实测总时长反而少一半。**别改回字符串。**
标注渲染层为此把底图类型放宽成 `FrameImage = HTMLImageElement | HTMLCanvasElement`
（图片编辑器与 Pin 窗口仍用 `<img>`），尺寸统一走 `frameWidth`/`frameHeight`
而不是 `naturalWidth`。

另外，隐藏自己的窗口之后那 140 ms 的合成器落定等待（`HIDE_SETTLE_MS`）现在**只在真的藏了
窗口时才等**。快捷键截图的常态是面板本来就没开着，`hide_sources` 返回空列表，这时白等
140 ms 纯粹是加在感知延迟上的。面板开着时这一等仍然必要，少了它会把 Clippy 自己的面板
烧进冻结帧。

## 4. 三套坐标空间

| 空间 | 谁在用 | 单位 |
|---|---|---|
| 逻辑像素 | 覆盖层 DOM、选区、`WindowCandidate` | 桌面逻辑尺寸（本机 1920×1200） |
| 冻结帧物理像素 | `FrozenFrame.rgba`、标注、`exportPngBase64` 的裁剪矩形 | 帧的真实宽高（本机 2560×1600） |
| X screen 像素 | `xcap::Window::x()/y()/width()/height()` | XWayland 的 X 屏（本机 3840×2400） |

无缩放的 X11 会话里三者恰好相等，所以这段代码长期看着是对的；一有缩放就错得离谱——
本机 scale 1.3333 时一个普通 QQ 窗口被报成 2598 像素宽，比整个逻辑桌面还宽。

**先判定坐标空间，再决定要不要换算。** 这一层由 `screenshot/geometry_check.rs::classify_stage`
负责，结论只有三种：`Logical`（舞台图 = 逻辑并集 × max(scale)，几何可信，**一个字都不改**）、
`Physical`（并集就是图本身而显示器自称有缩放，xcap 在 XWayland 上就是这样，按最大缩放反推）、
`Unknown`（两条都不像——枚举漏屏、热插拔后的陈几何、单位错了。这时**放弃修正**并
`log::error!`，因为任何修正都是在错上加错）。以前没有这一步：无条件调用修正函数，
靠"差值 ≤ 1 像素就提前返回"当护栏——混合缩放的多屏正好能绕过它，这就是下面那个 1.125 倍的来源。

换算规则，改代码时不要另立一套：

- **显示器逻辑尺寸 = round(帧像素 / scale_factor)**（`screenshot/backends.rs::normalize_monitor_geometry`）。
  xcap 0.9.6 的 `Monitor::width()` 返回的是 `RandR 像素 ÷ scale_factor`，在本机上给出 2880×1800
  这种既不是逻辑尺寸也不是物理尺寸的数；不归一化的后果是覆盖层里的图"没有正确缩放"。
  原点按同一比例缩放，容差 1 像素。
- **上面那个 `scale_factor` 是谁的缩放，取决于帧像素是谁的。** 两个调用点不一样：
  `capture_all_xcap_monitors` 逐屏抓图，帧宽就是这块屏自己的物理宽度，除数是这块屏自己的缩放；
  `split_portal_screenshot` 切的是一整张舞台图，除数必须是 `desktop_max_scale_factor`。
  Mutter 抓整屏时把舞台按**各视图里最大的**缩放渲染成一张图
  （`clutter_stage_get_capture_final_size`），所以舞台图 = 逻辑并集 × max(scale)，
  低缩放的屏在图里是被放大过的，它自己的缩放和舞台倍率不是一回事。
  混用过一次，代价是：HDMI 2560×1440@1.5 + 笔记本 1920×1200@1.3333 的组合下，
  笔记本被改写成 2160×1350@(2880,459)（1.5/1.3333 = 1.125 倍），于是覆盖层比屏幕大一圈、
  窗口候选整体左上偏移、右下工具条落到屏幕外、画面溢到隔壁显示器上。单屏时自己的缩放
  就是最大缩放，这个错误恰好是恒等变换，所以只在插上第二块屏时暴露
  （`tests/fixtures/monitor-layouts/gnome-dual-mixed-scale.json` 钉住，见 §4.1）。
- **旋转屏的缩放必须先换轴再相除。** Wayland 的 `physical_size` 是面板自己的原始分辨率
  （不含旋转），而 `logical_region` 已经是旋转后的桌面坐标。竖着摆的 1920×1080 面板于是
  报成 physical 1920×1080 + logical 1080×1920，直接除得到宽比 1.7778、高比 0.5625，
  取谁都是废数，而这个值会一路传成帧缩放和坐标换算的除数。
  `geometry_check.rs::output_scale_from_sizes` 拿 `wl_output` 的 `transform` 判断要不要
  换轴（八个取值里只有 90/270 及其镜像版本换），换完两个方向都是 1.0。
- **wlroots 那条路还得自己把像素转过来。** `libwayshot` 的 `screenshot_single_output`
  直接把 frame copy 转成图，`transform` 一个字都不用（会转的是它自己的
  `screenshot_outputs` 合成路径）。逐输出抓图的我们于是拿到面板原始朝向的横向像素：
  覆盖层里画面躺倒、帧宽高和逻辑矩形反着、选区坐标全错。`apply_output_transform`
  照抄上游 `image_util::rotate_image_buffer` 的方向约定（`_90` → `rotate90`，
  Flipped\* 先水平翻再转）——**方向搞反在横屏上永远看不出来**，所以有一条逐像素的单测
  钉着"左上角的红点转 90° 之后必须在右上角"。
  **舞台图那一侧不需要管旋转**：旋转已经被合成器烤进桌面了，旋转屏的逻辑矩形和裁剪
  都是旋转后的，所以 I3 对正常竖屏不该响（`rotated-portrait-secondary.json` 钉住）。
- **镜像屏（投影）的裁剪完全相同，那是配置本身如此，不是几何错。**
  `geometry_check.rs::find_mirror_sources` 先把"裁剪和前面某块屏**完全一致**"的屏摘成
  镜像：I2a 于是只剩下**部分**重叠这一类（已拔掉的屏、热插拔前的陈几何），没有任何正常
  配置能解释它，报出来才有指向性——以前两者都走 I2a，等于一接投影仪就报几何错。
  摘出来还有个实惠：镜像屏直接共享源屏那份 `Arc<[u8]>`，1080p 省掉一次 8 MB 的拷贝。
  镜像关系写进 `StageTile::mirror_of`、摘要行的 `（镜像自 #1）` 和 fixture 的 `mirrorOf`
  ——**必须单独钉住**，光钉"I2a 不报"的话，把镜像识别整段删掉那条测试照样过。
  覆盖率也只算非镜像的，否则同一块面积数两遍会算出 200%。
- **覆盖层绝不能比显示器大，这一层不靠上游算对。** 全屏请求不保证窗口被压到显示器尺寸：
  内容的最小尺寸比显示器大时，合成器给了 fullscreen 状态、GTK 仍按内容尺寸分配，
  窗口于是居中摆放并溢到隔壁屏上。`overlay_windows.rs::configure_platform_overlay`
  因此拿 GDK 的显示器几何和帧几何对一遍，不一致就按显示器尺寸 `resize` 并 `log::warn`；
  前端同时按**可见视口**（`window.innerWidth/innerHeight`）而不是帧逻辑尺寸给工具条和
  译文面板落位。两层兜底都只是让几何算错退化成"画布边缘被裁掉"，真正的修法仍是上一条。
  这个实测视口还会跟着首帧握手回到后端做自检，那是全链路里唯一的闭环，见 §4.1 的 I4。
- **窗口矩形先按 `X screen 像素 / 逻辑像素` 折算**（`window_probe.rs::x11_pixel_ratio`，
  比值钳在 1.0..=4.0），再和帧的逻辑边界求交。
- **选区在逻辑空间，标注在帧像素空间。** 前端 `scale = logicalWidth / pixelWidth`，
  `useCanvasInteractions.pointFromEvent` 把客户端坐标除以 `scale` 得到帧像素坐标；
  导出时用 `geometry.ts::toPixelRect(selection, 1/scale, 1/scaleY, frame)` 把选区换算回帧像素并钳进帧内。
- **选区坐标是相对覆盖层的，不是桌面全局的。** 覆盖层铺满一块显示器，它的 (0,0) 就是那块屏的
  左上角。要拿到"屏幕上的哪一块"必须加 payload 的 `logicalX`/`logicalY`（来自
  `CapturedMonitorFrame.x/y`）——贴图回原位就是这么算的（`App.tsx::originRect` →
  `pin::PinOrigin`），多屏时少加这一步会把第二块屏的选区贴到主屏上。
- **贴图窗口位置 = 原始矩形 − `SHADOW_GUTTER`（12 逻辑像素）。** 要盖住原处的是**内容区**
  而不是窗口左上角，而内容区相对窗口原点偏移 12 像素（`pin.css` 的
  `.pin-media { inset: 12px 56px 60px 12px }`）。这一份契约横跨 CSS 与
  `pin/window.rs::outer_size`/`pin_target_position`，改一处必须改另一处，
  `window_origin_offsets_the_content_area_by_the_shadow_gutter` 这条单测钉着它。
- **Tauri 的显示器几何是物理像素，扩展给的是逻辑像素。** 混合缩放的多屏上不能用一个系数
  换算整个桌面，要逐屏折算再挑包含目标点的那块（`pin/window.rs::logical_work_area`）。

### 4.1 几何自检：让算错的环境自己报出来

上面那个 1.125 倍的 bug 有个很难受的性质：**它在开发者的机器上是恒等变换**。单屏、
等比缩放、纯 X11 会话全都碰不到它，只有"混合缩放的多屏"这一种组合会暴露。
显示器配置的组合是无穷的（屏数 × 缩放 × 排布 × 镜像 × 旋转 × 合成器），
挨个测既做不到也没意义，所以换一个思路：**利用这份数据本身是过定的**。

同一份显示器几何同时决定三件事——逻辑并集、舞台图尺寸、每块屏的裁剪矩形。
它们互相印证，任意两个对不上就说明有一个是错的，而**判断这件事不需要知道用户的环境**。
纯函数都在 `screenshot/geometry_check.rs`：

| 不变量 | 内容 | 谁来查 |
|---|---|---|
| **I1** | 舞台图 / 逻辑并集必须是一个各向同性的比例，且等于 `max(scale)` 或 1.0 | `classify_stage`，见 §4 开头 |
| **I2a** | 各屏裁剪矩形**互不重叠**（镜像屏先摘走，剩下的部分重叠 = 陈几何/已拔掉的屏） | `find_mirror_sources` + `verify_crops_do_not_overlap` |
| **I2b** | 裁剪没有被图像边界**静默夹掉**（`scaled_monitor_rect` 会悄悄钳进图内） | `verify_crop_not_clamped` |
| **I3** | 每块屏的 `裁剪宽/逻辑宽` 与 `裁剪高/逻辑高` 必须相等（不等 = 裁剪朝向和几何对不上；**正常竖屏两边都是竖的，不该响**） | `verify_frame_isotropy` |
| **I4** | 覆盖层的**实测可见视口** == 后端算给它的逻辑尺寸 | `capture/manager.rs::viewport_mismatch` |
| **I5** | 扩展报出的"Clippy 自己那个窗口" == Tauri 自己知道的同一个窗口的逻辑外框 | `capture/diagnostics.rs::own_window_ratio`（只在诊断里跑） |

**I2 不是"裁剪必须铺满舞台图"**——非矩形的显示器并集天然留白（本机的排布只覆盖 83%），
拿铺满当判据会对着一个完全正常的桌面报错。覆盖率因此只是 `StageSplitPlan.coverage`
里一个**给人看的数字**，不参与判定。

**I4 与 I5 是两条闭环，闭的是两件不同的事。** I1–I3 都在后端内部自证一致，
只有这两条拿到了链路另一端的实测值：

- **I4：覆盖层被摆成多大。** 前端在首帧握手（`mark_capture_overlay_ready`）里捎上
  `window.innerWidth/innerHeight`，后端和自己下发的逻辑尺寸对一遍。那次真实事故里
  这一对是 `(2160,1350)` vs `(1920,1200)`，差值 `(-240,-150)`——有这条日志的话，
  报障第一句话就是答案。
- **I5：窗口候选画在哪。** 覆盖层尺寸对了，窗口矩形照样可能整体偏移或缩放（`window_probe.rs`
  那条 X11 像素比就是同一个 bug 类的第二个案发地）。两边唯一都能看到的参照物是
  **Clippy 自己的窗口**：扩展按 pid 报出它，Tauri 也知道它的逻辑外框，两个数字必须一致。
  同样不需要知道用户有几块屏、怎么缩放。

**I5 按比例判，不按像素差判。** 扩展给的 `frame_rect` 不含 CSD 阴影，而 Tauri 的
`outer_size` 含不含随 GTK 版本而异，两者天然差一圈几十像素。按像素判会天天误报，
那样的检查很快就没人看；按比例判（容差 8%）只在真的错了坐标空间时才响，
而它要抓的正好是 1.125×／1.5×／2× 这个量级。

**所有不变量失败都只记日志，绝不中断截图。** 几何算错的后果是"画布边缘被裁掉"，
而硬失败的后果是"用户截不了图"，后者糟得多。

#### 一个 json 就是一个环境、一条回归测试、一个 PR

为此把切舞台图这条路拆成了两半：`plan_stage_split` 是**不碰像素**的纯函数
（输入显示器列表 + 舞台图尺寸，输出 `StageSplitPlan`），`split_portal_screenshot`
只负责照着计划从 RGBA 里抠像素。于是新增一种显示器配置的成本是**一个 json 文件**：

```
src-tauri/tests/fixtures/monitor-layouts/*.json   ← 环境即数据
src-tauri/src/screenshot/layout_fixtures.rs       ← 唯一那条参数化测试
```

每个 fixture 写清 `session`（合成器 + 后端）、`stage`（舞台图尺寸）、`monitors`
（逐屏矩形与缩放）和 `expect`（分类结果、应当触发的不变量标签、逐屏修正后的矩形与裁剪）。
测试直接驱动**真正的** `plan_stage_split`，不是复制一份算法；因为不需要像素，
也就不用为多屏造几十 MB 的假图（历史上那两条测试各分配 64 MB）。

这条设计的实际意义是：**"收到一份报障"和"补一条回归测试"可以是同一件事**。
用户跑诊断（见 §4.2 的 `--emit-test-case`）拿到的就是这个格式，
把文件丢进目录、PR 就完整了，不用写一行 Rust。当前 9 个 fixture 覆盖了
混合缩放双屏（那个真实 bug）、XWayland 的假逻辑尺寸、单屏无缩放、整数缩放、
镜像输出、热插拔后的陈几何、负坐标纵向排布、三屏混合缩放、竖屏旋转副屏。

排障从 `CaptureManager` 那条汇总日志开始：`StageSplitPlan::summary_line` 打出舞台分类、
覆盖率和逐屏的 `#{id}@{x},{y} {w}x{h}×{scale}→{裁剪}`。这行里**没有窗口标题、没有像素**，
和扩展令牌是同一套威胁模型（§2.1），所以可以直接贴进 issue。

### 4.2 诊断报告：把"报障"和"补一条测试"变成同一件事

上面那套不变量解决的是"环境自己报出算错了"，但结论落在**日志**里——而报障的用户既不知道
日志在哪，也不知道该抄哪几行。中间这一步断掉，前面的自检就等于没有。所以有一个入口
把那些数字一次性摊开（`capture/diagnostics.rs` + `screenshot/diagnostics.rs`）：

| 入口 | 用法 | 场景 |
| --- | --- | --- |
| 设置页 | Screenshot → Run Diagnostics → Copy Report | 常规 |
| 命令行 | `clippy --capture-diagnose` | **几何算错时 GUI 本身就不可信**：覆盖层错位、面板跑到隔壁屏，这时候让用户去点设置页是在最不合适的时候要求他操作图形界面 |
| 命令行 | `clippy --emit-test-case > .../monitor-layouts/xxx.json` | 只吐 fixture json，直接重定向进目录 |
| 环境变量 | `CLIPPY_CAPTURE_DIAGNOSE=1` | 从桌面图标启动、加不了参数时 |

命令行这条路**在 Tauri 起来之前就结束**（`main.rs` 里先于 `clippy_lib::run()`）：
不建窗、不抢 single-instance 的 D-Bus name，所以用户正在用的那个实例不会被顶掉。

报告分五段：会话环境（桌面 + 会话类型 + `WAYLAND_DISPLAY`/`DISPLAY` 在不在）、
扩展状态、逐来源的显示器几何、舞台图与切分、不变量自检表，最后是 `monitor-layout`
段落——它**就是** §4.1 那个 fixture 格式。

几个刻意的选择：

- **"未检查"和"PASS"分开写。** 拿不到舞台图时 I1–I3 写的是"未检查（拿不到舞台图）"，
  没截过图时 I4 写的是"未观测"。把没查过写成通过，会让排障的人绕开唯一那条闭环自检，
  比不给结论更糟。为此 `CaptureManager::last_viewport` 刻意活得比会话长：用户总是
  "截完图发现界面错位"之后才去点诊断，那时会话早就结束了。
- **I5 只能长在这里，不能长在截图链路里。** 它需要一个"两边都看得见"的参照物，
  而截图前 `hide_sources` 已经把 Clippy 自己的窗口藏了——藏起来的窗口既不在扩展的
  列表里、也没有可信的几何。按需诊断反过来是它最舒服的时机：用户正开着设置页。
  配对条件是**两边各只有一个自己的窗口**（扩展只给 pid，同一 pid 下的多个窗口分不出
  谁对谁），凑不上就写"未检查"并说明原因，而不是猜一个来配。这也决定了命令行入口下
  I5 恒为"未检查"：那条路上根本没有窗口。
- **几何来源逐个列，不合并。** `wl_output` 和 `xcap`（XRandR/XWayland）对同一套屏幕
  的说法不一致本身就是结论（§4 的 `Physical` 那一类）。合并成一份就把它藏掉了。
- **第三个来源是 Tauri/GTK 自己那套显示器模型，它和前两个不是同级的。** 覆盖层与贴图
  窗口是把逻辑矩形交给 Tauri 去摆的，**最终落在哪、多大，取决于这一套**，不是我们枚举
  出来的那套。两套对不上时症状正好是用户报的"覆盖层偏了一块"，而 I1–I3 会全过——
  它们只在我们自己的数据内部对账。I4 能抓到后果（视口对不上）却指不出成因，
  所以三套并排列出来，走偏的是哪一套肉眼可见。三个来源共用同一个排版与求并集的函数
  （`describe_reported_monitors`），否则差异可能来自算法而不是数据，这份对比就白做了。
- **顺手预测舞台图尺寸。** 并集那一行写的是 `逻辑并集 × max(scale) = 预期舞台图`，
  而下一段是实测的舞台图尺寸——两个数字并排放着，I1 是不是过了肉眼就能看出来。
- **不含像素。** `probe_stage_image_size` 用 `image::image_dimensions` 只读 PNG 头，
  临时文件当场删掉。
- **不含窗口标题。** 窗口候选整段不进报告。标题泄露用户正在做什么，和扩展 `GetWindows`
  那个令牌是同一套威胁模型（§2.1）。
- **绝不自动上传。** 只写本机缓存目录（`capture-diagnostics.txt`，固定名字每次覆盖），
  报告原样显示给用户；"Report an Issue" 是**先复制到剪贴板再打开 issue 模板**，
  不把报告塞进 URL——它有两三千字符，URL 长度限制会静默截断，而被截断的诊断报告
  看起来齐全、数字却少了一半，比没有报告更糟。

`screenshot/layout_format.rs` 是这条链路的关键一环：fixture 的 serde 结构体**不在测试里**，
诊断的输出侧和回归测试的输入侧共用同一组定义，期望值两边都走
`Expect::from_plan`。于是"诊断吐出来的东西一定能被那条测试接受"是结构上成立的，
不靠人去核对——`the_diagnostic_emits_exactly_what_the_fixture_test_asserts` 把这点钉住了。
用户侧的操作步骤见 [CONTRIBUTING.md](../CONTRIBUTING.md)。

### 4.3 热插拔与已知边界

**没有监听显示器变化的代码，这是刻意的。** 整条链路上不存在缓存的显示器几何：
每次按快捷键都重新枚举（`capture_all_monitors` → `enumerate_wayland_monitors` /
`enumerate_xcap_monitors`），覆盖层窗口按当次结果新建、用完销毁；窗口候选每次会话重新问
（扩展的 `GetWindows` 是实时查询，扩展自己也不存显示器列表）；贴图与主面板每次摆位都现问
Tauri（`available_monitors` / `monitor_from_point`）。所以"插上外接屏之后要不要刷新"
这个问题在这个架构里不存在——下一次截图拿到的必然是新几何。唯一常驻的缓存是扩展的
**摆窗令牌探测结果**（`shell_extension::placement_token`），它和显示器无关。

代价是每次截图多付一次枚举（实测在冻结帧那 550 ms 里占不到 5 ms），换掉的是一整类
"配置变了但缓存没变"的 bug——那类 bug 的症状和几何算错完全一样，却查不到算式上。

仍然存在的边界，写在这里免得当成 bug 反复查：

- **会话进行中拔插屏不跟随。** 覆盖层已经按旧几何建好了，中途插拔不会重建。
  会话只有几秒，且 Esc 一按就重来，所以不做；真撞上时 I4 会把它记成视口不一致。
- **X11 那条窗口枚举只有一个全局像素比。** `x11_pixel_ratio` 是"X screen 根窗口宽 ÷
  逻辑并集宽"，各屏缩放不同时压根不存在这样一个数，缩放不等于该比例的屏上速选框会偏。
  这不是能修的算式错误——XWayland 不提供逐屏缩放，信息不够。所以只做识别：
  `frame_scales_are_uniform` 认出混合缩放并打一条 warn，指向真正的解法
  （GNOME Wayland 装扩展直接绕开这条路）。
- **裁剪来自扁平的冻结帧**，速选被部分遮住的窗口会带上遮挡者的像素（见 §5）。

## 5. 快速选区（hover → click）的交互约定

1. **下发数组即堆叠顺序，索引 0 是最上层**；前端 `windowAt` 取第一个命中的候选，
   因此重叠时选到的就是肉眼看到的那个窗口，被完全遮住的窗口自然选不到。
   两条路各自怎么拿到这个顺序：扩展侧用
   `global.display.sort_windows_by_stacking(...)` 再 `reverse()`；X11 侧读
   `_NET_CLIENT_LIST_STACKING`（协议规定自下而上，所以同样要反过来），
   枚举不到的窗口沉到最底（`window_probe.rs::order_x11_candidates`）。
   **不要按面积排序。** 早期版本按"面积小的在前"来近似遮挡关系，一个大窗口压在小窗口上时
   它给出的答案与肉眼恰好相反；`without_stacking_order_smaller_windows_win` 只是拿不到
   堆叠顺序时的退化兜底，不是主路径。
   已知的取舍：裁剪始终来自那张扁平的冻结帧，所以速选一个被部分遮住的窗口，
   结果里会包含遮挡者的像素——这正是用户当时看到的画面，与 Snipaste / flashot 的行为一致。
2. 鼠标移动时 `hoverCandidate` 只在选区**外面**给高亮预览：选区内部要让位给移动/缩放手势，
   否则随手框过一次之后窗口速选就再也用不上了。
3. 按下到松开的位移小于 `CLICK_SLOP = 4` 逻辑像素算"点击"：
   停在某个窗口上就取那个窗口，停在空地上就取**整个显示器**（参考项目 flashot 的手感）。
   位移超过 slop 就是拖拽，原样落地并钳进屏幕边界；面积不足 2×2 作废。
4. 点击或拖拽**都不结束截图**：工具条贴到选区旁边，选区仍可拖动与缩放，
   点对钩才把"裁剪 + 标注"后的 PNG 提交。铺满全屏的选区靠 `coversBounds` 让内部拖拽回到重新框选。
5. 右键丢掉选区回到 idle，Esc 取消整个截图。

## 6. 真实桌面上仍需人工验收的部分

单元测试覆盖到几何换算、状态机和提交合同，但覆盖不到合成器行为。以下必须在真机上看：
覆盖层是否铺满**当前**显示器（多屏、混合缩放）、Portal 首次确认与撤权，以及 GNOME Shell
扩展这条链路——**设置页点安装 → 注销一次 → 回来确认状态卡片变绿、悬停有窗口高亮、
按快捷键出来的是 Clippy 的覆盖层而不是系统截图 UI**，另外单独确认 `GetWindows`
带正确令牌能返回窗口、令牌错误时被拒。
这几步没法自动化：gnome-shell 只在登录时加载扩展，CI 里没有会话。
未装扩展的 GNOME Wayland 上还要确认原生 Wayland 应用如预期地不出现在速选列表里。

贴图窗口这一路同样只能人工看（合成器行不行由它自己说了算）：

- **摆位与尺寸**：截一块屏幕区域 → 贴图，贴图应当盖在**刚才选的那块**上，大小一致；
  再从剪贴板历史里 Pin 同一张图，应当还落在同一处（靠 `PinOriginRegistry` 的像素指纹认回来）。
  这一条要在**面板一直关着**的状态下做：复制完不去点面板，直接按 Pin 快捷键。历史上"贴出上一张图"
  就是这条路——面板关着时列表内存被释放，新条目到达后前端焦点被挤到第二行
  （见 `js/clipboard/navigation-state.js::releaseNavigation` 的注释）。先打开面板再 Pin 反而测不出来。
  从别处复制来的图（浏览器、文件管理器）没有位置信息，应当落在屏幕中间——这是正确行为。
- **置顶**：贴图之后点开别的窗口，贴图不应被盖住；缩放一次（滚轮）之后再确认一遍，
  因为改尺寸会把窗口带回普通层。
- **注意协议版本**：`PlaceWindow` 是 v3 才有的，抬过版本号之后当前会话必然是 `stale`
  （设置页显示 "Update pending"），此时摆位与置顶都会退回 Tauri 那套，也就是在 Wayland 上
  **看起来像没生效**。验收这两条之前必须先注销重登。
- **触控板捏合**：在贴图上做捏合手势，页面不应缩放（内容溢出、工具栏错位就是没拦住）；
  滚轮缩放、Shift+滚轮调不透明度仍应正常。
- **拖动不选中**：按住图片拖动窗口，图片不应变成系统强调色的选中块（Ubuntu 上是橙色）；
  文本贴图里的文字仍应能划选。
`src-tauri/src/screenshot/backends.rs` 里留了两个 `#[ignore]` 的诊断测试
（`backend_diagnostics`、`window_probe_diagnostics`），用
`cargo test -- --ignored --nocapture` 跑，会打印每个后端的尺寸、平均亮度、全黑像素比例和窗口矩形——
"截图是黑的"这类问题只能靠它定位，不要删。
