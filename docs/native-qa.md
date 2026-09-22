# 跨平台真机 QA

本手册用于完成跨平台兼容性验收中不能由 Linux 本机或
交叉编译证明的验收项。每份记录必须绑定完整 commit SHA 和实际安装包版本；“能编译”“看起来正常”
或旧版本截图不能代替指定场景的观测证据。

## 1. 先验证原生 CI

推送待测 commit 并等待 GitHub Actions 完成，然后运行：

```bash
node scripts/verify-native-ci.mjs \
  --repo 51hhh/Clippy \
  --sha <40位commit SHA> \
  --output native-ci-evidence.md
```

只有以下三个 job 对同一个 SHA 都是 `completed/success` 才能进入真机验收：

- `Check (ubuntu-22.04)`
- `Native Check (windows-latest)`
- `Native Check (macos-latest)`

Jammy job 执行完整 Rust 与前端门禁；Windows/macOS 原生 job 执行 Rust check/clippy/test，证明平台
条件编译、原生 API 与单元测试成立。安装包由下一步的 Native QA workflow 构建；CI 仍不能证明桌面权限、
焦点恢复、输入注入、混合 DPI 或签名证书链在真实用户环境中工作。

CI 通过后，在 GitHub Actions 中对同一 ref 手动运行 `Native QA Packages`。该 run 的 Artifacts 区会提供
四套以完整 SHA 命名的 QA 安装包、Ubuntu 24 AppImage X11 smoke 证据，以及
`qa-record-templates-<SHA>`。先核对安装包内的 `QA-BUILD.txt` 与 `SHA256SUMS.txt`，再使用同一 run
生成的 JSON 记录；不要把其它 run、旧 SHA 或本地临时构建混入证据。Windows QA 包使用临时自签名，
macOS QA 包仅做 Ad-Hoc 签名；updater 安装必须改用同 SHA 的正式 release 产物。当前正式 macOS release 也
采用 Ad-Hoc 签名，因此只能验证功能和更新链，不能作为 Developer ID、公证或 Gatekeeper 信任证据。

Linux x64 QA 包显式启用 `recording-wayland-qa,recording-linux-av-qa`（同时覆盖 X11 与 Wayland
视频源、PipeWire 默认系统声和默认麦克风），Windows x64 QA 包显式启用
`recording-windows-av-qa`；macOS 12.3+ QA 包显式启用 `recording-macos-av-qa`，Intel 另叠加
`recording-vp9-source-build`。Linux 与 Windows 工具条增加受门控的系统声和麦克风；macOS 13+
增加系统声，15+ 再增加系统默认麦克风。这些包只用于取得录屏原型证据，不代表正式 release
已启用录屏。
安装前必须核对 `QA-BUILD.txt` 的 `recording_feature` 与实际平台一致；macOS 还必须核对
`minimum_system_version=12.3`，否则不能执行下文录屏场景。

## 2. 生成绑定版本的记录

选择目标环境并生成模板：

```bash
node scripts/manual-qa.mjs template \
  --profile windows-11-x64 \
  --sha <40位commit SHA> \
  --version <SemVer> \
  --output windows-11-x64.json
```

支持的 profile：

| Profile | 环境 |
|---|---|
| `linux-gnome-x11` | Ubuntu 22.04 GNOME 42 X11 |
| `linux-gnome-wayland` | Ubuntu 22.04 GNOME 42 Wayland |
| `linux-gnome-wayland-ubuntu24` | Ubuntu 24.04 GNOME Wayland |
| `linux-gnome-wayland-ubuntu26` | Ubuntu 26.04 GNOME Wayland |
| `linux-kde-wayland` | KDE Wayland |
| `linux-wlroots-wayland` | 一个 wlroots compositor |
| `windows-10-x64` | Windows 10 22H2 x64 |
| `windows-11-x64` | Windows 11 x64 |
| `macos-intel` | macOS 12.3+ Intel |
| `macos-apple-silicon` | macOS 12.3+ Apple Silicon |

先补齐 `testedAt`、`environment.osVersion`、实际桌面/架构，再逐项填写：

- `status`：必须等于模板提示的 `acceptedStatuses`；初始 `not_run` 永远不能通过。
- `observedReasonCode`：安全降级场景必须属于 `acceptedReasonCodes`。
- `observation`：写实际发生了什么，不写“同预期”或“应该可以”。
- `evidence`：填写日志、诊断报告、录屏、截图或哈希记录的相对路径/URL。

模板里的期望字段只方便测试人员阅读。校验器以仓库内合同为准，修改 JSON 中的
`acceptedStatuses` 或 `acceptedReasonCodes` 不会放宽验收标准。

