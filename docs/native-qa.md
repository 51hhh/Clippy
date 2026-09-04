# 跨平台真机 QA

本手册用于完成 `.trellis/tasks/09-02-cross-platform-compatibility/prd.md` 中不能由 Linux 本机或
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
| `linux-kde-wayland` | KDE Wayland |
| `linux-wlroots-wayland` | 一个 wlroots compositor |
| `windows-10-x64` | Windows 10 22H2 x64 |
| `windows-11-x64` | Windows 11 x64 |
| `macos-intel` | macOS 11+ Intel |
| `macos-apple-silicon` | macOS 11+ Apple Silicon |

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
8. 对同一图片依次验证全部 16 种画布工具：裁剪、对象、橡皮、文本，pen/marker/rect/ellipse/
   highlight/arrow/line/measure 八种绘制，以及 blur/mosaic/spotlight/magnifier 四种像素效果；再验证
   调整、撤销/重做、Copy 和扁平导出。
9. 保存可编辑 PNG，关闭应用后重开，把该 PNG 拖入主窗口继续编辑并导出；确认普通 PNG 不会绕过
   剪贴板队列直接创建 Pin，并记录重开前后已验证 IDAT 预览和最终 PNG 的 SHA-256。跨平台比较必须
   使用相同工程文件和 renderer v2 金图测试，不能用目测代替摘要。
10. 验证系统 Pictures 目录、自定义目录、重名不覆盖及系统目录不可用时的应用数据目录 fallback。
11. 分别在 Tesseract 可用/不可用状态检查 OCR 与平台提示；Windows/macOS 不得出现 Linux 安装按钮。
12. 使用至少两个已启用翻译服务验证并行结果、自动换向、单服务重试、缓存和删除；图片只允许发送
    本地 OCR 文本，敏感内容必须在网络请求前阻断，并实际播放一次 TTS。
13. 保存翻译凭据并检查系统凭据管理器；模拟 keyring 失败时，不得在配置或日志出现明文密钥。
14. 切换六套主题、自动/中文/英文语言及各设置，重启后确认保持；托盘菜单语言和暂停状态同步，重复
    启动只激活已有实例，主窗口/设置窗口不会生成失控副本。
15. 启用登录启动并完成一次真实登录验证，再关闭并确认系统启动项被清理。Linux 另测 tmux copy-mode
    hook；Windows/macOS 确认 tmux 控件隐藏且没有执行 inotify/nix 命令。
16. 保存 typed 平台能力与截图诊断，确认不包含剪贴板内容、图片像素、窗口标题、Portal token 或密钥。
17. 使用该平台正式安装包完成检查更新、下载、安装和重启；确认安装类型不是被错误识别为 deb。

证据中不得包含真实剪贴板秘密、翻译 token、Portal restore token、完整截图像素或窗口标题。测试内容
使用专门的无敏感 fixture。截图诊断报告已按设计排除像素与窗口标题。

## 4. Linux X11

- 从登录界面明确选择 Xorg，会话内记录 `XDG_SESSION_TYPE=x11`。
- 用普通文本编辑器验证自动粘贴恢复焦点；再分别测试主面板打开/关闭和快捷键触发。
- 使用两块不同缩放/负坐标显示器验证窗口命中、选区边界、Pin 初始位置和拖动。
- 运行 `clippy --capture-diagnose`，保存 I1–I5、typed `PlatformInfo` 和 monitor-layout fixture。
- Pin 后切换工作区、全屏窗口和普通窗口，确认 X11 topmost 行为与工具条状态一致。

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
- Ubuntu 24.04 还必须记录 GNOME、xdg-desktop-portal 与 desktop portal backend 版本，分别触发
  Mutter、Shell helper、Portal 和后续 fallback 中环境实际支持的路径；诊断记录的 selected backend
  必须与观测一致，不能沿用 Ubuntu 22 的结论。

## 6. KDE 与 wlroots Wayland

- GlobalShortcuts Portal 分别验证首次允许、部分允许、全部拒绝、修改快捷键、暂停和恢复。
- RemoteDesktop Portal 分别验证允许与拒绝；拒绝后必须保持 copy-only 并显示稳定 reason code。
- KDE 验证 Portal 区域截图；wlroots 验证逐输出/data-control 可用路径及 Portal 缺失路径。
- 当全局窗口枚举、绝对定位或永久置顶不可用时，区域截图仍须成功，相应按钮不得承诺不可兑现能力。
- 暂停/移除 Portal backend 后重新打开设置页，typed capability 必须实时反映
  `wayland_portal_unavailable`，恢复 backend 后无需清理用户数据。

## 7. Windows 10/11

- 使用普通权限启动 Clippy 和记事本，确认选择条目后恢复目标窗口并注入一次粘贴。
- 保持 Clippy 为普通权限，以管理员身份启动记事本；再次选择条目，确认剪贴板更新但不抢焦点、不注入，
  UI 显示 `windows_integrity_boundary`。
- 在 100%/125%/150% 混合 DPI、多屏和负坐标排布下验证区域/窗口截图、窗口命中、Pin 初始位置和拖动。
- 验证 Pin 原生 topmost、最小化/恢复、全屏应用切换和目标窗口销毁后的行为。
- 分别安装 NSIS 和 MSI，验证升级、卸载、WebView2 bootstrapper、自动启动和 updater；记录安装包
  Authenticode 状态与 signer thumbprint。
- 检查应用私有目录 DACL：当前用户可访问，普通其他用户不可访问；旧宽松配置文件启动后被修复，
  连续配置更新可原子覆盖。`windows_integrity_query_failed` 的安全 copy-only 分支由自动化测试守卫。

## 8. macOS Intel/Apple Silicon

- 在系统设置中分别制造屏幕录制的未决定、拒绝、允许和允许后撤销状态。每个状态都重新触发截图：
  进程内最多主动请求一次；授权后无需重启即可恢复；撤销后实时回到
  `macos_screen_recording_permission`。
- 对辅助功能重复未决定、拒绝、允许和撤销流程。拒绝/撤销时复制成功但不注入，显示
  `macos_accessibility_permission_required`；设置页能力原因是 `macos_accessibility_permission`。
  允许后恢复目标应用并只注入一次粘贴。
- 在多个 Spaces、全屏应用、不同缩放显示器和外接屏上验证截图覆盖层、Pin、工具条和窗口层级。
- Intel 与 Apple Silicon 分别把同一可编辑 PNG 拖入主窗口，继续编辑并比较 renderer v2 RGBA 摘要。
- 对最终 `.app`/DMG 验证严格代码签名、`Signature=adhoc`、目标架构、首次打开提示和 updater；明确记录
  它没有 Developer ID authority、公证或 stapled ticket，不能把手动允许误记成 Gatekeeper 公共信任。

## 9. 结论规则

- 原生 CI 与对应 profile 的结构化记录必须绑定同一 commit SHA。
- `pass` 表示功能按步骤实际成功；`expected_degraded` 表示操作系统明确限制且产品按合同安全降级。
- `fail`、`not_run`、缺证据、reason code 不符或仅有交叉编译结果都不能勾选 PRD 验收项。
- 某个平台修复后必须重新运行受影响 profile；不得沿用修复前记录。
- 九个 profile 全部通过前，跨平台任务保持 `in_progress`。
