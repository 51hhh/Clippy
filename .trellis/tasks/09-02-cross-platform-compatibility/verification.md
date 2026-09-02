# 跨平台兼容验证记录

日期：2026-09-02

审查基线：`621e36f289a386dcf84d26c77742f6b0f1eae31e`

代码与 CI 已审查至：`2e0dea493489f5e9fb65c7ab9832668daa313d6c`

## 审查范围

- 截至上述审查点，审查基线之后共 115 个提交、133 个变更文件，以及提交时仍存在的工作区内容。
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
| `cargo test` | 413 通过、0 失败、6 个真实桌面/诊断测试忽略 |
| GNOME 扩展静态检查 | 通过，Shell 45–51 |
| `npm ci` | 通过，0 个已报告漏洞 |
| `tsc --noEmit` | 通过 |
| Vitest | 41 个文件、781 项测试通过 |
| DOM/Xvfb smoke | 9 项通过 |
| Canvas 导出像素 smoke | 通过，抽样像素 `0 208 0` |
| 主窗口布局像素 smoke | 通过，抽样像素 `0 208 0` |
| Vite production build | 通过，1888 个模块完成转换 |
| AppImage X11 可视 smoke | 通过；本机构建产物与 Ubuntu 22 finalized 产物均验证窗口、单实例回调和首帧 |

受限沙箱内首次运行时，16 个回环 HTTP 测试因禁止 `TcpListener::bind` 失败，Vite/Xvfb smoke
也无法绑定端口或显示；在允许回环网络和虚拟显示的同一主机上复跑后全部通过。该差异属于执行环境，
不是断言失败。

附加静态检查：

- 四份 Tauri JSON 配置均可解析，两份 GitHub Actions YAML 均可解析。
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
- 原生 CI 增加真正的 Tauri bundle smoke：关闭 updater 附加产物和代码签名，但不关闭 bundler；
  Windows 必须生成 NSIS/MSI，macOS 必须生成 app/DMG。本机已用同一 CI 配置完成
  `tauri build --debug --no-bundle`，验证配置合并、前端钩子和应用构建；安装包仍必须由原生 runner 生成。
- 已按仓库锁定的 Tauri CLI 2.11.4 / bundler 2.9.4 复核 updater 产物：`true` 模式下 Linux
  AppImage 与 Windows NSIS/MSI 是自包含更新器并直接签名，macOS 使用 `app.tar.gz`；工作流收集的
  文件名和 `linux/windows/darwin` 四个 `OS-ARCH` manifest key 与该规则一致。旧式 zip/tar 规则仅
  属于已弃用的 `v1Compatible` 模式。本结论是源码/配置审查，产物仍需原生 release job 验证。
- `git diff --check` 通过。

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

## 原生平台验证状态

截至上述审查点，`dev` 比 `origin/dev` 多 141 个本地提交。GitHub 上最近一次 `build.yml` 成功运行是
2026-08-30 的 `8f6c1b57ad844ebc98254b9f88b16e88f3cce314`，不包含本轮改动。

- Windows MSVC：完整工程在 Linux 主机交叉检查时缺少 `llvm-rc`、MSVC `lib.exe`/SDK 以及
  `ring`/SQLite 所需原生构建工具，因而在依赖构建阶段停止；私有文件模块的独立 MSVC 目标检查已
  通过；自动粘贴使用的 `OpenProcess`、令牌、SID 与完整性 RID API 也通过独立 MSVC 目标类型检查，
  但这些都不能作为 Windows 原生运行通过证据。
- macOS：Linux 主机没有 Apple SDK/clang，交叉 `cargo check` 在 `-arch`/SDK 参数处停止；
  这既不是项目源码失败，也不能作为 macOS 通过证据。
- 发布前必须把本分支推送到远端，让 `windows-latest` 与 `macos-latest` 原生 runner 完成
  check、clippy、test、前端测试、类型检查，以及新增的 NSIS/MSI、app/DMG bundle smoke。

## 实机 QA 矩阵

“待测”不得在没有目标系统证据时改成“通过”。

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
- macOS 构建仅依赖 bundler 成功返回：上传前现在独立核对 Developer ID authority、Hardened Runtime、
  严格代码签名、Gatekeeper assessment 和 stapled notarization ticket。
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
- Windows Authenticode workflow 已接入，但仓库尚未提供可由本任务读取的证书 secret，也未在原生
  release runner 上执行；必须配置 `WINDOWS_CERTIFICATE*` 并取得 NSIS/MSI 签名验证通过的运行证据。
- macOS 最终产物验证门禁已接入，但仍需带真实 Developer ID 和公证凭据的 release runner 运行证据。
- Tauri 2.11 的默认 linuxdeploy 路径仍会生成包含 `libwayland-*` 的原始 AppImage；Clippy 正式
  release workflow 已通过 finalization 修复并重签名，但手工分发未经 finalization 的原始产物仍不受支持。
- Windows 私有文件 ACL 已有显式实现、目标编译测试和单元测试，但仍需 Windows 原生 runner 执行
  ACL 测试，并在 NTFS 实机确认旧文件修复、目录继承与连续 token 更新；keyring 失败不会退回明文。

## 画布与可编辑 PNG 结论

当前混合方案正确，且比“只保存操作、需要时再渲染”更适合剪贴板场景：

1. 运行期以 `Arc` 共享可信原图，避免重复复制；进程指针不写入文件。
2. 所有标注与调整都使用原图像素坐标，补偿缩放图只负责显示。
3. 保存可编辑 PNG 时，标准 IDAT 写入最新合成像素，普通图片软件与快速粘贴无需重放操作。
4. 压缩 iTXt `clippy-project` 同时内嵌原始 PNG、尺寸/哈希、标注、调整、schema 和渲染器版本，
   重开后可继续编辑且不依赖原路径、数据库行或进程生命周期。
5. 未修改工程的显示、Copy 与再次保存直接复用已验证的 IDAT，不因操作系统字体或 WebView Canvas
   实现不同而发生无意义重渲染；开始编辑后才以原图和操作层生成新的合成像素。
6. 扁平导出移除工程数据；编辑后的 Copy 只复制合成像素。

不可把“原图指针”理解为持久化内存地址、临时路径或 clip id：它们在重启、清理、移动和分享后都会失效。
若未来大图导致单文件成本过高，可增加内容寻址资产仓库（hash + 外部引用），但可编辑文件仍应保留内嵌
fallback 或提供显式打包操作，不能默认牺牲可移植性。

隐私边界：可编辑 PNG 的 iTXt 包含未打码原图；模糊/马赛克只改变合成预览。发送给他人前必须使用
“导出扁平 PNG”。当前实现是标注/操作对象层，不是完整 PSD，不承诺组、蒙版、混合模式或任意滤镜插件。
