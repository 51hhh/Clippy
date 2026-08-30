# 综合重构 QA 矩阵

更新日期：2026-08-30

## 自动化证据

2026-08-30 复跑（截图改成单窗口覆盖层之后）：`./scripts/ci-local.sh` = 11 通过 / 0 失败 /
1 跳过（跳过项仍是需 `CLIPPY_APPIMAGE_SMOKE=1` 显式开启的 AppImage 可视 smoke）。
下表 Rust/前端测试数量已按当次结果更新（删掉独立编辑器窗口后 Rust 246、前端 603）。
v0.1.17 发布前 deb 与 AppImage 均已按 0.1.17 重新构建（`tauri build --no-sign --ci`，两个 bundle 一次产出）。

| 范围 | 命令/证据 | 结果 |
|---|---|---|
| Rust 格式 | `cargo fmt -- --check` | 通过 |
| Rust 编译 | `cargo check --all-targets` | 通过 |
| Rust 测试 | `cargo test --all` | 246 passed（2 个 `#[ignore]` 诊断测试需 `-- --ignored --nocapture` 手动跑）；新增两条删除原子性测试（用 BEFORE DELETE 触发器打断删除链，断言 clips/clips_fts/translation_history 一起回滚）；新增失焦豁免纯函数；截图链路新增：显示器逻辑尺寸归一化（`normalize_monitor_geometry`）、窗口矩形的 X 像素折算与 `_GTK_FRAME_EXTENTS` 裁边（8 条）、覆盖层显示器选择按最大重叠面积（6 条）、`commit_capture_action` 的 PNG 校验与大小上限、payload 不再下发提交动作、默认能力清单不含已删除的 `capture` 窗口；覆盖层显示时机 3 条（光标所在覆盖层独占焦点、拿不到光标时先画完的拿焦点、非本会话标签被拒）；新增 GNOME 自定义快捷键条目认领（`plan_slots`）与 X11 逐动作注册计划测试；新增领域错误码稳定性/文案一致性测试，含截图动作错误/竞态清理、Portal token 阶段状态机与翻译 provider 回环测试 |
| Rust lint | `cargo clippy --all-targets -- -D warnings` | 通过 |
| 本地敏感文件权限 | Rust Unix 回归测试 | `config.json`、`clips.db`、`-wal`、`-shm`、Portal token 均为 `0600`；旧配置/数据库宽松权限可修复 |
| 前端类型 | `npx tsc --noEmit` | 通过 |
| 前端测试 | `npx vitest run` | 34 files / 603 passed（新增 release 守卫 3 条：下载表的发行版后缀恰好等于矩阵 label、无后缀 updater 产物只从一个 label 上传；新增哈希 vs 可逆编码边界：hex 摘要判成 HASH 3 条 + 同长度 hex 文本仍归编码 1 条 + `decodeReadableBytes` 严格 UTF-8 4 条 + Base64/hex 分支各 4 条；新增回归守卫 4 条：tauri before*Command 从任一 cwd 都能进前端目录、`#codec-output` 是 `<div>`、两个列表行渲染器都不写类型标签、预览面板不自己再嗅探类型；新增源码写死的 i18n key 全量存在性扫描 1 条；新增内容类型判定表 9 条：锁表顺序、kind 唯一、每条规则指向的渲染器存在、只有 JSON/JWT 需要延迟库、判不出来交给异步尾段；新增 codec 多类别结果键值对 6 条与列表行不显示类型 1 条；codec 收藏脏数据自愈 4 条、Shift+Tab 聚焦退化 2 条、下拉 Esc 归属 1 条；含 16 个标注工具的几何/绘制指令/交互覆盖、锁定工具条三组成员并断言工具 id 与标注核心对齐（`crop → select`）的分组测试；覆盖层交互测试改按新流程断言：拖拽/点窗口/点空地各自的选区、松手不提交而是弹工具条、全屏选区仍可重新框选、手柄仍可缩放、对钩把裁剪后的 PNG 交给 `commit_capture_action`、右键丢选区、退化提示；新增显示握手 4 条（首帧画好之前不显示、重绘不重复显示、出错也要显示出来让提示可见、显示失败不拖住截图）；新增覆盖层几何 4 组（`coversBounds`/`toPixelRect`/工具条落点/点空地取整屏）；另有 codec 面板真实 DOM 测试；另有 codec 收藏星星两态切换与中文文案测试，加一条结构守卫：新增操作不挂 `data-i18n` 直接失败） |
| 前端构建 | `npx vite build` | 通过，4 个窗口入口均生成（独立截图编辑器窗口已删除） |
| X11/DOM smoke | `./scripts/smoke-dom.sh` | 1 file / 9 passed（Xvfb） |
| 主窗口布局像素 smoke | `./scripts/smoke-layout.sh`（headless Firefox + 像素读取，视口 780×500 = 预览展开时的真实逻辑尺寸） | 通过，pixel=0 208 0；断言翻译区与预览内容共用同一列且不溢出预览面板、`.preview-content` 不被压到 96px 以下、codec 侧栏打开后列表宽度不变。失败时把断言原因画进红色浮层，避免"fixture 没跑"与"断言不成立"混淆 |
| Canvas 导出像素 smoke | `./scripts/smoke-canvas-export.sh`（headless Firefox 149 + 像素读取） | 通过，pixel=0 208 0；覆盖裁剪/调整/圆角遮罩、矢量标注合成、高亮半透明与聚光灯压暗。缺 ffmpeg 时改用 python3-pil 读像素，两者都没有才跳过 |
| Release X11 startup | 最终 AppImage 解包后的 `AppRun` + `dbus-run-session` + `xvfb-run`，临时 HOME/XDG，12 秒超时 | 进程持续运行至预期超时，无提前崩溃；无完整桌面环境产生的 PipeWire/EGL/user-systemd 警告不等同视觉验收 |
| 截图动作生命周期 | Rust 单元测试 | crop/action 失败会精确结束本代会话、关闭覆盖层并恢复源窗口；并发取消/双动作无法在失去会话所有权后继续产生副作用 |
| 翻译 provider 回环集成 | `cargo test translation::service::tests`（本地临时 TCP mock） | 8 passed，覆盖 Libre/OpenAI 路径、请求体和认证头 |
| 翻译 HTTP 错误归类 | `cargo test translation::http` | 11 passed；4xx 正文限读 4 KiB 后把"缺少/无效 key"从不透明 `http_status` 中还原，5xx 正文不参与判定 |
| npm 依赖安全 | `npm audit --json` | 0 vulnerabilities |
| deb | `tauri build --no-sign --ci`（08-30，版本 0.1.17） | exit 0，`Clippy_0.1.17_amd64.deb` 5,328,002 bytes，`Version: 0.1.17`，依赖 `libayatana-appindicator3-1/libwebkit2gtk-4.1-0/libgtk-3-0`，推荐 `tesseract-ocr`；同时验证 `beforeBuildCommand` 的 cwd 无关钩子在真实 CLI 上执行（无 `can't cd`） |
| 构建/开发钩子 | `cargo tauri dev`、`npx @tauri-apps/cli@^2 dev`（cwd = 仓库根，原来必挂的那条路） | 两条路都跑通：vite 起在 1420、`clippy-app` 拉起、无 `can't cd`。CLI 版本 2.11.4，cargo 侧与 `src/` 的 devDependency 一致，不再依赖 npx 缓存 |
| AppImage | `tauri build --no-sign --ci`（08-30，版本 0.1.17）+ `xvfb-run` 启动 smoke（独立 `XDG_*`，25 秒超时） | `Clippy_0.1.17_amd64.AppImage` 85,174,776 bytes；启动后持续运行至预期超时，无崩溃输出。`scripts/smoke-appimage-x11.sh` 的可视 smoke 因本机缺 ffmpeg 跳过（该脚本无 python3-pil 回退），`.DirIcon` 重封装与签名由 release workflow 执行 |

