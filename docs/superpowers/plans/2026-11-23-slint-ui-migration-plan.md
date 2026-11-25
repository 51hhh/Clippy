# Slint UI 迁移执行计划

Requirement ID: [`SLINT-UI-MIGRATION-01`](../specs/2026-11-23-slint-ui-migration.md)

继续开发见[新版实现计划](2026-11-24-slint-native-implementation-plan.md)。后续按新版计划的串行顺序实施，
本页保留迁移决策、M0–M9 里程碑和既有证据。

## 已确认决策

| 项目 | 决策 |
|---|---|
| 开发方式 | 当前分支串行开发，不维护长期双分支 |
| 目录 | legacy Tauri 与 Slint 分目录共存，共享 crate 独立 |
| 版本 | legacy 保持 v0.1.x；Slint 从 v0.2.0 开始 |
| 预发布 | v0.2.0-alpha/beta/rc 全部走 Slint |
| 正式身份 | v0.2.0 复用现有 app/bundle ID、配置与数据库目录 |
| QA 身份 | 独立 app/bundle ID 和数据目录，不能与正式版并行运行 |
| 旧版维护 | 允许必要的小修和安全修复，不继续大改；维护发布不能覆盖稳定更新入口 |
| Slint 许可 | royalty-free desktop license，补齐 About 与第三方归属 |
| 框架基线 | legacy Tauri 2.12.4；Slint 1.19.0 |

## 目标目录

```text
Cargo.toml
Cargo.lock
apps/
  legacy-tauri/
    frontend/                 # 现 src/
    native/                   # 现 apps/legacy-tauri/native/
  slint/
    Cargo.toml                # package version = 0.2.0
    build.rs
    src/
    ui/
    Packager.toml
crates/
  clippy-core/                # 配置与领域模型
  clippy-platform/            # 身份、路径，后续接系统服务
  clippy-storage/             # SQLite、FTS5 和存储工作线程
scripts/
docs/
```

`fxxkDJTU/` 是独立项目，不进入根 workspace，也不参与本迁移。

## 发布路由

唯一入口为 `scripts/resolve-release-backend.mjs`：

| tag | backend | framework |
|---|---|---|
| `v0.0.x` / `v0.1.x` | `legacy-tauri` | Tauri 2.12.4 |
| `v0.2.0-alpha.*` / `beta.*` / `rc.*` | `slint` | Slint 1.19.0 |
| `v0.2.0` 及以上 | `slint` | Slint 1.19.0 |

正式 release 生成 `BUILD-INFO.json`，至少包含：

```json
{
  "appVersion": "0.2.0",
  "uiBackend": "slint",
  "framework": "Slint",
  "frameworkVersion": "1.19.0",
  "renderer": "femtovg-or-skia",
  "gitSha": "<40-char SHA>"
}
```

legacy 产物必须明确写 `Slint: not included`，Slint 产物必须明确写 `Tauri: not included`。

## 串行阶段

### M0：冻结当前基线并建立规格

- [x] 固化 Goal / Requirements / Acceptance Criteria / Out of Scope。
- [x] 记录版本、tag、正式身份、旧版维护和许可决策。
- [x] 对当前未提交 GNOME 截图修复执行完整本地门禁，记录失败和跳过，不把既有失败归因给迁移。
- [x] 记录当前静置进程树 PSS、RSS 与动态库基线。
- [ ] 使用固定启动定义补测启动时间；不得用主窗口首次显示时间替代托盘就绪时间。

退出条件：当前工作树的真实基线可复现，迁移前问题与迁移引入问题可区分。

### M1：可测试的版本路由

- [x] 实现纯 Node semver tag 解析器，不引入 npm 运行时依赖。
- [x] 覆盖 v0.1、v0.2 预发布、v0.2 正式、更高主次版本和非法 tag。
- [x] release workflow 从解析器读取 `version/backend/framework/framework_version`，不在 YAML 重复判断。
- [ ] legacy 维护 release 在 v0.2.0 上线后标记为非 latest，不覆盖稳定 updater 元数据。