完成后生成归档报告：

```bash
node scripts/manual-qa.mjs verify \
  --input windows-11-x64.json \
  --output windows-11-x64.md
```

缺场景、重复场景、未知场景、错误环境、错误 reason code、无文字观测或无证据都会返回非零退出码。

## 3. 所有平台的公共场景

每个平台都必须使用真实系统剪贴板和目标应用完成以下步骤：

1. 分别复制 Unicode 文本、带样式 HTML 和透明 PNG，确认历史记录内容、预览和再次复制一致。
2. 暂停/恢复监听，验证外部复制、自复制后的即时唤醒、相同内容去重、历史上限和收藏免清理；再验证
   Unicode/短词/前缀搜索、分页、删除、清空和设置页统计。
3. 逐一操作列表、搜索、预览、翻译和 codec 面板的键盘状态机，包括方向键、WASD、数字、Enter、
   Space、Ctrl+P、Ctrl+Enter、Tab、Shift+Tab、Esc 和反引号；焦点不能误驱动背后的列表。
4. 用代码、GFM Markdown、恶意 HTML、透明图片以及公网/回环/私网 URL fixture 验证富预览；HTML 必须
   被净化，URL 元数据不得访问本机或私网地址；再覆盖 JWT、可逆编码、哈希/标识符、数学表达式、
   加密内容和大文本的识别与渲染。
5. 在 codec 面板逐项验证 Base64、URL、HTML entities、Unicode、Hex、ROT13、MD5/SHA、JSON、JWT、
   URL Parse、Timestamp 和 Number Base；同时验证智能建议、方向/I-O 交换、收藏、清空和复制结果。
6. 修改主面板、Pin 和截图三个全局快捷键，验证注册、冲突提示、暂停、恢复及重启后保持；不得只点击
   设置界面而不实际触发动作。
7. 截取区域并分别执行 Copy、Save、Pin、Translate 和取消；确认取消没有写文件、复制或创建 Pin。
8. 在截图选区和图片查看器分别扫描 QR Code、Code 39、Code 128、EAN-13；至少覆盖屏幕小码、
   低对比、90°、反色和 QR + 一维码同图，核对格式、文本、从上到下顺序与复制结果。扫描失败不得
   关闭截图或打开 URL，选区改变后旧结果不得回写。
9. 对同一图片依次验证全部 16 种画布工具：裁剪、对象、橡皮、文本，pen/marker/rect/ellipse/
   highlight/arrow/line/measure 八种绘制，以及 blur/mosaic/spotlight/magnifier 四种像素效果；再验证
   调整、撤销/重做、Copy 和扁平导出。
10. 保存可编辑版本，确认落盘文件与剪贴板都只有扁平 PNG；关闭应用后重开，把同一文件拖入主窗口
   或从历史再次 Pin，确认恢复根图与累计操作。对文件改动一个像素后应按普通 PNG 处理。记录根图、
   各修订渲染结果与重开结果的 SHA-256；renderer v2 跨平台比较不能用目测代替摘要。另用旧版
   iTXt v2/v3 fixture 验证兼容导入与保存后迁移。
11. 验证系统 Pictures 目录、自定义目录、重名不覆盖及系统目录不可用时的应用数据目录 fallback。
12. 分别在 Tesseract 可用/不可用状态检查 OCR 与平台提示；Windows/macOS 不得出现 Linux 安装按钮。
13. 使用至少两个已启用翻译服务验证并行结果、自动换向、单服务重试、缓存和删除；图片只允许发送
    本地 OCR 文本，敏感内容必须在网络请求前阻断，并实际播放一次 TTS。
14. 保存翻译凭据并检查系统凭据管理器；模拟 keyring 失败时，不得在配置或日志出现明文密钥。
15. 切换六套主题、自动/中文/英文语言及各设置，重启后确认保持；托盘菜单语言和暂停状态同步，重复
    启动只激活已有实例，主窗口/设置窗口不会生成失控副本。
16. 启用登录启动并完成一次真实登录验证，再关闭并确认系统启动项被清理。Linux 另测 tmux copy-mode
    hook；Windows/macOS 确认 tmux 控件隐藏且没有执行 inotify/nix 命令。
17. 保存 typed 平台能力与截图诊断，确认不包含剪贴板内容、图片像素、窗口标题、Portal token 或密钥。
18. 使用该平台正式安装包完成检查更新、下载、安装和重启；确认安装类型不是被错误识别为 deb。

证据中不得包含真实剪贴板秘密、翻译 token、Portal restore token、完整截图像素或窗口标题。测试内容
使用专门的无敏感 fixture。截图诊断报告已按设计排除像素与窗口标题。