产物校验：

- deb SHA-256（0.1.17）: `c2ab16a4b9ced5db8882556645c4e204ea852d12150a2409d55689828595fa3c`
- AppImage SHA-256（0.1.17，未经 `finalize-appimage.sh` 重封装）: `199d6b6c0f949c565e3beeecb2d09855f84e7bb09ea4186a78fe812fe52fee25`
- 本地未配置 `TAURI_SIGNING_PRIVATE_KEY`，所以 updater 签名未生成；release workflow 已从 GitHub Actions secret 注入签名密钥。
- CI 系统依赖缺口（v0.1.17 发布前修复）：两个 workflow 都没装 `libpipewire-0.3-dev`，`libspa-sys`（经 `xcap` → `pipewire` 引入）因此 build script 失败，`dev` 分支自 08-26 起 CI 一直红；顺带补上 libwayshot-xcap 链接所需的 `libgbm-dev/libegl-dev/libdrm-dev/libwayland-dev/libxcb1-dev`。
- 首次 tag（release run 33299874854）在 `Finalize portable AppImage` 失败：重封装与 `.DirIcon` 校验都过了，紧接着 `cargo tauri signer sign` 报 `no such command: tauri`——runner 上没有 cargo-tauri。脚本改为优先用 `src/node_modules/.bin/tauri`（lockfile 锁定），并在签名后断言 `.sig` 非空。本机用一次性 `tauri signer generate` 密钥跑通完整 `finalize-appimage.sh`（重封装 + 校验 + 签名 + `.sig` 校验，exit 0），密钥与测试签名已删除。
- 补依赖后 `ubuntu-24.04` 通过、`ubuntu-22.04` 仍在 `cargo check` 失败（run 33292389744）：`libspa 0.9.2` 无条件访问 bindgen 从系统头文件生成的 `spa_video_info_raw.flags`，该字段要 pipewire ≥ 0.3.65，jammy 仓库只有 0.3.48（另有两处 i64/u64 签名不一致）。`xcap 0.9.6` 的 feature 只有 `image` 和 Windows 的 `wgc`，没有关掉 pipewire 的开关；xcap 自 0.5 起每个版本都依赖 pipewire。用 PPA 换新头文件能过编译但会与 22.04 实机的 libpipewire ABI 不一致，因此本版起 CI 与 release 矩阵都只留 `ubuntu-24.04`，release notes、CHANGELOG 升级说明和 docs/CI.md 同步说明。