退出条件：路由测试通过，两个版本区间不会同时进入打包 job。

### M2：目录共存与根 workspace

- [x] 把原 `src/`、`src-tauri/` 移到 `apps/legacy-tauri/`；该步骤不改业务逻辑。
- [x] 批量更新脚本、workflow、配置、文档和测试里的路径，并让 legacy 完整门禁恢复绿色。
- [x] 建立根 `Cargo.toml` / `Cargo.lock`，纳入 legacy 与 Slint；vendor 相对路径统一从 workspace
  根解析。
- [x] 新建最小 `apps/slint` 0.2.0 应用，固定 Slint 1.19.0，仅显示可关闭的空壳窗口。

退出条件：两套入口可在同一 checkout 独立 check，legacy 行为与搬迁前一致。

### M3：版本化 CI 与 QA 身份隔离

- [x] Linux 与 Windows 本地门禁拆成 core、legacy、Slint 三段；默认 `current` 只跑 core + Slint，
  `all`/`legacy` 作为维护入口显式构建旧版。
- [x] 保留 `Check (ubuntu-22.04)`、`Native Check (windows-latest)`、
  `Native Check (macos-latest)` 名称，普通 CI 只编译 core/platform/Slint；legacy 录屏原型改为手动开关，
  workflow 合同已实现，远程同 SHA 结果仍待验证。
- [x] Native QA workflow 保留 `qa-legacy-*` 和 `qa-slint-*`，默认只生成 Slint；选择 `legacy` 或 `all`
  才构建旧版。单一生成器写入 backend、framework、renderer、SHA 与未包含框架。
- [x] Slint QA 建立独立 app/bundle ID、可执行文件和桌面入口；生产入口显式绑定原身份，不依赖
  feature 切换身份。
- [ ] 平台服务接入后实际消费 QA 的快捷键、单实例和数据命名空间，并验证与正式版互不污染。

退出条件：同一 SHA 三平台验证 core/platform/Slint；显式 legacy 维护入口和两类 QA 包均不会污染正式数据。

### M4：共享核心抽离

- [x] 首批抽出 `ContentType`、`ClipItem`、`TranslationHistoryEntry`、`UrlMeta` 与产品标识；
  legacy 通过原模块重导出保持调用路径，Slint 已消费共享标识，共享 crate 不依赖 Tauri 或 Slint。
- [x] 抽离配置值对象与持久化边界：`clippy-core::config` 拥有 `AppConfig`、目标平台快捷键默认策略、
  v1→v2 迁移、JSON codec 与结构化提交/回滚协议；legacy 只保留私有文件权限和 Tauri 副作用适配。
  Slint 的实际数据目录装配随平台服务接入，不在本项伪造一套临时路径。
- [x] 抽离数据库与搜索：`clippy-storage` 拥有现有 SQLite/FTS5 schema、迁移、搜索、归档、图片修订和
  Pin 工作区持久化；连同专用线程边界共 45 项存储测试随适配器迁移。`clippy-platform::paths` 与旧
  Tauri 数据目录合同对齐，Slint 通过容量为 8 的专用存储线程异步加载最近历史，UI 线程不持有
  SQLite 连接。

按“可独立测试、无窗口类型”的顺序迁移：

1. `models`、`config`、数据库与搜索；
2. 剪贴板监听、动作与敏感内容分类；
3. 截图/编辑 renderer、OCR、翻译；
4. 录屏 session、manifest、恢复和媒体目录；
5. 托盘、快捷键、单实例、窗口策略等平台适配。

约束：共享 crate 禁止依赖 `tauri::AppHandle`、`tauri::WebviewWindow` 或 Slint component；host 通过窄接口
注入窗口/对话框/通知能力。抽离时以原 legacy 行为与测试作为输入，但新能力直接落入分层共享 crate，
Slint 首先接入；legacy 只保留必要适配和显式维护验证，避免为兼容旧 host 复制一套新架构。

