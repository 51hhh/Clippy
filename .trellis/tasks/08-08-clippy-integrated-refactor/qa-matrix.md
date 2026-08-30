# 综合重构 QA 矩阵

更新日期：2026-08-30

## 自动化证据

2026-08-30 复跑（UI 状态机 + 截图链路对齐之后）：`./scripts/ci-local.sh` = 11 通过 / 0 失败 /
1 跳过（新增"主窗口布局像素 smoke"一步；跳过项仍是需 `CLIPPY_APPIMAGE_SMOKE=1` 显式开启的
AppImage 可视 smoke）。下表 Rust/前端测试数量已按当次结果更新。
deb 一行已在 08-30 用 `cargo tauri build` 重新构建（顺带验证修好的 `beforeBuildCommand`）；
AppImage 一行仍是 08-13 的证据，未在 08-30 重新构建。

| 范围 | 命令/证据 | 结果 |
|---|---|---|
| Rust 格式 | `cargo fmt -- --check` | 通过 |
| Rust 编译 | `cargo check --all-targets` | 通过 |
| Rust 测试 | `cargo test --all` | 235 passed；新增失焦豁免纯函数、编辑器窗口尺寸（1:1/HiDPI/工作区/异常 scale）、`capture_commit_action` 默认与归一化、覆盖层 payload 下发提交动作；新增 GNOME 自定义快捷键条目认领（`plan_slots`）与 X11 逐动作注册计划测试；新增领域错误码稳定性/文案一致性测试，含截图动作错误/竞态清理、Portal token 阶段状态机与翻译 provider 回环测试 |
| Rust lint | `cargo clippy --all-targets -- -D warnings` | 通过 |
| 本地敏感文件权限 | Rust Unix 回归测试 | `config.json`、`clips.db`、`-wal`、`-shm`、Portal token 均为 `0600`；旧配置/数据库宽松权限可修复 |
| 前端类型 | `npx tsc --noEmit` | 通过 |
| 前端测试 | `npx vitest run` | 33 files / 557 passed（含 16 个编辑器工具的几何/绘制指令/交互覆盖、锁定侧栏三组成员的分组测试，以及新增的 codec 面板真实 DOM 测试与覆盖层提交/退化提示交互测试；另有 codec 收藏星星两态切换与中文文案测试，加一条结构守卫：新增操作不挂 `data-i18n` 直接失败） |
| 前端构建 | `npx vite build` | 通过，5 个窗口入口均生成 |
| X11/DOM smoke | `./scripts/smoke-dom.sh` | 1 file / 10 passed（Xvfb） |
| 主窗口布局像素 smoke | `./scripts/smoke-layout.sh`（headless Firefox + 像素读取，视口 780×500 = 预览展开时的真实逻辑尺寸） | 通过，pixel=0 208 0；断言翻译区与预览内容共用同一列且不溢出预览面板、`.preview-content` 不被压到 96px 以下、codec 侧栏打开后列表宽度不变。失败时把断言原因画进红色浮层，避免"fixture 没跑"与"断言不成立"混淆 |
| Canvas 导出像素 smoke | `./scripts/smoke-canvas-export.sh`（headless Firefox 149 + 像素读取） | 通过，pixel=0 208 0；覆盖裁剪/调整/圆角遮罩、矢量标注合成、高亮半透明与聚光灯压暗。缺 ffmpeg 时改用 python3-pil 读像素，两者都没有才跳过 |
| Release X11 startup | 最终 AppImage 解包后的 `AppRun` + `dbus-run-session` + `xvfb-run`，临时 HOME/XDG，12 秒超时 | 进程持续运行至预期超时，无提前崩溃；无完整桌面环境产生的 PipeWire/EGL/user-systemd 警告不等同视觉验收 |
| 截图动作生命周期 | Rust 单元测试 | crop/action 失败会精确结束本代会话、关闭覆盖层并恢复源窗口；并发取消/双动作无法在失去会话所有权后继续产生副作用 |
| 翻译 provider 回环集成 | `cargo test translation::service::tests`（本地临时 TCP mock） | 8 passed，覆盖 Libre/OpenAI 路径、请求体和认证头 |
| 翻译 HTTP 错误归类 | `cargo test translation::http` | 11 passed；4xx 正文限读 4 KiB 后把"缺少/无效 key"从不透明 `http_status` 中还原，5xx 正文不参与判定 |
| npm 依赖安全 | `npm audit --json` | 0 vulnerabilities |
| deb | `cargo tauri build --bundles deb --no-sign --ci`（08-30 重跑） | exit 0，`Clippy_0.1.16_amd64.deb` 5,326,260 bytes；同时验证 `beforeBuildCommand` 的 cwd 无关钩子在真实 CLI 上执行（日志有 `{ cd ../src || cd src; } …`，无 `can't cd`） |
| 构建/开发钩子 | `cargo tauri dev`、`npx @tauri-apps/cli@^2 dev`（cwd = 仓库根，原来必挂的那条路） | 两条路都跑通：vite 起在 1420、`clippy-app` 拉起、无 `can't cd`。CLI 版本 2.11.4，cargo 侧与 `src/` 的 devDependency 一致，不再依赖 npx 缓存 |
| AppImage | `scripts/finalize-appimage.sh` + `scripts/smoke-appimage-x11.sh` + 最终文件独立 SquashFS 解包/`ldd` 检查 | 85,117,432 bytes，x86-64 ELF，依赖无缺失，镜像内 `.DirIcon -> Clippy.png`，X11 smoke 通过，本地未签名 |

产物校验：

- deb SHA-256（08-30 重构建）: `d83e3266584122b6c4b36e673712f93aca99384b63c22c6e8b3e285594b577dc`
- AppImage SHA-256: `026e4cc2b2c40de1467cee7abef318d1e1b54f0c465224d02aa832c78128fd43`
- 本地未配置 `TAURI_SIGNING_PRIVATE_KEY`，所以 updater 签名未生成；release workflow 已从 GitHub Actions secret 注入签名密钥。

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
| 截图提交即进编辑器 | 框选松手后直接弹出编辑器窗口，窗口按截图尺寸开（1:1，不超工作区）；松手时按住 Alt 或把设置改成 Show Toolbar 时停在覆盖层工具条上，选区翻译仍可用 | 待真实桌面验证 |
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
