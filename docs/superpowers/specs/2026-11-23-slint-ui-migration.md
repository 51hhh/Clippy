# Slint UI 大版本迁移

Requirement ID: `SLINT-UI-MIGRATION-01`

## Goal

在同一仓库、同一开发分支内让旧 Tauri UI 与新 Slint UI 分目录共存，逐步复用现有 Rust
能力，并从 v0.2.0 起交付不常驻 WebView、空闲总 PSS 低于 50 MiB 的 Slint 桌面应用。

## Requirements

- R1：旧版 Tauri 与新版 Slint 必须位于独立目录，禁止通过长期平行 Git 分支维护两套产品。
  v0.2 日常开发、本地默认门禁和普通三平台 CI 只验证共享 Rust 层与 Slint；legacy 仅在显式维护门禁、
  legacy QA 或低于 v0.2.0 的发布 tag 中构建，避免旧 WebView 工程持续占用构建时间和磁盘。
- R2：旧版保持 v0.1.x，可继续接受必要的兼容、安全和严重缺陷修复，但不再承接大功能或大规模
  UI 改造；新版从 v0.2.0 开始。
- R3：发布后端只由语义化 tag 决定：低于 v0.2.0 的 tag 打包 Tauri，v0.2.0 的
  alpha/beta/rc 和所有更高版本打包 Slint。版本比较必须由单一、带自动化测试的脚本完成，禁止在
  workflow 中使用字符串比较。
- R4：正式发布一次只能包含一种 UI 后端。v0.2.0 沿用正式版应用标识、配置目录、数据库目录和
  更新入口；迁移期 QA 包使用独立标识和独立数据目录，且不得与正式版同时运行。
- R5：正式 v0.2.0 不得链接或启动 WebKit/WebView。富文本、图片、截图编辑、设置、贴图和录屏
  库必须由 Slint 或原生媒体组件呈现；不得把 Tauri 嵌入 Slint 作为最终兼容层。
- R6：业务、存储、截图、录屏、OCR、翻译和平台能力逐步下沉到 UI 无关的 Rust crate；UI 只能
  通过类型化模型和明确的异步边界调用这些能力。
- R7：Slint 依赖固定为 1.19.x 迁移基线，首个正式版本固定为 1.19.0；旧版 Tauri 基线记录为
  2.12.4。发布说明、安装包内构建信息和 QA 元数据必须明确写出实际 UI 后端及框架版本，未包含的
  框架写明 `not included`。
- R8：Slint 使用 royalty-free desktop license，并在 About、第三方许可清单和安装包材料中保留
  要求的归属说明。
- R9：v0.2.0 正式版必须支持从最新 v0.1.x 原地升级。升级前备份数据库；本轮不主动改变数据库
  schema。旧版维护 tag 仍可构建和发布，但 v0.2.0 上线后不得覆盖稳定版更新入口或把用户降级到
  v0.1.x。
- R10：空闲内存以进程树 PSS 总和判定：启动并静置 60 秒、无截图/录屏/OCR/预览任务时低于
  50 MiB。截图、录屏和 OCR 峰值单独记录，不得混入空闲指标，也不得用单进程 RSS 代替总 PSS。
- R11：开发过程按当前分支串行推进；每个阶段仍需保持逻辑独立、单独验证和可回退，不把目录搬迁、
  功能修复、CI 路由与发布切换混为一次不可审查的改动。

## Acceptance Criteria

- [ ] 仓库存在独立的 legacy Tauri 与 Slint 应用目录，根 Cargo workspace 能检查共享 crate 和两套
  Rust 入口；旧版前端测试仍可独立执行。
- [ ] tag 路由自动化测试至少覆盖 `v0.1.21`、`v0.1.99`、`v0.2.0-alpha.1`、
  `v0.2.0-rc.1`、`v0.2.0`、`v0.10.0`、`v1.0.0` 和非法 tag。
