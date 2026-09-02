# 跨平台兼容验证记录

日期：2026-09-02

审查基线：`621e36f289a386dcf84d26c77742f6b0f1eae31e`

应用实现、九环境 QA 合同与三平台原生门禁已完成至：`5d703675aefc082e5ad10656f8288c05058200a1`

## 审查范围

- 截至上述审查点，审查基线之后共 154 个提交，以及提交时仍存在的工作区内容。
- 逐项检查截图、窗口命中、Pin、标注画布、可编辑 PNG、剪贴板、自动粘贴、快捷键、OCR、
  私有存储、自动启动、平台配置、CI 和 release 数据流。
- `.omo/` 与 `.trellis/workspace/codex/` 是未跟踪的用户工作区，本次未读取、未修改、未提交。
- 复核基线之后的 commit subject：type/scope 符合项目允许的 Conventional Commits 形式，
  首行均不超过 72 字符；没有为“改标题”而重写共享历史。

## 本机自动验证

在 Linux 主机上执行 `./scripts/ci-local.sh`，结果为 12 通过、0 失败、1 跳过：

| 门禁 | 结果 |
|---|---|
| `cargo fmt --check` | 通过 |
| `cargo check` | 通过 |
| `cargo clippy -- -D warnings` | 通过 |
| `cargo test` | 425 通过、0 失败、7 个真实桌面/诊断/手动性能测试忽略 |
| GNOME 扩展静态检查 | 通过，Shell 45–51 |
| `npm ci` | 通过，0 个已报告漏洞 |
| `tsc --noEmit` | 通过 |
| Vitest | 44 个文件、818 项测试通过 |
| DOM/Xvfb smoke | 9 项通过 |
| Canvas 导出像素 smoke | 通过，抽样像素 `0 208 0` |
| 主窗口布局像素 smoke | 通过，抽样像素 `0 208 0` |
| Vite production build | 通过，1888 个模块完成转换 |
| AppImage X11 可视 smoke | 本次按环境开关跳过；此前本机构建产物与 Ubuntu 22 finalized 产物已独立验证窗口、单实例回调和首帧 |

受限沙箱内首次运行时，16 个回环 HTTP 测试因禁止 `TcpListener::bind` 失败，Vite/Xvfb smoke
也无法绑定端口或显示；在允许回环网络和虚拟显示的同一主机上复跑后全部通过。该差异属于执行环境，
不是断言失败。

附加静态检查：

- 五份 Tauri JSON 配置均可解析，三份 GitHub Actions YAML 均可解析。
- 设置页 About/平台能力区域在本地 Vite 页面完成浏览器结构与空载布局检查；普通浏览器没有 Tauri
  IPC，因此填充后的九项能力列表由 3 个 DOM 单测验证状态色、原因文案和纯文本节点渲染。
- Linux 默认依赖图不含 `pipewire-rs`。
- Windows/macOS 目标依赖图不含 GTK、WebKitGTK、zbus、ashpd、libwayshot、PipeWire、inotify、
  x11rb 或 Linux `nix` 实现。
- 三平台应用身份由回归测试固定：Linux/Windows 继承公共 `com.clippy.app`；macOS 有意覆盖为
  `com.clippy.desktop`。仓库锁定的 Tauri CLI 2.11.4 会明确警告以 `.app` 结尾的 bundle identifier
  与 macOS 应用包扩展冲突；macOS 覆盖值不得随意变化，否则 TCC 授权、WebView 数据目录和更新身份
  会一起漂移。
- Windows 私有文件 ACL、原子替换，以及自动粘贴完整性令牌所用的 Win32 API 在独立最小工程中完成
  MSVC 目标检查；这验证目标条件编译和 `windows-sys 0.61.2` 类型调用，不代替原生运行。
- 独立 Native QA workflow 增加真正的 Tauri bundle：关闭 updater 附加产物但不关闭 bundler；
  Windows 必须生成 NSIS/MSI，macOS 必须生成 ad-hoc 签名的 app/DMG。本机已用同一 CI 配置完成
  `tauri build --debug --no-bundle`，验证配置合并、前端钩子和应用构建；安装包仍必须由原生 runner 生成。