退出条件：Slint 所需业务能力不需要调用 Tauri IPC；legacy 维护入口仍可按需独立重建和验证。

### M5：Slint 设计系统与窗口骨架

- [ ] 建立颜色、排版、间距、圆角、阴影、动效和高对比度 token；六个现有主题映射为同一语义层。
- [ ] 实测 FemtoVG 与 Skia：Linux/Windows/macOS 中文、透明窗口、阴影、多屏缩放、启动时间、空闲 PSS。
- [ ] 仅根据数据选一个正式 renderer；software renderer 不进入桌面中文正式路径。
- [ ] 建立 main、settings、capture、pin、library 等窗口控制器，避免默认一次实例化所有 component。
- [ ] 托盘常驻但无可见窗口时不创建 WindowAdapter；首次打开再创建对应 component。

退出条件：空壳托盘态接近最终生命周期，窗口按需创建/销毁且无泄漏。

#### 第一轮界面对照

- Goal：先恢复旧版主剪贴板和设置的窗口尺寸、信息密度与操作位置。重用六套主题和窗口标题栏。
- Requirements：主窗口以 380×500 的整宽列表、选中条和底部收藏/全部为基准。搜索、行操作与键盘入口继续可用。设置以 720×560 的可拖动标题栏、General / Screenshot / Translation / About 分页为基准，设置项沿用原有保存接口。
- Acceptance Criteria：用相同合成历史对照旧版和 Slint 截图，核对浅色、深色及窄窗口。验证搜索、历史复制、收藏分页、设置切页与关闭，再运行 Linux 桌面回归。
- Out of Scope：本轮暂缓预览侧栏、截图编辑、贴图和录屏窗口的视觉对齐；采集、存储、翻译协议和历史数据模型沿用现有实现。

旧版主窗口用整宽的 77px 列表行、左侧选中条和底部分段。新版把行、搜索和筛选放进同一个原生视图，复用 Slint 的虚拟列表和六套主题。设置标题栏沿用旧版的 52px 高度，主面板和设置按各自窗口尺寸打开。

隔离 X11/Openbox 的浅色、深色场景都验证了 380×500 主窗口、720×560 设置窗口、拖动后逐页点击、最大化、还原、最小化和从托盘重新打开。设置缩到 480×430 时仍可滚动。原先标题栏处理按下事件后，窗口管理器接收释放事件，导致设置控件接不到后续点击。现在原生拖动接管按下事件，界面控件保留正常点击。
隔离 GNOME Wayland 验证了设置窗口尺寸。虚拟指针连关闭按钮的对照点击都没有送到应用，拖动仍需实际桌面验收。

| 旧版入口 | Slint 当前界面与需要对齐的部分 |
| --- | --- |
| 剪贴板、设置 | 已对齐默认尺寸、列表、原生预览侧栏、标题栏、分页、主题与快捷键录制。历史列表显示可见图片缩略图，文本工具显示原文和译文双栏。其他旧版设置项仍需核对。 |
| 截图、图片查看器 | 已有选区随动操作栏、像素尺寸、选区内矩形和原生编辑器。其他内联工具与查看器入口仍需对齐。 |
| 启动器 | 已接入原生命令面板、键盘导航和带选中项的图片及文本动作。 |
| 贴图、工作区 | 已对齐主题和工作区分组，贴图浮窗可拖动内容区并从侧边调整呈现。缩略图和各平台层级仍待对照。 |
| 长截图指南、控制器 | 一个原生窗口显示引导、当前拼接预览和操作。已对齐主题、关闭与 Esc 取消。 |
| 录屏控制、录屏库 | 录屏库和托盘共用控制，窗口按实际暂停、继续及封尾状态更新。独立控制浮窗仍待接入。 |

静态布局和控件样式可复用同一套 Slint 主题。富文本、代码高亮和图片翻译需要对应的原生视图与交互；窗口置顶、缩放和拖动按各窗口管理器验收。

#### 剪贴板预览侧栏

需求 ID `SLINT-HISTORY-PREVIEW-01`。