## 4. Linux X11

- 从登录界面明确选择 Xorg，会话内记录 `XDG_SESSION_TYPE=x11`。
- 用普通文本编辑器验证自动粘贴恢复焦点；再分别测试主面板打开/关闭和快捷键触发。
- 使用两块不同缩放/负坐标显示器验证窗口命中、选区边界、Pin 初始位置和拖动。
- 运行 `clippy --capture-diagnose`，保存 I1–I5、typed `PlatformInfo` 和 monitor-layout fixture。
- Pin 后切换工作区、全屏窗口和普通窗口，确认 X11 topmost 行为与工具条状态一致。
- 从同一选区进入长截图，依次验证上下左右自动滚动。每个方向至少追加两帧，按 Stop 与 Esc 均只
  暂停自动循环且保留画布；随后 Copy、Save、Pin 各完成一次。滚动期间移动鼠标、关闭/移动目标窗、
  页面到底以及插入动态内容，分别确认控制窗恢复、自动模式停止、旧快照可继续手动追加或输出，
  鼠标回到启动该步前的位置。记录目标窗身份错误与质量门 reason code，不能只看最终 PNG。

## 5. GNOME Wayland

- 会话内记录 `XDG_SESSION_TYPE=wayland`、`XDG_CURRENT_DESKTOP`、Portal 接口版本和 XWayland 状态。
- 按顺序验证窗口速选扩展：未安装、安装后待注销、注销后 active、磁盘升级但会话仍旧、再次注销恢复。
- 在 RemoteDesktop 授权允许时验证自动粘贴；拒绝时确认复制已成功、没有输入注入循环，并观测
  `portal_select_devices_rejected`、`portal_start_rejected`、`portal_keyboard_not_granted` 或
  `portal_attempt_exhausted` 之一；设置页能力原因仍应是 `wayland_portal_permission`。
- 验证区域截图始终可用；窗口几何缺失时 UI 不得声称窗口命中可用。
- 绝对定位和永久置顶受 compositor 限制时，能力面板与 Pin UI 必须显示
  `wayland_protocol_limited`，不得循环调用无效定位。
- 使用混合缩放多屏完成截图诊断并保存 I1–I5；I4/I5 未观测不能写成 PASS。
- Ubuntu 24.04/26.04 还必须记录 GNOME、xdg-desktop-portal 与 desktop portal backend 版本，分别触发
  Mutter、Shell helper、Portal 和后续 fallback 中环境实际支持的路径；诊断记录的 selected backend
  必须与观测一致，不能沿用其它 Ubuntu 版本的结论。
- 长截图控制窗只有在 RemoteDesktop 与 ScreenCast Portal 同时可用时才显示“授权自动滚动”；
  `DISPLAY` 或 XWayland 存在不能让它提前进入可用状态。分别记录拒绝、关闭选择器、允许以及选错
  显示器；前三种不得发送滚轮，允许后才显示上下左右控制。
- 授权成功后在主屏、负坐标副屏和混合 DPI 副屏各执行四个方向。每方向至少提交两帧，并验证
  Stop/Esc、页面到底、动态内容、手动改变页面、撤销后继续、Copy/Save/Pin；视觉身份变化必须在
  输入前以 `longshot_auto_target_lost` 暂停，旧画布仍可手动追加或输出。
- 完成、取消、关闭控制窗和授权过程中按 Esc 后检查 Portal 会话已关闭；重新进入长截图必须重新
  授权，不能复用前一代次的 stream 或指针权限。

## 6. KDE 与 wlroots Wayland

- GlobalShortcuts Portal 分别验证首次允许、部分允许、全部拒绝、修改快捷键、暂停和恢复。
- RemoteDesktop Portal 分别验证允许与拒绝；拒绝后必须保持 copy-only 并显示稳定 reason code。
- KDE 验证 Portal 区域截图；wlroots 验证逐输出/data-control 可用路径及 Portal 缺失路径。
- 当全局窗口枚举、绝对定位或永久置顶不可用时，区域截图仍须成功，相应按钮不得承诺不可兑现能力。
- 暂停/移除 Portal backend 后重新打开设置页，typed capability 必须实时反映
  `wayland_portal_unavailable`，恢复 backend 后无需清理用户数据。
- KDE 与 wlroots 分别执行 GNOME 小节中的长截图允许、拒绝、选错显示器、四方向和取消矩阵；
  compositor 不返回 stream position/size 时，多屏必须保守拒绝，单屏可按冻结显示器尺寸继续。

## 7. Windows 10/11