- 已按仓库锁定的 Tauri CLI 2.11.4 / bundler 2.9.4 复核 updater 产物：`true` 模式下 Linux
  AppImage 与 Windows NSIS/MSI 是自包含更新器并直接签名，macOS 使用 `app.tar.gz`；工作流收集的
  文件名和 `linux/windows/darwin` 四个 `OS-ARCH` manifest key 与该规则一致。旧式 zip/tar 规则仅
  属于已弃用的 `v1Compatible` 模式。本结论是源码/配置审查，产物仍需原生 release job 验证。
- `latest.json` 现由受测 Node 生成器统一生成：同一份平台契约同时绑定 Tauri 覆盖配置中的 bundle
  target、四个 `OS-ARCH` key、标准化 artifact 文件名和 `.sig` 文件名；生成器拒绝缺失/空签名、
  非 SemVer 版本、非 UTC RFC 3339 时间和非 HTTPS 下载地址。签名内容直接内嵌，不接受签名 URL。
- `git diff --check` 通过。
- renderer v2 组合金图覆盖四种效果、九种矢量/文字工具、调整与圆角，Linux RGBA SHA-256 为
  `0868d38bf2e18a1f62d01cfa55d37954b1a66d3f2b99b3affb83dbe5d1b64478`；同一常量已进入原生 CI 测试。
- 3840×2160 手动性能探针包含模糊、马赛克、聚光、放大镜、矢量和中英文文字；当前 Linux 主机的
  优化 dev profile 合成耗时 1.627 秒。探针在常规 CI 中忽略，需显式 `--ignored` 运行以免硬件噪声
  造成不稳定门禁。

## Ubuntu 22 release 与 AppImage 前向兼容验证

- 使用 Docker 官方 `ubuntu:22.04` 镜像、`5c71dde` 的干净 `git archive` 和 Jammy 官方仓库完成
  release 构建；随后两个提交只修改依赖清单和 finalization 脚本，没有改变应用源码。没有使用 PPA，
  系统 PipeWire 为 0.3.48，`cargo tree` 明确不含 `pipewire-rs`。
- 首轮在 Rust release 链接完成后发现 AppImage bundler 需要 `/usr/bin/xdg-open`，而仓库依赖清单未
  显式安装 `xdg-utils`。补齐 CI、release 和两份开发文档后，同一干净 Jammy 环境成功生成
  `Clippy_0.1.17_amd64.AppImage`。
- Jammy 原始 AppImage 为 83,192,312 字节，SHA-256
  `5cf2fa2bdf116f6a6b73a6e2cc519792946ea07b0c9e358e320126e04b62bacc`，SquashFS offset
  944632，`.DirIcon` 是相对链接；但在 Ubuntu 26.04 / Mesa 25 上启动时，内置 Jammy
  `libwayland-*` 与宿主图形栈混载，WebKit 报 `EGL_BAD_PARAMETER` 并产生空白首帧。
- 该结果与 Tauri 上游问题 `tauri-apps/tauri#15665` 的根因一致。`finalize-appimage.sh` 现把
  `libwayland-client/cursor/egl/server` 四个 ABI 库作为一个集合移除，重封装后再从 SquashFS
  反查，发现残留会直接失败：
  https://github.com/tauri-apps/tauri/issues/15665
- finalized Jammy AppImage 为 83,134,968 字节，SHA-256
  `10edc7680aa1f1745673e6bb31eb0d985d2de5115154a4263b6ca297a320480b`。同一 Ubuntu 26 主机的
  隔离 X11/DBus smoke 通过：主窗口 380×500、无装饰、位于 1280×800 屏幕内，单实例回调成功，
  首帧非空白。修改后的脚本同时通过 `bash -n` 和 ShellCheck。
- Ubuntu 26 本机直接构建的原始 AppImage 为 85,899,768 字节，SHA-256
  `e4cff34b1d00d0140a48e87e7193cc951cc275672b5b31b335b4549f8adf6103`，同样通过隔离 X11 smoke；
  此项只证明当前发行版运行，不替代 Jammy 构建证据。
- 2026-09-02 在当前 Ubuntu 26 GNOME Wayland 真机从沙箱外执行
  `clippy-app --capture-diagnose`：typed 平台探测为 `linux/wayland/gnome/x86_64`、XWayland 可用，
  Portal Desktop/GlobalShortcuts/RemoteDesktop/Screenshot/ScreenCast 均存在，接口版本分别为
  `1/2/2/5`。GNOME 扩展能应答但运行会话版本陈旧，正确报告“磁盘已升级，等注销一次”。