- Goal：恢复选中历史时在同一窗口查看文字和图片的操作，默认保留 380×500 的紧凑布局。
- Requirements：列表 Tab 或行菜单 View 打开，Esc 或关闭按钮收起。展开时列表选中状态和滚动位置保留，侧栏跟随当前选中项。文字和富文本提供可选中的纯文本，图片在打开侧栏后才读取和缩放。敏感项只显示保护状态。详情加载期间不接受上一条的复制、编辑或工具操作。
- Acceptance Criteria：隔离 X11 核对收起与展开的尺寸，使用 Unicode、HTML、透明 PNG 和敏感样本验证切换、复制、关闭和重新打开。单元测试覆盖图片解码边界和异步选择更新，再运行新版完整本地门禁。
- Out of Scope：HTML 富样式、Markdown 排版、列表缩略图、其他窗口的布局和采集协议改动。

侧栏跟随选中项，空间从 380×500 扩到 780×500。每次最多读取和解码一项，预览读取上限为 16 MiB PNG，解码器分配上限为 48 MiB，显示图像长边最多 800px。收起或关闭窗口后丢弃未完成结果。文字支持选择和复制，富文本按纯文本显示。屏幕像素、键盘和剪贴板回归在隔离 X11 运行，其他桌面环境的窗口尺寸与焦点行为待实机验收。

#### 原生动作启动器

需求 ID `SLINT-LAUNCHER-01`。

- Goal：恢复托盘和剪贴板共用的可搜索动作窗口，集中查找截图、图片和文本操作。
- Requirements：托盘 Actions 和剪贴板 Ctrl/Cmd+K 打开同一个原生窗口，支持搜索、方向键、Enter 和 Esc。剪贴板入口使用打开时选中的项目，托盘在显式选择图片动作时使用最新图片。敏感选择不传给动作窗口，识字和扫码仅消费有界本地图片，翻译沿用已有显式提交。
- Acceptance Criteria：用隔离 X11 验证两个入口、搜索空结果、键盘选择、取消、带入 Unicode 文本和打开图像分析窗口。用单元测试覆盖动作过滤和敏感内容。
- Out of Scope：新翻译接口、剪贴板或截图协议更改、全局动作快捷键及自定义命令执行。

#### 设置页快捷键录制

需求 ID `SLINT-SHORTCUT-RECORDER-01`。

- Goal：在剪贴板和截图的快捷键输入框旁恢复按键录制。
- Requirements：录制实体键和修饰键，保留手动编辑，Esc 取消且保留原值。复用现有快捷键解析、冲突校验与保存规则。保存后提示重启应用快捷键。
- Acceptance Criteria：用隔离 X11 覆盖两个分页的录制、取消、重复键冲突和配置回读，使用解析测试验证常见修饰键和主键名称。
- Out of Scope：运行中重绑全局快捷键、系统绑定冲突探测与新增贴图热键。

#### 贴图工作区分组

需求 ID `SLINT-PIN-WORKSPACE-01`。

- Goal：恢复独立工作区的分组导航和贴图操作。
- Requirements：工作区使用共享标题栏和主题。分组栏显示全部、未分组及各命名组的数量，切换时保留有效选中项。支持创建分组、移动、打开和删除贴图。工作区列表继续读取轻量摘要，打开某张贴图时才加载内容。
- Acceptance Criteria：隔离 X11 检查窗口大小、创建和选择分组、更新归属、关闭重开和恢复贴图。已有修订及保存像素用例继续通过。
- Out of Scope：缩略图网格、桌面贴图坐标协议、旧版格式及数据迁移。

#### 编辑与采集辅助窗口

需求 ID `SLINT-AUX-WINDOWS-01`。

- Goal：让图片编辑、识字、滚动截图和录屏库使用同一套桌面窗体与主题，录屏控制反映会话实际状态。
- Requirements：各窗口沿用共享标题栏和六套主题。滚动截图允许 Esc 取消。录屏库按已确认状态显示暂停或继续，停止后显示封尾进度，避免重复提交。
- Acceptance Criteria：隔离 X11 在深色主题下打开编辑与采集窗口，验证窗口关闭、截图像素、滚动拼接、录屏暂停和停止，以及库导出字节一致。
- Out of Scope：浮窗屏蔽与摆位协议、音频采集、编码、播放器实现和发布包。