- 使用普通权限启动 Clippy 和记事本，确认选择条目后恢复目标窗口并注入一次粘贴。
- 保持 Clippy 为普通权限，以管理员身份启动记事本；再次选择条目，确认剪贴板更新但不抢焦点、不注入，
  UI 显示 `windows_integrity_boundary`。
- 在 100%/125%/150% 混合 DPI、多屏和负坐标排布下验证区域/窗口截图、窗口命中、Pin 初始位置和拖动。
- 从同一选区进入长截图，依次验证上下左右自动滚动与 Stop/Esc；在主屏、负坐标副屏和混合 DPI
  副屏分别确认指针命中选区中心、滚动目标不串窗、每个方向至少提交两帧。移动鼠标时必须暂停并
  保留用户的新位置；目标移动/关闭、页面到底和动态内容失败后旧画布仍可手动追加或输出。
- 保持 Clippy 普通权限并把滚动目标提升为管理员权限，确认自动滚动在注入前以稳定错误暂停，目标
  页面不滚动，已提交画布不变；不能只依赖 `SendInput` 返回值判断 UIPI。
- 验证 Pin 原生 topmost、最小化/恢复、全屏应用切换和目标窗口销毁后的行为。
- 分别安装 NSIS 和 MSI，验证升级、卸载、WebView2 bootstrapper、自动启动和 updater；记录安装包
  Authenticode 状态与 signer thumbprint。
- 检查应用私有目录 DACL：当前用户可访问，普通其他用户不可访问；旧宽松配置文件启动后被修复，
  连续配置更新可原子覆盖。`windows_integrity_query_failed` 的安全 copy-only 分支由自动化测试守卫。
- 在录屏选区工具条依次选择无音频、系统声音和麦克风；后两种模式的结果库必须显示 Opus 音轨摘要，
  系统声不得混入麦克风，麦克风模式不得静默切到 loopback。暂停/继续后听感和时间线都应连续。
- 录制中禁用或拔出当前音频设备，确认视频与音频一起中止、控制窗关闭、Recording gate 可再次使用，
  结果库自动打开且已原子提交的双轨分段仍可列出；不得留下继续占用设备的 WASAPI client。
- 使用稳定节拍声与可见计时器连续录制至少 30 分钟，记录开头/中段/结尾 A/V 偏差、CPU、峰值内存、
  丢帧和音频缺口。未记录数值不能把 `PX-REC-WINDOWS-AV-QA-01` 标为真机通过。

## 8. macOS Intel/Apple Silicon

- 在系统设置中分别制造屏幕录制的未决定、拒绝、允许和允许后撤销状态。每个状态都重新触发截图：
  进程内最多主动请求一次；授权后无需重启即可恢复；撤销后实时回到
  `macos_screen_recording_permission`。
- 对辅助功能重复未决定、拒绝、允许和撤销流程。拒绝/撤销时复制成功但不注入，显示
  `macos_accessibility_permission_required`；设置页能力原因是 `macos_accessibility_permission`。
  允许后恢复目标应用并只注入一次粘贴。
- 在多个 Spaces、全屏应用、不同缩放显示器和外接屏上验证截图覆盖层、Pin、工具条和窗口层级。
- 辅助功能拒绝或撤销时，长截图控制窗只显示权限说明且没有方向/启动控件；允许后重新进入长截图，
  在 Retina 主屏和外接屏分别验证上下左右、Stop/Esc、目标切换、页面到底和动态内容。每步须命中
  同一 Quartz 窗口 ID/PID；移动鼠标时保留用户的新位置，其它结束路径恢复步骤开始前的位置。
- Intel 与 Apple Silicon 分别导入同一旧版工程，并在各自数据库中保存内部修订；重开扁平结果继续
  编辑，比较 renderer v2 RGBA 摘要，确认输出 PNG 不含原图或操作层。
- 对最终 `.app`/DMG 验证严格代码签名、`Signature=adhoc`、目标架构、首次打开提示和 updater；明确记录
  它没有 Developer ID authority、公证或 stapled ticket，不能把手动允许误记成 Gatekeeper 公共信任。
- 在 macOS 12.x、13/14 与 15+ 分别核对录屏工具条：12.x 只有无音频，13/14 增加系统声，15+ 再
  增加默认麦克风。伪造不可用模式必须在消费冻结会话前失败。
- macOS 15+ 分别拒绝与允许麦克风权限；拒绝后两轨共同中止、控制窗关闭、已提交分段保留且下一次
  可以重试，允许后结果库显示 Opus 音轨摘要。静音、暂停/继续与强杀恢复不能混入系统声。
