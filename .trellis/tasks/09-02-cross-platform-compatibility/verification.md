# 跨平台兼容验证记录

日期：2026-09-02

审查基线：`621e36f289a386dcf84d26c77742f6b0f1eae31e`

代码与 CI 已审查至：`4db4f6743507efb9496c120160def800c24deaa2`

## 审查范围

- 审查基线之后 103 个提交、132 个变更文件，以及提交时仍存在的工作区内容。
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
| `cargo test` | 409 通过、0 失败、6 个真实桌面/诊断测试忽略 |
| GNOME 扩展静态检查 | 通过，Shell 45–51 |
| `npm ci` | 通过，0 个已报告漏洞 |
| `tsc --noEmit` | 通过 |
| Vitest | 41 个文件、774 项测试通过 |
| DOM/Xvfb smoke | 9 项通过 |
| Canvas 导出像素 smoke | 通过，抽样像素 `0 208 0` |
| 主窗口布局像素 smoke | 通过，抽样像素 `0 208 0` |
| Vite production build | 通过，1888 个模块完成转换 |
| AppImage X11 可视 smoke | 未执行；需 `CLIPPY_APPIMAGE_SMOKE=1` 和真实 AppImage |

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
- Windows 私有文件 ACL 与原子替换模块在独立最小工程中完成 MSVC 目标 `cargo check --tests`
  和 `cargo clippy --tests -- -D warnings`；这验证目标条件编译和 Win32 类型调用，不代替原生运行。
- 原生 CI 增加真正的 Tauri bundle smoke：关闭 updater 附加产物和代码签名，但不关闭 bundler；
  Windows 必须生成 NSIS/MSI，macOS 必须生成 app/DMG。本机已用同一 CI 配置完成
  `tauri build --debug --no-bundle`，验证配置合并、前端钩子和应用构建；安装包仍必须由原生 runner 生成。
- `git diff --check` 通过。

## 原生平台验证状态

当前 `dev` 比 `origin/dev` 多 129 个本地提交。GitHub 上最近一次 `build.yml` 成功运行是
2026-08-30 的 `8f6c1b57ad844ebc98254b9f88b16e88f3cce314`，不包含本轮改动。

- Windows MSVC：完整工程在 Linux 主机交叉检查时缺少 `llvm-rc`、MSVC `lib.exe`/SDK 以及
  `ring`/SQLite 所需原生构建工具，因而在依赖构建阶段停止；私有文件模块的独立 MSVC 目标检查已
  通过，但两者都不能作为 Windows 原生运行通过证据。
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
- Windows 私有文件 ACL 已有显式实现、目标编译测试和单元测试，但仍需 Windows 原生 runner 执行
  ACL 测试，并在 NTFS 实机确认旧文件修复、目录继承与连续 token 更新；keyring 失败不会退回明文。

## 画布与可编辑 PNG 结论

当前混合方案正确，且比“只保存操作、需要时再渲染”更适合剪贴板场景：

1. 运行期以 `Arc` 共享可信原图，避免重复复制；进程指针不写入文件。
2. 所有标注与调整都使用原图像素坐标，补偿缩放图只负责显示。
3. 保存可编辑 PNG 时，标准 IDAT 写入最新合成像素，普通图片软件与快速粘贴无需重放操作。
4. 压缩 iTXt `clippy-project` 同时内嵌原始 PNG、尺寸/哈希、标注、调整、schema 和渲染器版本，
   重开后可继续编辑且不依赖原路径、数据库行或进程生命周期。
5. 扁平导出移除工程数据；编辑后的 Copy 只复制合成像素。

不可把“原图指针”理解为持久化内存地址、临时路径或 clip id：它们在重启、清理、移动和分享后都会失效。
若未来大图导致单文件成本过高，可增加内容寻址资产仓库（hash + 外部引用），但可编辑文件仍应保留内嵌
fallback 或提供显式打包操作，不能默认牺牲可移植性。

隐私边界：可编辑 PNG 的 iTXt 包含未打码原图；模糊/马赛克只改变合成预览。发送给他人前必须使用
“导出扁平 PNG”。当前实现是标注/操作对象层，不是完整 PSD，不承诺组、蒙版、混合模式或任意滤镜插件。