隔离 X11 的浅色和深色均验证标题栏像素和相关操作。Openbox 单独验证设置、启动器、贴图工作区和录屏库在拖动后仍能关闭。

#### 截图选区工具栏

需求 ID `SLINT-CAPTURE-TOOLBAR-01`。

- Goal：在框选后就近提供常用截图操作和真实像素尺寸。
- Requirements：鼠标松开后在选区边缘放置复制、保存、编辑、滚动截图和录屏操作。靠近屏幕底部时改放上方。支持 Enter 复制和 Esc 取消。像素尺寸与共用裁剪逻辑一致。
- Acceptance Criteria：隔离 X11 分别验证普通选区和底部选区的工具栏位置、复制/保存像素、原有编辑与采集入口。
- Out of Scope：覆盖层内联标注、系统截图协议、原始画面与落盘编码变更。

#### 文本工具双栏

需求 ID `SLINT-TEXT-PANES-01`。

- Goal：在同一个窗口同时读写原文、阅读译文和查看文本处理结果。
- Requirements：原文可粘贴与修改，结果可选择和复制。沿用原有翻译提交与历史，点击历史记录恢复两栏，不重新发送网络请求。工具支持切换与复制，沿用主题与标题栏。
- Acceptance Criteria：隔离 X11 从历史选中 Unicode 原文进入工具，粘贴一段文本并运行本地转换，读取右栏结果。用隔离历史记录验证原文、译文与复制；验证深浅主题和关闭。
- Out of Scope：增加服务商、更改翻译协议、多服务并发和 OCR 排版。

#### 贴图浮窗工具栏

需求 ID `SLINT-PIN-TOOLBAR-01`。

- Goal：让贴图窗口用侧边工具栏保留更多图片空间，并可拖动内容区移动窗口。
- Requirements：工具栏提供缩放比例、改变透明度、锁定尺寸、置顶、编辑、复制、保存和隐藏。透明度控件按需展开。沿用贴图原图、修订与呈现设置。
- Acceptance Criteria：隔离 X11 验证透明度和锁定写入数据库、锁定后缩放保持原值、复制的图像像素保持一致、工作区恢复贴图。用 Openbox 验证内容区拖动。
- Out of Scope：更改持久化协议、悬浮控件独立系统层级与 Wayland 的绝对摆位。

#### 剪贴板列表缩略图

需求 ID `SLINT-HISTORY-THUMBNAILS-01`。

- Goal：在历史列表中直接区分相邻的图片。
- Requirements：按当前列表视口读取图片，用有界解码器在后台缩到 64px，最多一张缩略图同时解码。关闭面板或滚出视口后释放缩略图，敏感项只显示保护状态，读取失败时保留原有行摘要。
- Acceptance Criteria：隔离 X11 验证透明 PNG 在列表中的像素、搜索换列表后不串图、预览侧栏和复制入口仍可用。完整门禁验证有界图片解码和已有截图与粘贴流程。
- Out of Scope：工作区修订缩略图、额外持久化文件和新的图像编码协议。

列表与侧栏复用有界 PNG 解码。每次最多保留 32 行的缩略图，滚动、切换搜索或关闭窗口后更新缓存。解码不占用 SQLite 线程。

#### 截图选区快速标注

需求 ID `SLINT-CAPTURE-MARKUP-01`。

- Goal：框选后在原地画矩形，能直接复制或保存带标注的截图。
- Requirements：选区内绘制、撤销或重新框选，固定红色和线宽。复制、保存与独立编辑器共用项目渲染，长截图与录屏沿用未标注区域。普通截图路径不启用标注渲染。
- Acceptance Criteria：隔离 X11 比较框选后的可见矩形和复制的像素，撤销后保持原始裁剪，打开独立编辑器后仍可撤销。快照、保存、长截图和录屏继续通过已有用例。
- Out of Scope：内联任意画笔、涂抹、文字、模糊和系统截图协议。

