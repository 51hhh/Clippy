# 跨平台兼容验证记录

日期：2026-09-02

审查基线：`621e36f289a386dcf84d26c77742f6b0f1eae31e`

已审查至：`0d9884ce8f93609c8d8e6a73212bf8819a5a4e32`

## 审查范围

- 审查基线之后 80 个提交、121 个变更文件，以及提交时仍存在的工作区内容。
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
| `cargo test` | 393 通过、0 失败、6 个真实桌面/诊断测试忽略 |
| GNOME 扩展静态检查 | 通过，Shell 45–51 |
| `npm ci` | 通过，0 个已报告漏洞 |
| `tsc --noEmit` | 通过 |
| Vitest | 41 个文件、771 项测试通过 |
| DOM/Xvfb smoke | 9 项通过 |
| Canvas 导出像素 smoke | 通过，抽样像素 `0 208 0` |
| 主窗口布局像素 smoke | 通过，抽样像素 `0 208 0` |
| Vite production build | 通过，1887 个模块完成转换 |
| AppImage X11 可视 smoke | 未执行；需 `CLIPPY_APPIMAGE_SMOKE=1` 和真实 AppImage |

受限沙箱内首次运行时，16 个回环 HTTP 测试因禁止 `TcpListener::bind` 失败，Vite/Xvfb smoke
也无法绑定端口或显示；在允许回环网络和虚拟显示的同一主机上复跑后全部通过。该差异属于执行环境，
不是断言失败。

附加静态检查：

- 四份 Tauri JSON 配置均可解析，两份 GitHub Actions YAML 均可解析。
- Linux 默认依赖图不含 `pipewire-rs`。
- Windows/macOS 目标依赖图不含 GTK、WebKitGTK、zbus、ashpd、libwayshot、PipeWire、inotify、
  x11rb 或 Linux `nix` 实现。
- `git diff --check` 通过。

## 原生平台验证状态

当前 `dev` 比 `origin/dev` 多 106 个本地提交。GitHub 上最近一次 `build.yml` 成功运行是
2026-08-30 的 `8f6c1b57ad844ebc98254b9f88b16e88f3cce314`，不包含本轮改动。

- Windows MSVC：Linux 主机缺少 MSVC `lib.exe`，交叉 `cargo check` 在原生工具链依赖处停止；
  这既不是项目源码失败，也不能作为 Windows 通过证据。
- macOS：Linux 主机没有 Apple SDK/clang，交叉 `cargo check` 在 `-arch`/SDK 参数处停止；
  这既不是项目源码失败，也不能作为 macOS 通过证据。
- 发布前必须把本分支推送到远端，让 `windows-latest` 与 `macos-latest` 原生 runner 完成
  check、clippy、test、前端测试、类型检查和构建。

## 实机 QA 矩阵

“待测”不得在没有目标系统证据时改成“通过”。

| 平台/会话 | 必测场景 | 状态 |
|---|---|---|
| Ubuntu 22.04 GNOME 42 X11 | 文本/HTML/图片、快捷键、自动粘贴、区域/窗口截图、混合 DPI、Pin、画布、可编辑 PNG、OCR、keyring、更新 | 待测 |
| Ubuntu 22.04 GNOME 42 Wayland | 上述场景；另测 Portal 授权/恢复、GNOME Helper 未装/待注销/已就绪、窗口层级降级 | 待测 |
| Ubuntu 24.04 GNOME Wayland | 原生 Wayland 截图 fallback、快捷键、Pin、Portal 版本差异 | 待测 |
| KDE Wayland | Portal 截图、缺少窗口几何和绝对定位时的 UI/reason code | 待测 |
| wlroots compositor | 区域截图、data-control、窗口枚举缺失与 copy-only 降级 | 待测 |
| Windows 10 22H2 / Windows 11 | 普通与管理员目标自动粘贴、UIPI 降级、混合 DPI/多屏、topmost、截图、安装/更新 | 待测 |
| macOS Intel / Apple Silicon | TCC 未决定/拒绝/允许/撤销、Spaces、全屏辅助窗口、截图、粘贴、签名/公证/更新 | 待测 |

## 已修复的审查问题

- Windows 无法用 Unix `rename` 原子覆盖已有配置：改用 `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)`。
- 截图目录 fallback 和文件名只适配 Unix：增加 Windows home 变量、非法字符、尾随点/空格及保留设备名处理。
- 自动启动开发二进制判断只识别 `/target/`：改为平台路径规范化判断。
- 设置页的 OCR 与保存路径提示写死 Linux：改为平台中性文案和系统 Pictures 目录语义。
- `CHANGELOG.md` 仍宣称停止 Ubuntu 22：改为与依赖图、CI 和 release 一致的 Ubuntu 22 基线。
- 发布文档混淆 updater `.sig` 与 Authenticode：明确 Windows 可信代码签名仍是发布前置项。

## 已知限制与发布阻断项

- 非 GNOME Wayland 尚未实现 GlobalShortcuts Portal；注册失败时会提示用户在系统设置中配置，
  不能宣称所有 Wayland 桌面都有应用内全局快捷键。
- Wayland compositor 可以拒绝窗口枚举、绝对定位和永久置顶；此时区域截图仍是保底能力。
- Windows `SendInput` 不能越过 UIPI 控制更高完整性目标；设计行为是复制成功后降级为 copy-only。
- macOS 屏幕录制与辅助功能受 TCC 控制，必须覆盖未决定、拒绝、允许和撤销四种实机状态。
- OCR 目前依赖系统 PATH 中的 Tesseract，尚未捆绑签名 sidecar；安装按钮仅在 Linux 展示。
- Windows updater 产物有 Tauri updater 签名，但 workflow 尚未接入 Authenticode 证书。
- Windows 私有文件当前依赖应用数据目录继承的用户 ACL，尚无显式 ACL 加固验证；keyring 失败不会退回明文。

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
