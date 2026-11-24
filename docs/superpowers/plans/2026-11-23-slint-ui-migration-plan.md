# Slint UI 迁移执行计划

Requirement ID: [`SLINT-UI-MIGRATION-01`](../specs/2026-11-23-slint-ui-migration.md)

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
  clippy-core/                # 存储、模型、用例，不依赖 UI host
  clippy-platform/            # 托盘、快捷键、窗口、系统集成
  clippy-ui-model/            # Slint/legacy 共用的展示模型与动作
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

### M3：双实现 CI 与 QA 身份隔离

- [x] Linux 与 Windows 本地门禁拆成 core、legacy、Slint 三段；默认 wrapper 全跑，任何只跑子集或
  quick 的选项都标记为部分证据。
- [x] 保留 `Check (ubuntu-22.04)`、`Native Check (windows-latest)`、
  `Native Check (macos-latest)` 名称，在其中加入 Slint 编译、clippy、test 和 `.slint` 编译；
  workflow 合同已实现，远程同 SHA 结果仍待验证。
- [x] Native QA workflow 并行定义 `qa-legacy-*` 和 `qa-slint-*`，单一生成器写入 backend、framework、
  renderer、SHA 与未包含框架；远程同 SHA 的八套 artifact 仍待实际运行验证。
- [x] Slint QA 建立独立 app/bundle ID、可执行文件和桌面入口；生产入口显式绑定原身份，不依赖
  feature 切换身份。
- [ ] 平台服务接入后实际消费 QA 的快捷键、单实例和数据命名空间，并验证与正式版互不污染。

退出条件：同一 SHA 三平台同时验证 legacy、core、Slint，QA 包不会污染正式数据。

### M4：共享核心抽离

- [x] 首批抽出 `ContentType`、`ClipItem`、`TranslationHistoryEntry`、`UrlMeta` 与产品标识；
  legacy 通过原模块重导出保持调用路径，Slint 已消费共享标识，共享 crate 不依赖 Tauri 或 Slint。
- [x] 抽离配置值对象与持久化边界：`clippy-core::config` 拥有 `AppConfig`、目标平台快捷键默认策略、
  v1→v2 迁移、JSON codec 与结构化提交/回滚协议；legacy 只保留私有文件权限和 Tauri 副作用适配。
  Slint 的实际数据目录装配随平台服务接入，不在本项伪造一套临时路径。

按“可独立测试、无窗口类型”的顺序迁移：

1. `models`、`config`、数据库与搜索；
2. 剪贴板监听、动作与敏感内容分类；
3. 截图/编辑 renderer、OCR、翻译；
4. 录屏 session、manifest、恢复和媒体目录；
5. 托盘、快捷键、单实例、窗口策略等平台适配。

约束：共享 crate 禁止依赖 `tauri::AppHandle`、`tauri::WebviewWindow` 或 Slint component；host 通过窄接口
注入窗口/对话框/通知能力。每抽一块，legacy 先改用共享 crate，再接 Slint，避免复制实现。

退出条件：Slint 所需业务能力不需要调用 Tauri IPC，legacy 回归测试仍通过。

### M5：Slint 设计系统与窗口骨架

- [ ] 建立颜色、排版、间距、圆角、阴影、动效和高对比度 token；六个现有主题映射为同一语义层。
- [ ] 实测 FemtoVG 与 Skia：Linux/Windows/macOS 中文、透明窗口、阴影、多屏缩放、启动时间、空闲 PSS。
- [ ] 仅根据数据选一个正式 renderer；software renderer 不进入桌面中文正式路径。
- [ ] 建立 main、settings、capture、pin、library 等窗口控制器，避免默认一次实例化所有 component。
- [ ] 托盘常驻但无可见窗口时不创建 WindowAdapter；首次打开再创建对应 component。

退出条件：空壳托盘态接近最终生命周期，窗口按需创建/销毁且无泄漏。

### M6：功能迁移顺序

1. 托盘、单实例、全局快捷键、主面板；
2. 剪贴板列表、搜索、详情、复制/粘贴/删除/收藏；
3. 设置、主题、自启动、快捷键编辑；
4. 截图覆盖层、区域选择、编辑器和导出；
5. 贴图、贴图工作区和历史；
6. OCR、二维码/条码、翻译和启动器；
7. 录屏控制、录屏库和回放；
8. 富文本/Markdown/代码/媒体预览。

每一项必须更新功能矩阵，写明自动测试、宿主机手测和未验证平台。图片效果继续使用现有 Rust
`render_v2` 输出 RGBA buffer，禁止在 `.slint` 中重写模糊、马赛克和标注算法。

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