选区标注交给共享 Rust 渲染器，直接导出保留合成像素，进入编辑器保留可撤销的项目。提交裁剪时移交冻结帧缓冲区，避免复制整屏。

### M6：功能迁移顺序

1. 托盘、单实例、全局快捷键、主面板；
2. 剪贴板列表、搜索、详情、复制/粘贴/删除/收藏；
3. 设置、主题、自启动、快捷键编辑；
4. 截图覆盖层、区域选择、编辑器和导出；
5. 贴图、贴图工作区和历史；
6. OCR、二维码/条码、翻译和启动器；
7. 录屏控制、录屏库和回放；
8. 富文本/Markdown/代码/媒体预览。

每一项必须更新功能矩阵，写明自动测试、宿主机手测和未验证平台。图片效果沿用现有 Rust
`render_v2` 语义。Slint 路径的像素和按需 PNG 输出见[新版实现计划](2026-11-24-slint-native-implementation-plan.md)。

- [x] 托盘、单实例、快捷键与剪贴板基础闭环已通过 Linux 隔离桌面测试，包含搜索、复制和目标窗口粘贴。
- [x] 快速截图基础闭环已验证 X11 框选、复制和保存像素，并测到 GNOME Wayland 双屏原始帧。
- [ ] 保留原计划的编辑器、主题、详情和平台验收。GNOME 覆盖层与菜单、Portal 粘贴授权、连续五轮真实空闲 PSS 仍待验证。

### M7：两个阻塞原型

#### 富文本

建立 `PreviewDocument` 类型模型，把已清洗的 HTML/Markdown/代码转换成受限 block/span/image/link；最终
Slint UI 不执行 HTML、JavaScript 或远程资源。过渡期可保留 legacy 预览，但正式 Slint 产物不得按需拉起
WebView。

#### 录屏回放

比较 GStreamer、Media Foundation、AVFoundation 或统一解码器到纹理/音频队列的路径。验收至少覆盖
WebM VP9/Opus、暂停/拖动/结束、窗口关闭后的线程和句柄回收。未完成前不能删除 legacy 播放实现。

退出条件：两项都有三平台可交付路径和资源回收测试，不再依赖 DOM video。

### M8：打包、更新桥和切换

- [ ] Slint 使用 `cargo build --release -p clippy-slint` 和 cargo-packager 生成 deb/AppImage、NSIS/MSI、
  app/DMG；保留现有平台签名验证。
- [ ] 生成 release notes、`BUILD-INFO.json` 与 `QA-BUILD.txt`，明确框架版本和未包含框架。
- [ ] 在隔离 VM/真机验证最新 v0.1.x → v0.2.0：安装、签名、数据库备份、升级、启动、回滚。
- [ ] 如当前 Tauri updater 不能直接接收 Slint 安装包，先发布最后一个 v0.1.x bridge release；不得假设
  `latest.json` 和签名格式天然兼容。
- [ ] v0.2.0 发布后，v0.1.x 维护 release 使用非 latest/独立维护元数据，不改变稳定 updater 指向。

退出条件：v0.2.0 三平台安装与升级通过，正式产物不包含 WebView。

### M9：性能验收与清理

- [ ] 固定机器运行 5 次：启动、静置 60 秒、进程树 PSS、首次开窗、截图、录屏、OCR。
- [ ] 空闲总 PSS 每次都低于 50 MiB；峰值和任务结束后的回落分别记录。
- [ ] 删除 Slint 正式路径中已无消费者的 Tauri IPC、HTML/React、Node/Vite/Vitest、WebKit 依赖和
  tauri-action；legacy 目录继续保留维护所需依赖。
- [ ] 观察一个稳定发布周期后再提议是否删除 legacy 源码；本计划不自动删除。