- 同次诊断的 `wl_output` 为 2560×1440@1.5 与 1920×1200@1.3333，逻辑排布
  `4480×1608@0,0`；截图选择 `gnome-shell-extension + wl_output`，获得 6720×2412 舞台图，I1、I2a、
  I2b、I3 全部 PASS。XWayland/xcap 同时误报两屏为 2.0，证明运行时没有拿错误来源覆盖可信几何。
  命令行模式没有 Tauri 窗口和覆盖层会话，所以 I4 明确为“未观测”、I5 为“未检查”，没有伪记 PASS。
  该几何已经由 `gnome-dual-mixed-scale.json` 覆盖；仅运行期 output ID/枚举顺序不同，不重复添加 fixture。
- 同一真实桌面环境继续运行后端逐链路诊断：当前会话中的扩展协议为 v4、磁盘为 v5，因此逐屏 v5
  路径按预期拒绝；扩展整屏舞台图和非交互 Portal 均返回两块有效画面，分别为 2880×1800
  （平均亮度约 236.8、全黑 0%、全透明 0%）与 3840×2160（平均亮度约 233.5、全黑 2.5%、
  全透明 0%）。wlroots 因会话没有所需协议而拒绝，GNOME Screenshot D-Bus 被策略拒绝；xcap 则给出
  错误的 2.0 缩放与一块 14.2% 透明画面。实际选择链仍取得扩展与 `wl_output` 的可信帧，证明回退链
  能区分“后端可调用”和“几何/像素可信”。
- `capture_stage_timings` 真机探针显示：扩展 Screenshot D-Bus 680.8 ms、2102192 字节文件读取
  1.3 ms、6720×2412 PNG 解码 131.0 ms，`capture_monitor_frames` 总计 1039.0 ms；窗口枚举 4.3 ms、
  候选探测 3.7 ms；两屏 PNG 编码/BASE64 分别为 73.0/0.5 ms（1050/1400 KiB）和
  100.4/1.1 ms（1501/2002 KiB）。这些数字说明当前会话因扩展尚未注销升级到 v5，仍在使用较慢的
  PNG 兜底，不把它误记成 Mutter ScreenCast 快速路径结果。
- 脱敏后的 `window_probe_diagnostics` 枚举到 2 个窗口，耗时 0.802 ms；输出只包含最小化状态与几何，
  不包含标题或 PID。枚举后再次截取两屏均成功，平均亮度约 236.4/233.8、全黑 0%/2.5%、全透明均
  为 0%，证明窗口枚举没有污染后续截图。以上三组结果只属于当前 Ubuntu 26 GNOME Wayland 主机，
  不替代 Ubuntu 22/24 GNOME profile，也没有被写成九环境真机验收通过。

## 原生平台验证状态

2026-09-02 的 0.1.18 发布前 CI 复审把职责拆成三条：`CI Check` 仅保留三平台源码门禁，
`Native QA Packages` 手动生成 Linux x64、Windows x64、macOS Intel/Apple Silicon 测试包，并由
Ubuntu 24 runner 强制运行 Jammy AppImage；`Release` 在同 SHA CI 与签名策略预检通过后并行构建，
Linux x64、Windows x64、macOS Intel/Apple Silicon 全部进入 workflow artifacts，最终 job 才创建 draft、
生成固定四目标的 `latest.json` 并公开发布。Windows 没有 PFX 时使用临时自签名证书并在发布说明标出
SmartScreen 风险，不把它冒充为公共 CA 信任。macOS QA 与正式 release 都通过
`signingIdentity: "-"` 使用 Ad-Hoc 签名，并独立验证 `codesign --strict`、`Signature=adhoc` 和目标架构；
发布说明明确它没有 Developer ID 公证。
这套新结构的 GitHub 原生运行证据需在修复提交推送后补记，以下 `8ec2e25` 记录保留为上一版可安装
产物基线，不冒充新 workflow 的验证结果。