- 使用内建与外接麦克风核对原生格式。当前 QA 切片只接受 48 kHz、mono/stereo packed Float32；
  其它格式必须明确中止，不能误标为 48 kHz。记录设备消失/切换与至少 30 分钟 A/V 漂移。

## 9. 受门控录屏原型

录屏原型当前只在同一 SHA 的 Linux X11/Wayland、Windows 10/11 与 macOS 12.3+ QA 包开放；Linux
与 Windows AV QA 包可显式选择系统声或默认麦克风，macOS 13+ AV QA 包可显式选择系统声，15+
再可选择系统默认麦克风，macOS 12.3–12.x 仍为无音频。每次先保存安装包
SHA-256、`QA-BUILD.txt` 和完整 commit，再执行对应模板中的录屏场景：

1. 选取一个已知尺寸（建议 640×360 或 1280×720）的区域，录制至少 10 秒并移动光标；暂停至少 3 秒后
   继续并停止。核对输出尺寸、光标、有效时长不含暂停段、结果库播放、首帧缩略图、导出和文件定位。
2. Linux X11 把控制窗放在选区外；Windows 10 22H2/11 可把控制窗移入选区，输出仍不得出现控制窗，
   以验证 `WDA_EXCLUDEFROMCAPTURE`。macOS 把控制窗移入选区并验证 ScreenCaptureKit 只排除该
   `CGWindowID`，Pin 与其他 Clippy 窗口仍应被录入。控制窗排除失败时开始操作必须回滚，不能生成
   静默污染的录屏。
3. 在 100%/125%/150% 混合 DPI、负坐标和多显示器布局下重复区域录制，逐帧抽查边界与光标像素；记录
   CPU、峰值内存、丢帧/重复帧和播放器兼容结果，不能只记录“文件能打开”。
4. 连续录制超过 65 秒，确认至少一个周期分段已提交后强制终止进程。重启后中断会话必须列出可播放
   分段和首帧缩略图，并能无损合并、播放及导出；未提交尾段不得冒充已恢复数据。
5. Wayland 五个 profile 先验证授权窗与 ScreenCast Portal 系统选择器绑定；分别记录允许、拒绝、
   Cancel、关闭授权窗、选错显示器和多屏元数据结果。失败后入口可再次打开，且不得留下文件、Portal
   session、PipeWire stream 或 Recording gate。
6. Wayland 授权成功后，授权窗必须在首帧前隐藏；录制阶段只用托盘 Pause/Resume/Stop。全屏选区也须
   可控，输出不得出现授权窗；GNOME、KDE、wlroots 分别记录 Portal/PipeWire 后端、光标、分数缩放、
   旋转屏、4K 带宽和静态画面行为。普通 release 包仍保持门控。
7. Linux X11/Wayland 与 Windows 分别录制系统声和默认麦克风；Linux 额外记录 PipeWire 与 session
   manager 版本，并确认系统声来自默认 sink monitor、麦克风来自默认 capture source，二者不会
   静默互换。macOS 13+ 录制系统声；macOS 15+ 另录制默认麦克风并分别验证权限拒绝/允许，确认
   麦克风回调不会接收系统声。macOS 12.x 工具条只显示无音频，13/14 不得显示麦克风。
8. 对有声模式核对静音片段、暂停/继续、设备消失或默认设备切换后的完整回收，以及至少 30 分钟的
   音视频漂移；记录输出 Opus 参数、首尾可听内容、schema v2 统计、CPU 和峰值内存。Linux 还要在
   PipeWire 服务重启后确认两轨共同中止、已提交恢复分段仍可见，下一次录制可以重新建立 stream。

Linux X11/Wayland、Windows 与 macOS 的录屏记录全部完成前，对应 `PX-REC-WINDOWS-QA-01`、
`PX-REC-MACOS-SCK-01`、`PX-REC-MACOS-AUDIO-01`、`PX-REC-MACOS-MIC-01` 与
`PX-REC-WAYLAND-QA-01` 保持待验收；编译、
Xvfb、GitHub runner 或合成帧不能替代原生桌面证据。Linux 音频矩阵全部完成前，
`PX-REC-LINUX-AUDIO-01` 同样保持待验收。

## 10. 结论规则

- 原生 CI 与对应 profile 的结构化记录必须绑定同一 commit SHA。
- `pass` 表示功能按步骤实际成功；`expected_degraded` 表示操作系统明确限制且产品按合同安全降级。
- `fail`、`not_run`、缺证据、reason code 不符或仅有交叉编译结果都不能勾选 PRD 验收项。
- 某个平台修复后必须重新运行受影响 profile；不得沿用修复前记录。
- 九个 profile 全部通过前，跨平台任务保持 `in_progress`。