退出条件：性能、功能、更新、三平台原生 CI 和人工 QA 都有同一发布 SHA 的证据。

## 当前未决但无需人工拍板的技术门槛

| 门槛 | 判定方式 | 失败时动作 |
|---|---|---|
| FemtoVG / Skia | 同场景三平台 PSS、启动、中文、透明窗口与多屏显示 | 选数据更优且三平台无阻塞的一方 |
| 原生回放 | VP9/Opus 功能、分发依赖、资源回收 | 保留 legacy，v0.2.0 不宣称功能完整 |
| updater 兼容 | 从真实 v0.1 安装包升级 Slint 包 | 增加最后一个 bridge release |
| 50 MiB | 固定机 5 次进程树 PSS | 用模块/分配剖析优化；不得改成单进程 RSS 或放宽口径 |

## 首轮实施切片

本轮只完成以下可独立审查的内容：

1. 规格和执行计划；
2. tag/backend 解析器与测试；
3. 当前未提交基线的门禁与资源记录；
4. 在基线通过后，再进入纯目录搬迁。

目录搬迁前若基线已有失败，先记录并修复真实失败；禁止把迁移建立在不可复现的红色基线上。

## 当前证据（2026-11-23）

- 迁移前当前工作树完整 Linux 本地门禁：27 通过 / 0 失败 / 2 跳过；跳过项为非宿主交叉 lint 与
  AppImage 可视 smoke。
- 已安装 legacy 静置样本：主进程 PSS 99,432 KiB、network 进程 2,557 KiB、WebProcess
  52,499 KiB，进程树总 PSS 154,488 KiB（约 150.9 MiB）、总 RSS 约 228.5 MiB；动态链接包含
  WebKit/JSC/GTK3/PipeWire。该样本不是 Slint 性能结果，也不替代 5 次固定流程验收。
- 发布路由测试、Slint `cargo check` / `clippy -D warnings` / `test` 与两项 vendored 供应链校验已
  独立通过。目录搬迁和门禁分段后的完整 Linux 本地门禁为 35 通过 / 0 失败 / 2 跳过；legacy
  Tauri 从新目录完成 `--no-bundle --no-sign` release 构建，产物为根
  `target/release/clippy-app`。三平台原生 CI、实际安装包和人工 QA 仍待执行。
- Slint release 壳层为 12,789,816 字节，动态依赖不含 Tauri、WebKit、JSC 或 GTK。隔离 Xvfb
  的单次 3 秒样本为 PSS 127,429 KiB、RSS 163,728 KiB；该环境使用 Mesa llvmpipe 软件 OpenGL，
  不是 GPU 桌面空闲结果，也不计入 50 MiB 验收。它说明 renderer 测试必须同时记录实际 GPU 与
  软件回退，不能用无头 runner 的 FemtoVG PSS 代替固定真机数据。
- Slint QA 已用 cargo-packager 0.11.8 在 Linux 真实生成
  `clippy-slint-qa_0.2.0_amd64.deb`：包、二进制、桌面入口和图标均使用 QA 名称，control 含
  `libc6`、`libfontconfig1`、`libgcc-s1` 依赖，包内含 MIT 与 Slint 许可。该结果只验证 Linux
  本地打包配置；Windows/macOS 原生包、CI artifact 和平台服务的运行期命名空间仍未验证。
- 共享存储接入后的 v0.2 Linux 默认门禁为 9 通过 / 0 失败 / 0 跳过，未编译 legacy；
  cargo-audit 0.22.2 对 998 个依赖扫描为 0 vulnerability、11 个允许的上游维护/健全性 warning。
  release QA 二进制为 14,061,296 字节且动态依赖不含 Tauri/WebKit/JSC/GTK。Xvfb + llvmpipe 的
  单次 12 秒开窗样本 PSS 为 95,338 KiB：应用堆 41,296 KiB、LLVM 26,116 KiB、Gallium 6,029 KiB。
  该样本不是实际 GPU、托盘无窗口或 5 次 60 秒验收，不计入 50 MiB AC。