`dev` 与 `origin/dev` 已同步到实施 SHA
`8ec2e252f7949db384c64d497789bfc12b6fdb77`。GitHub Actions run
[`33641161291`](https://github.com/51hhh/Clippy/actions/runs/33641161291) 对这个完整 SHA 给出三项原生
job 全绿；`scripts/verify-native-ci.mjs` 再从 check-runs API 独立核对精确 job 名、状态和结论，结果为
PASS。机器生成的逐 job 时间与链接保存在 [native-ci-evidence.md](native-ci-evidence.md)。

| 原生 job | 结果 | 已执行门禁 |
|---|---|---|
| `Check (ubuntu-22.04)` | `completed/success` | fmt、check、Jammy 依赖基线、clippy、425 项 Rust 测试、819 项前端测试、DOM/Xvfb、typecheck、build、deb/AppImage |
| `Native Check (windows-latest)` | `completed/success` | 原生 check/clippy/test、前端 test/typecheck/build、Tauri bundle、NSIS/MSI 产物核对 |
| `Native Check (macos-latest)` | `completed/success` | 原生 check/clippy/test、前端 test/typecheck/build、Apple Silicon Tauri bundle、app/DMG 产物核对 |

Windows runner 同时执行并通过私有文件/目录 ACL 修复验证；Windows 与 macOS runner 都执行 renderer
v2 固定 RGBA 摘要测试，因此软件渲染器在三平台对同一 fixture 输出相同像素。上述 CI 证明原生编译、
自动测试和无发布签名的安装包能够生成，不替代 Windows/macOS 真实桌面交互，也不证明 Authenticode、
Developer ID、公证或正式 updater 发布链。

同一 run 的独立 `QA Bundle (macOS Intel)` job 也以 `completed/success` 结束。五份保留 14 天的 artifact
均以完整 SHA 命名，已从 GitHub 重新下载到临时目录并检查实际内容：

| QA artifact | 远端大小 | 下载后核验 |
|---|---:|---|
| `qa-linux-x64-8ec2e252…` | 117,180,099 B | deb 与 finalized AppImage SHA-256 通过；tar 保留 AppImage 执行位 |
| `qa-windows-x64-8ec2e252…` | 34,195,786 B | NSIS、MSI 与 `QA-BUILD.txt` SHA-256 通过 |
| `qa-macos-aarch64-8ec2e252…` | 18,926,875 B | Apple Silicon DMG SHA-256 通过 |
| `qa-macos-x64-8ec2e252…` | 19,192,994 B | Intel DMG SHA-256 通过 |
| `qa-record-templates-8ec2e252…` | 26,924 B | 九份 JSON 均绑定完整 SHA、0.1.17 和至少 20 项场景 |

每套安装包内的 `QA-BUILD.txt` 都明确标记 `unsigned-qa-only` 或 `ad-hoc-qa-only`，因此只解决“真机
测试人员拿不到同 SHA 安装包”的交付缺口，不会被误当作签名发布证据。Linux artifact 额外把 deb、
AppImage、元数据和摘要封装到 tar，规避 GitHub artifact 解压后丢失可执行权限的问题。

Linux 主机上的 Windows 交叉检查仍会因缺少 MSVC `lib.exe`/SDK 及 C 依赖工具停止，macOS 交叉检查
仍缺 Apple SDK；现在它们只是本机环境限制，原生 runner 结果才是本轮的平台构建证据。

## 实机 QA 矩阵

“待测”不得在没有目标系统证据时改成“通过”。

九个环境现由 `scripts/manual-qa.mjs` 生成结构化 JSON 模板并校验：每份记录绑定完整 SHA/SemVer，
逐场景要求实际状态、文字观测和证据引用，安全降级还必须记录精确 reason code；缺失、重复、额外场景
或 `not_run` 均返回失败。完整操作步骤见 `docs/native-qa.md`。下表仍保持人读摘要，最终结论以通过校验
且与原生 CI 同 SHA 的记录为准。
公共合同另外锁定监听与去重、搜索/收藏/清理/统计、键盘状态机、全部 codec 操作、安全富预览与 URL 元数据、
多服务翻译/TTS/隐私、主题/语言/设置持久化、托盘/单实例/窗口复用、自启动、tmux 平台行为和诊断隐私，
防止只测平台后端却漏掉产品功能。

| 平台/会话 | 必测场景 | 状态 |
|---|---|---|
| Ubuntu 22.04 GNOME 42 X11 | 文本/HTML/图片、快捷键、自动粘贴、区域/窗口截图、混合 DPI、Pin、画布、可编辑 PNG、OCR、keyring、更新 | 待测 |
| Ubuntu 22.04 GNOME 42 Wayland | 上述场景；另测 Portal 授权/恢复、GNOME Helper 未装/待注销/已就绪、窗口层级降级 | 待测 |
| Ubuntu 24.04 GNOME Wayland | 原生 Wayland 截图 fallback、快捷键、Pin、Portal 版本差异 | 待测 |
| KDE Wayland | GlobalShortcuts Portal 首次授权/部分接受/修改/录制暂停、Portal 截图、缺少窗口几何和绝对定位时的 UI/reason code | 待测 |
| wlroots compositor | GlobalShortcuts Portal 存在/缺失、区域截图、data-control、窗口枚举缺失与 copy-only 降级 | 待测 |
| Windows 10 22H2 / Windows 11 | 普通与管理员目标自动粘贴、UIPI 降级、混合 DPI/多屏、topmost、截图、安装/更新 | 待测 |
| macOS Intel / Apple Silicon | TCC 未决定/拒绝/允许/撤销、Spaces、全屏辅助窗口、截图、粘贴、签名/公证/更新 | 待测 |

## 已修复的审查问题

- 真实桌面用的窗口探测诊断原先会把窗口标题和 PID 打到终端，违反诊断证据的隐私边界；现在只输出
  窗口数量、最小化状态和几何，并由前端回归守卫禁止 `.title()`/`.pid()` 重新进入该诊断函数。
- Windows 无法用 Unix `rename` 原子覆盖已有配置：改用 `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)`。
- Windows Portal restore token 的滚动更新同样无法用 `std::fs::rename` 覆盖已有目标：配置与 token
  现共用跨平台私有文件原子替换函数，并回归验证连续两次写入和最终权限。
- Windows 私有文件不再只依赖父目录继承权限：为文件和目录设置受保护 DACL，只保留当前用户
  `GENERIC_ALL` ACE，目录 ACE 向子项继承；启动读取旧文件时会校正并验证 ACL。
- 截图目录 fallback 和文件名只适配 Unix：增加 Windows home 变量、非法字符、尾随点/空格及保留设备名处理。
- 自动启动开发二进制判断只识别 `/target/`：改为平台路径规范化判断。
- 设置页的 OCR 与保存路径提示写死 Linux：改为平台中性文案和系统 Pictures 目录语义。
- `CHANGELOG.md` 仍宣称停止 Ubuntu 22：改为与依赖图、CI 和 release 一致的 Ubuntu 22 基线。
- 发布文档混淆 updater `.sig` 与 Authenticode：明确 Windows 可信代码签名仍是发布前置项。
- Windows release 会直接发布未做 Authenticode 的 NSIS/MSI：现在强制导入 PFX，检查私钥、代码签名
  EKU 与有效期，以 SHA-256/RFC 3161 签名，并在上传前验证两个安装包的状态和 signer thumbprint。
- release 曾只比较 tag 与 Tauri 版本、对非发布分支 tag 仅给 warning：现在硬校验 SemVer、Tauri/Cargo/
  CHANGELOG 三方一致性和 `main`/`dev` 可达性；现有 `v0.1.17` fixture 已通过同一组本地命令。
- release 曾在 YAML 内联 `jq` 中重复硬编码四个平台键、artifact URL 和签名变量，配置 target 改动后
  可能静默漂移；现改由可单测生成器维护唯一平台契约，并从标准化产物目录读取精确同名的四份签名。
- macOS 构建仅依赖 bundler 成功返回：上传前现在独立核对严格 Ad-Hoc 代码签名、`Signature=adhoc`
  和 Mach-O 目标架构；发布说明明确不具备 Developer ID、公证或 Gatekeeper 公共信任。
- 非 GNOME Wayland 只尝试 GNOME GSettings 并必然失败：接入 GlobalShortcuts Portal，按 XDG/xkb
  格式提交完整动作集合，核对 Portal 返回子集，并用可取消代次管理 Bind/session 生命周期。
- 平台能力只按 Wayland 环境变量推断 Portal：现在分别探测 Portal 桌面服务和 GlobalShortcuts、
  RemoteDesktop、Screenshot、ScreenCast 接口版本，并单独记录 XWayland；接口不存在时不会误报为
  “等待授权”。同一份 typed `PlatformInfo` 进入 IPC 和本地截图诊断报告。
- 前端仅用平台信息控制 OCR 安装入口、用户看不到其他降级原因：现在设置页按同一 typed 结构展示
  九项能力、稳定状态、本地化 reason、XWayland 与 Portal 接口版本，不再要求用户从日志猜测。
- OCR 可用性只检查 PATH，macOS GUI 和 Windows 常规安装后容易误报缺失；现在探测显式 override、
  未来 sidecar、PATH 与三平台常见目录，识别复用同一路径。设置页按 typed OS 显示对应安装提示。
- macOS 每次截图失败都会再次调用屏幕录制授权请求：现在每个进程最多请求一次，同时每次截图前仍
  重新 preflight；用户在系统设置中授权后无需重启即可恢复，拒绝或撤销后不会在进程内反复请求。
- 干净 Ubuntu 22 只按仓库清单安装依赖时，AppImage bundler 缺少 `xdg-open`：现在 CI、release
  和开发文档都显式安装 `xdg-utils`，同一 Jammy 容器已完成 release AppImage 构建。
- Ubuntu 22 构建的 AppImage 在 Ubuntu 26/Mesa 25 混载内置 Wayland ABI 库后空白：release
  finalization 现移除整组 `libwayland-*`，验证 SquashFS 无残留，并已用失败/通过 A/B 首帧复现闭环。
- Windows 自动粘贴不再把所有 `SetForegroundWindow` 失败都猜成 UIPI：捕获目标时同时记录 PID，
  粘贴前校验 HWND 所属进程未变化，并比较当前与目标进程的 Mandatory Integrity Level RID。目标更高
  时不恢复焦点、不注入按键，稳定返回 `windows_integrity_boundary`；令牌查询失败同样安全降级为
  copy-only，并返回 `windows_integrity_query_failed`。错误码、IPC 透传和 Win32 API 类型均有自动检查，
  普通/管理员目标的实际行为仍留在 Windows 10/11 实机矩阵。
- GNOME Wayland 的贴图置顶现在明确为依赖 Shell 扩展的降级能力；其它 Wayland 桌面标记为不支持，
  Pin 工具条、右键菜单和 `T` 快捷键不会再提供无法兑现的“置顶”操作。已置顶窗口仍允许关闭该状态。
- 截图默认目录不再在 Tauri `picture_dir()` 失败时猜测 `$HOME/Pictures`，也不会把 `Clippy` 重复拼接
  成 `Clippy/Clippy`：正常路径使用系统 Pictures/Clippy，系统图片目录不可用时使用应用数据目录下的
  Screenshots。Windows 风格的 `~\\Shots` 自定义目录也能正确展开。
- 打开可编辑 PNG 后，在用户尚未修改工程时直接显示并复用文件里的 IDAT 合成预览；复制、扁平保存和
  再存工程都不经过当前平台 WebView Canvas 重渲染，避免字体、模糊和滤镜实现差异改变像素。第一次真实
  编辑后才加载内嵌原图并提交新的合成图与工程文档；普通图片不能使用“复用预览”捷径。
- 新建或首次编辑后的工程改用 renderer v2：Copy、扁平保存、可编辑保存共用后端唯一合成入口，固定
  调整顺序、整数效果算法、纯 Rust 软件光栅器和仓库内 Noto Sans CJK SC 字体；v2 拒绝前端同时上传
  另一份 PNG。32 Mi 像素、32 Mi 模糊缓存和 512 Mi 效果工作量预算在分配前检查，渲染放到 blocking
  worker，不阻塞 WebView/GTK 事件线程。
- 截图覆盖层也不再上传 Canvas PNG：提交逻辑选区、renderer v2 操作层与桌面 origin，
  后端从当前 session/monitor 的可信冻结帧合成完整画面并输出选区视口。Copy、Save、Pin
  复用该结果；跨选区路径、模糊邻域和取消竞态有 Rust/前端合同测试。
- 完整 CI 首轮发现旧 `pin-gestures` 夹具没有实现新增的 typed 平台能力查询，导致组件挂载失败；补齐
  与产品 API 一致的 `platform()` mock 后，相关 42 项测试及上述完整门禁全部通过。

## 已知限制与发布阻断项

- 非 GNOME Wayland 已实现 GlobalShortcuts Portal，但要求系统 portal 与桌面 backend 支持该接口；
  Ubuntu 22 自带的 portal 1.14 没有该接口，GNOME/Jammy 因此继续使用 GSettings/D-Bus。Portal
  不可用或用户拒绝时会逐动作报告失败并提示系统设置手动绑定，不会假装注册成功或无限重试。
- Wayland compositor 可以拒绝窗口枚举、绝对定位和永久置顶；此时区域截图仍是保底能力。
- Windows `SendInput` 不能越过 UIPI 控制更高完整性目标；设计行为是复制成功后降级为 copy-only。
- macOS 屏幕录制与辅助功能受 TCC 控制。Apple 查询 API 只返回当前是否授权的布尔值，不公开区分
  未决定、拒绝和撤销；代码以实时 preflight + 单次用户触发请求收敛，但四种状态仍必须分别实测。
- OCR 尚未捆绑签名 sidecar；当前依赖用户安装的 Tesseract 5，并在 Linux 才展示应用内安装按钮。
  Windows 新版安装器来自 Tesseract 文档列出的第三方构建，正式捆绑前仍需单独完成来源、许可、
  DLL、traineddata、签名和更新策略审计。
- Windows Authenticode workflow 已接入；仓库没有 `WINDOWS_CERTIFICATE*` 时会在 runner 的个人证书库
  生成临时自签名证书，并只把其公钥短暂加入 `CurrentUser\TrustedPeople`，使 NSIS/MSI 的摘要校验
  必须严格返回 `Valid`；验证后删除个人证书与临时信任，不导入 Root 信任库。该模式不建立公共 CA
  信任，正式页面必须保留 SmartScreen 警告；首次 tag 运行仍需补齐实际 runner 证据。
- macOS Intel/Apple Silicon 已固定进入 release 并使用 Ad-Hoc 签名；这满足当前功能分发策略，但不建立
  Developer ID、公证或 Gatekeeper 公共信任。首次 tag 运行仍需补齐两架构产物与 updater 的 runner 证据。
- Tauri 2.11 的默认 linuxdeploy 路径仍会生成包含 `libwayland-*` 的原始 AppImage；Clippy 正式
  release workflow 已通过 finalization 修复并重签名，但手工分发未经 finalization 的原始产物仍不受支持。
- Windows 私有文件 ACL 已由原生 runner 在 NTFS 临时目录执行文件/目录修复测试并通过；真实安装后的
  旧配置升级、系统 keyring 与连续 Portal token 更新仍属于 Windows 10/11 实机矩阵，keyring 失败不会
  退回明文。
- renderer v2 的固定字体、调整、效果和矢量金图已由 Linux、Windows 与 macOS 原生 runner 执行同一
  RGBA 摘要测试并通过；跨系统手工打开、继续编辑和导出可编辑 PNG 仍需实机记录。旧 renderer v1
  未修改时继续复用 IDAT，第一次真实编辑才升级到 v2。

## 画布与可编辑 PNG 结论

当前混合方案正确，且比“只保存操作、需要时再渲染”更适合剪贴板场景：

1. 运行期以 `Arc` 共享可信原图，避免重复复制；进程指针不写入文件。
2. 所有标注与调整都使用原图像素坐标，补偿缩放图只负责显示。
3. 保存可编辑 PNG 时，标准 IDAT 写入最新合成像素，普通图片软件与快速粘贴无需重放操作。
4. 压缩 iTXt `clippy-project` 同时内嵌原始 PNG、尺寸/哈希、标注、调整、schema 和渲染器版本，
   重开后可继续编辑且不依赖原路径、数据库行或进程生命周期。v3 还以宽高和解码后的 RGBA
   sha256 绑定 IDAT 合成像素：移植到另一张图的工程块会安全降级成扁平 PNG；同像素、不同压缩编码
   仍可读取。旧 v2 保持可读，并在下一次可编辑保存时升级为 v3。
5. 未修改工程的显示、Copy 与再次保存直接复用已验证的 IDAT，不因操作系统字体或 WebView Canvas
   实现不同而发生无意义重渲染；开始编辑后才把原图和操作层交给 renderer v2 生成新合成像素。
6. renderer v2 的 Copy、扁平保存和可编辑保存共用同一次后端权威合成语义；扁平导出移除工程数据，
   Copy 只复制合成像素，editable 文件则同时保存合成 IDAT 与可继续编辑的 iTXt 工程。

不可把“原图指针”理解为持久化内存地址、临时路径或 clip id：它们在重启、清理、移动和分享后都会失效。
若未来大图导致单文件成本过高，可增加内容寻址资产仓库（hash + 外部引用），但可编辑文件仍应保留内嵌
fallback 或提供显式打包操作，不能默认牺牲可移植性。

隐私边界：可编辑 PNG 的 iTXt 包含未打码原图；模糊/马赛克只改变合成预览。发送给他人前必须使用
“导出扁平 PNG”。当前实现是标注/操作对象层，不是完整 PSD，不承诺组、蒙版、混合模式或任意滤镜插件。