- [ ] 普通 Ubuntu、Windows、macOS 门禁在同一 SHA 上检查共享 core、平台层和 Slint；现有
  required-check 名称保持稳定。legacy 维护改动必须额外执行显式 legacy 门禁，但不进入 v0.2 默认路径。
- [ ] 发布 workflow 对 v0.1.x 只构建 legacy，对 v0.2.0 预发布及以上只构建 Slint；两条路径均验证
  tag、manifest、CHANGELOG、签名和产物来源。
- [ ] Linux、Windows、macOS QA 包能从同一 SHA 产出，并通过元数据区分 `legacy-tauri` 与
  `slint`；QA 包使用独立应用标识和数据目录。
- [ ] v0.2.0 功能矩阵覆盖剪贴板监听/搜索/详情/操作、托盘、快捷键、设置、截图与编辑、贴图、OCR、
  翻译、录屏与回放；每项均有自动测试或明确的三平台人工步骤。
- [ ] v0.2.0 产物依赖检查证明未链接 WebKit/WebView，运行时进程树没有 WebKit WebProcess 或
  Tauri 前端进程。
- [ ] 固定测试机上连续 5 次空闲 PSS 测量均低于 50 MiB，并单独记录启动时间、截图峰值、录屏峰值
  和 OCR 峰值；托管 CI 不作为严格内存门槛。
- [ ] 从最新 v0.1.x 安装包升级到 v0.2.0 后，数据库、设置和剪贴板历史可用；失败路径能从升级前
  备份恢复。
- [ ] `./scripts/ci-local.sh` 完整通过；同一 SHA 的三平台原生 CI 成功；真实安装、系统权限、托盘、
  多显示器与更新流程的人工结果单独记录，未执行项不得勾选。

## Out of Scope

- 不在迁移期重写稳定的截图、录屏、OCR、翻译、存储算法；只抽离 UI 耦合并修复迁移真实触发的
  问题。
- 不为 v0.1.x 增加新的大功能、主题系统或第二套前端架构。
- 不承诺旧 Tauri 与新 Slint 可同时连接同一个正式数据库、注册同一组全局快捷键或同时运行。
- 不把 Slint software renderer 作为桌面中文正式渲染器；最终 renderer 由 FemtoVG 与 Skia 原型
  的三平台显示、内存和启动数据决定。
- 不用 hosted runner 的瞬时 RSS 作为 50 MiB 验收证据。
- 不在 v0.2.0 同时更换数据库 schema、同步协议或更新服务。

## Decision

**Context**：现有 Tauri/WebView 实现功能完整，但常驻 WebView 使单应用进程树长期处于数百 MiB；
直接在原目录替换 UI 会让旧版维护、升级验证和跨平台回退失去稳定参照。

**Decision**：同仓库、同分支、分目录保留两套应用入口，把 UI 无关能力逐步抽到共享 Rust crate；
v0.2 默认门禁只构建共享层与 Slint，legacy 由显式维护入口和 v0.1 tag 守护。正式发布根据 semver tag
只选择其中一套。v0.2.0 复用正式身份和数据，QA 构建使用隔离身份。Slint 采用 1.19.0 与
royalty-free desktop license，最终产物不包含 Tauri 或 WebView。

**Consequences**：仓库仍保留两套源码和旧 tag 的可重建性，但日常构建不会下载 Node/WebKit 或生成
legacy 产物。共享层变更可能影响 v0.1.x 时，必须在准备 legacy 维护发布前显式补跑其三平台门禁；
普通 v0.2 CI 不再提供这项兼容证据。富文本和录屏回放不能依赖 DOM，必须分别建立类型化文档模型和
原生媒体路径。发布切换前必须完成更新桥和三平台真实 QA，不能只凭编译成功发布 v0.2.0。

## 2026-11-24 已批准调整

用户确认后续开发以新版为唯一默认构建目标，legacy 继续保留但不参与每次本地/普通 CI。该调整不改变
v0.1.x tag 的可发布性，也不删除 legacy 源码；它把兼容验证从“每次必跑”改为“维护或旧版发布时显式
执行”，以换取更低的构建空间、依赖下载和反馈时延。