## 真实桌面人工矩阵

下列项目不能由无交互沙箱代替。当前结果明确标为“待真实桌面验证”，不能由单元测试或 Xvfb 推断为通过。

| 场景 | 验收点 | 当前结果 |
|---|---|---|
| GNOME X11 | 原窗口恢复、Ctrl+V、无 Portal 弹窗、主页面非黑屏 | 待真实桌面验证 |
| GNOME Wayland | 首次授权、同进程会话复用、Copy-only fallback | 待真实桌面验证 |
| GNOME Wayland 重启 | restore token 滚动更新、静默恢复或后端提示 | 待真实桌面验证 |
| Portal 撤权 | 仅一次失败、设置页显式重试、剪贴板仍保留 | 待真实桌面验证 |
| KDE Wayland | Portal 后端兼容、覆盖层、Pin 置顶 | 待真实桌面验证 |
| KDE Wayland 快捷键 | 三个全局快捷键均不注册（gsettings 路径只覆盖 GNOME），设置页应显示"该 Wayland 桌面不托管 Clippy 快捷键，请在系统键盘设置中手动添加" | 待真实桌面验证 |
| GNOME 已有自定义快捷键 | 事先在设置里建 3 个自定义快捷键（占满 custom0/1/2）再启动 Clippy：用户那三个的 name/command/binding 必须原样保留，Clippy 用 custom3/4/5；重启 Clippy 不再新增条目（按 command 复用） | 待真实桌面验证 |
| codec 侧栏下拉框 | `` ` `` 打开左侧栏后点操作下拉框：窗口不隐藏、不闪退（原生 `<select>` 已换成 custom-select，Rust 侧另有 `codec_visible` 失焦豁免兜底） | 待真实桌面验证 |
| 侧栏键盘归属 | codec 打开时字母/数字打进输入框且不驱动列表，`` ` ``/`Esc` 关侧栏并把焦点还给列表；Tab 打开预览后 `↑↓`/`ws` 仍然翻列表而不是滚翻译区 | 待真实桌面验证 |
| 主窗口高度恒定 | Tab 反复开关右侧预览：窗口高度不变（380×500 → 780×500），列表可见行数一格都不变；翻译区在预览列内自己滚动，不遮挡预览内容 | 待真实桌面验证 |
| codec 收藏星星 | 左侧栏顶部星星：未收藏是描边、点一下变实心并把当前操作加进"收藏"分组，再点取消；切换操作时星星跟着该操作的收藏状态；重启应用后收藏仍在 | 待真实桌面验证 |
| codec 面板语言 | 设置切到中文后左侧栏立刻变中文（不用重启）：操作名如"Base64 解码"、分组标题"编码/收藏"、按钮提示"反向操作/复制结果/加入收藏"、输入框占位"输入…"；ROT13/MD5/SHA-256/JWT 等专有名词保持原文 | 待真实桌面验证 |
| 截图全程只有一个窗口 | 按快捷键后覆盖层铺满**当前**显示器（多屏/混合缩放各试一次，画面不是黑的）；点空地取整屏、悬停窗口点一下取该窗口、拖拽取自由区域；三种情况都不结束截图，工具条贴在选区旁，选区仍可拖动与缩放；标注后点对钩，剪贴板里是裁剪且带标注的图；不再出现独立的编辑器窗口 | 待真实桌面验证 |
| 覆盖层没有白屏 | 按快捷键后除系统截图那一下的闪白外，不应出现"整屏白色几秒再出画面"：覆盖层隐藏建窗，前端画完首帧才显示（`mark_capture_overlay_ready`），底色为黑 | 待真实桌面验证 |
| Wayland 窗口速选退化 | 合成器不给窗口几何时覆盖层顶部显示 "Window picking unavailable in this session"，日志有对应 info，拖拽选区不受影响 | 待真实桌面验证 |
| deb 实装 | 主窗口渲染、托盘、截图、Pin、设置 | 未修改系统，待实装验证 |
| AppImage 实机 | 主窗口渲染、托盘、截图、Pin、更新器 | 待真实桌面验证 |
| Secret Service | API key 保存/查询/删除且配置文件无明文 | 待真实服务验证 |
| LibreTranslate-compatible | 实际请求、超时、复制结果 | 需要可用测试端点 |
| OpenAI-compatible | 实际请求、模型/key、结构化错误 | 需要可用测试端点与 key |

## 人工操作边界

- Portal 首次确认和撤权必须由用户操作，自动化不得代答。
- 不安装 root/uinput 常驻服务，不修改用户组、Polkit 或系统安全策略。
- deb 实装会修改系统状态，因此未在无人值守沙箱内执行。
