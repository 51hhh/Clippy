# WIN-NATIVE-01 — 最新分支 Windows 审查与整改

日期：2026-10-01；状态：in_progress。

## Goal

以最新功能分支为基线，在 Windows 复核现有核心功能及非默认录屏 QA 能力，修复可复现问题，
补齐本机验证入口，并建立可追踪的原生编译、安装包和桌面验收任务。

## 当前续审状态

- `WIN-WGC-BRIDGE-ROLLBACK-01` / W35：独立 `codex/windows-wgc-bridge-rollback`，基于 `f8409b7`。
  应用帧桥在 start 错误和正常销毁时取消并 join，接收循环不依赖全部 sender 被释放；成功转移、
  原错误、帧/时钟与暂停恢复语义保持。提取旧协议 MSVC 红基线 3 passed / 4 failed，同组七项
  真实线程合同全绿；干净 7922457 完整默认/QA 门禁确认原生子进程 exit 0，30 passed / 0 failed /
  1 Linux smoke skipped。默认 Rust 1137、QA Rust 1194（各 5 ignored，重叠不累加），新增七项
  每图在总数内；前端 77 文件 / 1307 passed，源码/日志哈希与干净检出已核对。真实 WGC/Close 时限、系统释放、
  设备/硬件矩阵、当前 SHA CI 与其它宿主未验；vendor、默认 feature 和桌面停止边界保持。

- `WIN-PIN-LIVE-DPI-01` / W34：独立 `codex/windows-pin-live-dpi`，基于 `c6075e2`。
  Windows 渲染由当前窗口原生 DPI 驱动，首读使用 caller-bound 只读业务命令，既有 core 权限
  与来源/展示元数据保持；先订阅事件，旧查询或 payload 不覆盖实时比例，未知回退 auto。
  原 App 十二项红基线 2 passed / 10 failed，同组全绿；三项 API 适配器、六项 MSVC 纯读取
  合同、既有 Pin/权限回归与 TS 检查通过。干净 e339042 完整默认/QA 门禁确认原生子进程 exit 0，
  30 passed / 0 failed / 1 Linux smoke skipped；默认 Rust 1130、QA Rust 1187（各 5 ignored，
  重叠不累加），新增六项每图在总数内，前端 77 文件 / 1307 passed。真实 DPI 与 WebView2 像素、
  Windows 10/多屏、当前 SHA CI 和其它宿主未验；源码/日志哈希与干净检出已核对。
  WGC 续审确认现有 monitor-local crop 合同和 prepare/connect 的原生 descriptor 复核；
  未把冻结到 prepare 的原点变化定为缺陷或修改它。运行中热插拔/身份重用仍未验。

- `WIN-PIN-WORKAREA-01` / W33：独立 `codex/windows-pin-workarea`，基于 `7017562`。
  原生 owner/物理交集驱动保存、保留原生目标的恢复和客户区工具条边界；旧存储格式与正常旧记录
  相对位置/展示参数保留。提取旧生产协议红基线 1 passed / 15 failed，同组十六项 MSVC 回归通过；
  干净源码 4a58101 完整默认/QA 门禁确认原生子进程 exit 0，30 passed / 0 failed / 1 Linux smoke skipped；
  默认 Rust 1124、QA Rust 1181（各 5 ignored，重叠不累加），前端 1292 passed；十六项包含在各图。
  旧错误来源不能反推；真实 API/窗口/DPI/热插拔、WGC 与新 SHA CI 未验。

- `WIN-PIN-ORIGIN-01` / W32：独立 `codex/windows-pin-physical-origin`，基于 `e1c7191`。
  冻结物理来源贯穿普通/长截图、输出重试和像素登记；新建图片 Pin 按一次原生快照和 PNG 像素
  规划，创建/reveal 保留物理请求，Windows resize 不重复逻辑定位，来源失效按光标/主屏回退。
  旧生产输出及提取的旧布局/请求协议红基线 3 passed / 15 failed，同组十八项 MSVC 离线回归通过；
  干净源码 888127a 完整默认/QA 门禁确认原生子进程 exit 0，30 passed / 0 failed / 1 Linux smoke skipped；
  默认 Rust 1108、QA Rust 1165（各 5 ignored，重叠不累加），前端 1292 passed；十八项包含在各图。
  工作区迁移、工具条交集、WGC、真实多屏/DPI、新 SHA CI 与其它宿主未验。

用户最新要求停止桌面操控，先进行代码 review 与修复。桌面操作保持停止；本机仅执行原生
编译、自动合同和文件证据核对，Linux/WSL 未启动。

- `WIN-PIN-TOOLBAR-01`：独立 `codex/windows-pin-toolbar-height`，修复源码 `7aa6cf6` 的完整
  Windows 默认/录屏 QA 门禁 exit 0，23 passed / 0 failed / 1 skipped；修复后桌面复测未运行。
- `WIN-PRIVATE-WRITE-01`：独立 `codex/windows-private-write-order`，基于 Pin 修复后的 `aefc413`。
  修复源码 `f788b1f57d852b7df87e334b579c3224c7fd2543` 的完整 Windows 默认/录屏 QA 门禁 exit 0，
  23 passed / 0 failed / 1 skipped（Linux smoke）；默认 Rust 1042、QA Rust 1095、前端 1292 passed。
  两组 Rust 重叠且各有 5 ignored，不累加；新 SHA 原生 CI 尚未执行。
- `WIN-LONGSHOT-CURSOR-01`：独立 `codex/windows-longshot-cursor-restore`，基于 `1cd09c8`。
  提前失败的生产恢复 guard 红基线 1 passed / 3 failed，修复后八项定向合同通过；源码
  `d8dff808e320fd840376e2acec396887e6bbc3ce` 的完整 Windows 默认/录屏 QA 门禁 exit 0，
  23 passed / 0 failed / 1 skipped；默认 Rust 1046、QA Rust 1099、前端 1292 passed。
  两组 Rust 重叠且各有 5 ignored，不累加。新增回归使用注入指针接口；真实桌面接管及新 SHA CI 未验。
- `WIN-CF-HTML-01`：独立 `codex/windows-cf-html-bounds`，基于 `46e3fdc`。Windows HTML 读取关闭
  clipboard-win 5.4.1 未受缓冲区边界约束的复制调用，改为实际字节和安全片段解析；离线红基线
  2 passed / 7 failed，修复后九项通过。源码 `50b7778ec9e4bd52fa31aa657be607877c4990ef` 完整
  Windows 默认/录屏 QA 门禁 exit 0，24 passed / 0 failed / 1 skipped，包含新增依赖库定向组。
  默认 Rust 1046、QA Rust 1099、前端 1292 passed；Rust 重叠且各有 5 ignored，不累加。
  Windows Native CI 已添加入口，但远程新 SHA CI、真实富文本互操作未验，未执行原生畸形复制。
- `WIN-CLIP-IMAGE-BUDGET-01`：独立 `codex/windows-image-decode-budget`，基于 `cf59157`。
  将 watcher 的尺寸预算前移到 Windows PNG / DIB 像素解码前；四字节故障注入红基线
  3 passed / 2 failed，修复后七项预算合同通过。531d791 本机完整默认/QA 门禁 exit 0，
  25 passed / 0 failed / 1 skipped；CI 定向入口已接线但远程未运行。
  当时扩展图片组 9 passed / 1 failed，Chrome DIB 在原 cf59157 源码也失败；后来由下项 W22 独立修复。
- `WIN-DIBV5-PIXEL-01`：独立 `codex/windows-dibv5-pixel-offset`，基于 `6069cce`。
  W22 红基线 2 passed / 2 failed：原 Chrome 读取失败及带尾部数据的成功错图；改用借用原数据的
  BMP 文件视图提供显式像素偏移。原 Chrome/Firefox 逐像素断言保留，五项 DIB 和三项文件视图
  合同通过；连同预算和富文本 24 项 Windows 离线合同通过。源码 25fb5d7159af66a88d829eda199efa649698633d
  完整本机默认/QA 门禁 exit 0，27 passed / 0 failed / 1 skipped；默认 Rust 1046、QA Rust 1099，
  两图重叠且各有 5 ignored，不累加；前端 75 文件 / 1292 passed，CI 已接线未远程运行，桌面未验。
- `WIN-PASTE-RECHECK-01`：独立 `codex/windows-paste-input-recheck`，基于 `949a2d9`。
  Windows 输入后端初始化后、首次按键前再查窗口/PID/前台；初始化期间目标变化的离线红基线
  2 passed / 4 failed，修复后六项通过，未调用 Enigo 或窗口/输入 API。干净源码
  `14bf61630efab7b62905bbc5b976fbed8e62166c` 完整 Windows 默认/QA 门禁 exit 0，
  27 passed / 0 failed / 1 skipped；默认 Rust 1052、QA Rust 1105（各 5 ignored，重叠不累加），
  前端 75 文件 / 1292 passed。六项粘贴合同已包含在两个 Rust 图，日志哈希已核对；
  最后复核后的系统竞争、真实用户接管、新 SHA CI 与 macOS 原生图仍未验。
- `WIN-WASAPI-STOP-TAIL-01`：独立 `codex/windows-wasapi-stop-tail`，基于 `da18681`。
  正常 Stop 与 Pause 的清空策略分开；先停止、按实际 endpoint 容量排空，再 Reset 并保留 PCM。
  六项新增停止合同与七项既有音频合同：旧控制协议红基线 9 passed / 4 failed，修复后 13 passed。
  干净源码 `a463c3ba9be876dbfe1a45893dcca28e013cadb6` 完整 Windows 默认/QA 门禁 exit 0，
  27 passed / 0 failed / 1 skipped；默认 Rust 1058、QA Rust 1111（各 5 ignored，重叠不累加），
  13 项音频合同在两个真实 Cargo 图通过，已包含在 Rust 总数；前端 75 文件 / 1292 passed。
  WASAPI API 图编译/lint、检出干净与日志哈希已核对；录屏仍非默认，真实设备/混音和新 SHA CI 未验。
- `WIN-WGC-CLOSE-01`：独立 `codex/windows-wgc-close-retry`，基于 `3308677`。
  原全局 closed 和 session 错误短路的离线红基线 1 passed / 5 failed，修复后六项通过；
  vendor 原始字节正校验和 runtime/mod/recorder 三个篡改负例通过。关闭状态按资源记成功，
  两者均尝试，失败可重试。产品修复 869a13f、独立 CI 接线及被测 SHA 03b4cb8；完整
  Windows 默认/QA 门禁 exit 0，28 passed / 0 failed / 1 skipped（Linux smoke）。
  默认 Rust 1058、QA Rust 1111（各 5 ignored，重叠不累加），前端 75 文件 / 1292 passed。
  六项真实 vendor Cargo 合同通过，不包含在应用 Rust 总数；含测试的 vendor 严格 clippy、
  干净检出与日志哈希已核对。系统 Close 最终释放、真实 WGC 与新 SHA CI 未验。
- `REC-AV-BRIDGE-JOIN-01`：独立 `codex/windows-av-bridge-join`，基于 `99f83f8`。
  视频 join 出错后布尔短路遗漏音频 join；两条线程均 join 后再报告既有 panic 错误。
  完整纯模块和真实受控线程红基线 2 passed / 2 failed，修复后四项通过。干净源码
  091b5cb7663055a3b1a44e2958255bf3717bef79 完整 Windows 默认/QA 门禁 exit 0，28 passed /
  0 failed / 1 skipped；默认 Rust 1058、QA Rust 1115（各 5 ignored，重叠不累加），前端 1292 passed。
  四项回归仅在 QA 图、已计入总数；既有 worker 合并/失败中止测试通过，检出和日志哈希已核对。
  共享 Linux/macOS 图、新 SHA CI 和真实 panic/桌面仍未验，既有原型 CI 前缀覆盖新模块。
- `WIN-WGC-INIT-ROLLBACK-01`：独立 `codex/windows-wgc-init-rollback`，基于 `3c19883`。
  pool 创建后、完整 runtime 建立前的注册/session 错误须尝试 Close，再返回原错误。
  原控制协议红基线 1 passed / 3 failed，真实 scopeguard 的四项绿合同通过；vendor 原始字节
  正例/三个篡改负例通过。产品修复 db05650、独立 CI 接线及被测 SHA 61d6823，完整 Windows
  默认/QA 门禁 exit 0：29 passed / 0 failed / 1 skipped（Linux smoke）；默认 Rust 1058、QA Rust
  1115（各 5 ignored，重叠不累加），前端 75 文件 / 1292 passed。独立 vendor 四项初始化及六项
  关闭合同通过，不计入应用 Rust 总数；严格 clippy、检出与日志哈希已核对。真实最终释放、桌面与新 SHA CI 未验。
- `WIN-REGISTRY-BUFFER-01`：独立 `codex/windows-registry-buffer`，基于 `f84e6be`。
  RegGetValueW 字节数误作 u16 长度的初始化合同缺陷已修复；八项生产入口离线回归通过，
  安全旧单位协议辅助模型 4 passed / 4 failed，原未定义行为不执行。8c6fbfe 首次门禁因遗留
  导入的严格 lint 失败（29 passed / 1 failed）；修正后干净源码 bb38cc6 完整 Windows 默认/QA
  门禁 exit 0，30 passed / 0 failed / 1 skipped（Linux smoke）；默认 Rust 1058、QA Rust 1115，
  各 5 ignored、重叠不累加，前端 1292 passed。八项真实 vendor Cargo 回归、严格 clippy、
  三个字节篡改负例、检出干净和日志哈希已核对；新 SHA CI、实际注册表与桌面未验。
  其它 Windows 持久化/自启动路径未确认新增缺陷；混合 DPI 的实际硬件验收保留 W04。
- `WIN-WINDOW-SCALE-01`：独立 `codex/windows-window-candidate-scaling`，基于 `2bb755a`。
  单一窗口比例跨屏求交的实际旧函数抽取协议红基线 2 passed / 6 failed；Windows 物理矩形
  逐帧转换、裁剪并保留分数坐标，八项 MSVC 离线回归通过，含空像素帧边界。干净源码
  78bd83fc3547459c523a150faa8a999248c16267 完整默认/QA 门禁 exit 0，30 passed / 0 failed /
  1 skipped（Linux smoke）；默认 Rust 1066、QA Rust 1123（各 5 ignored，重叠不累加），前端
  1292 passed，八项回归含在两个 Rust 图。首次包装器退出码缺失不计通过，记录保留；
  修正捕获并验证退出 0/17 后，同 SHA 完整重跑通过。W04 真机、新 SHA CI 与其它宿主仍未验。
- `WIN-OVERLAY-FOCUS-01`：独立 `codex/windows-overlay-focus`，基于 `6a9a84e`。原 reveal 把
  物理光标与逻辑矩形比较，实际生产入口红基线 2 passed / 6 failed；按各冻结帧比例判定，
  保留会话/焦点兜底，既有双屏夹具补齐一致两帧并保留原断言。八项绿回归及干净源码
  f5779966f2adaca65d75dd0a0f6affece6e4fc6c 完整默认/QA 门禁确认子进程 exit 0，30 passed /
  0 failed / 1 skipped（Linux smoke）；默认 Rust 1074、QA Rust 1131（各 5 ignored，重叠不累加），
  前端 1292 passed；八项新回归在两个 Rust 图执行，日志哈希、检出干净和原断言审计通过。
  原生覆盖层/guide 建窗的逻辑位置歧义及原始物理原点舍入仍单列 W04，未用本修复关闭。
- `WIN-NATIVE-MONITOR-01`：独立 `codex/windows-physical-monitor-bounds`，基于 `4630b38`。
  原始物理边界在逻辑归一化前保留，覆盖层/guide 请求为物理类型；光标、窗口候选、长截图
  滚动点和重捕获身份直接使用原始边界，拒绝缺失/不匹配/空/溢出元数据。实际旧算法红基线
  1 passed / 15 failed，五份回归原始字节不变，十六项 MSVC 离线回归通过。首次 b3e8f7a 门禁
  28 passed / 2 failed / 1 skipped，夹具严格 lint 失败；原日志保留，修正后的干净源码
  2f0e225494dcf850bacac21c59566b781a3b6767 完整默认/QA 门禁确认子进程 exit 0，30 passed /
  0 failed / 1 skipped（Linux smoke）；默认 Rust 1090、QA Rust 1147（各 5 ignored，重叠不累加），
  前端 75 文件 / 1292 passed、Python 33 + 3。每图新十六项及既有焦点/候选各八项包含在总数；
  原始日志哈希、干净检出、严格 lint、供应链和构建通过，后继仅四份 Markdown。
  当前 SHA CI、实际窗口/DPI 事件、Pin/WGC 原点身份、Windows 10/多屏仍未验，不关闭 W04。
- 已安装包仍为旧源码 `45769c9`。实际 Windows 11 桌面记录为 2 pass / 1 fail（旧 Pin 工具栏裁切）/
  36 not_run；原始 39 项 not_run 模板保持原字节，模板不能替代实际记录。
- NSIS 落盘及启动已有子步骤证据；完整安装升级、MSI、卸载、录屏/音频、管理员目标、
  Windows 10、双屏/混合 DPI/负坐标仍未验收。暂停中的桌面项不能用代码合同或旧 CI 勾选。

对应修复规格见 `2026-10-01-windows-pin-toolbar-height.md`、`2026-10-01-windows-private-write-order.md`、
`2026-10-01-windows-longshot-cursor-restore.md`、`2026-10-01-windows-cf-html-bounds.md`、
`2026-10-01-windows-image-decode-budget.md`、`2026-10-01-windows-dibv5-pixel-offset.md`、
`2026-10-01-windows-paste-input-recheck.md`、`2026-10-01-windows-wasapi-stop-tail.md`、
`2026-10-01-windows-wgc-close-retry.md`、`2026-10-01-windows-av-bridge-join.md`、
`2026-10-01-windows-wgc-init-rollback.md`、`2026-10-01-windows-registry-buffer.md`、
`2026-10-01-windows-window-candidate-scaling.md`、`2026-10-01-windows-overlay-focus.md`。
以下基线与早期门禁记录保留各自来源 SHA；本状态更新为后继文档，不冒称文档 SHA 已执行门禁。

## Baseline

- 已刷新 origin；最新分支为 `origin/codex/recording-audio-device-selection`。
- 基线 SHA：`8b99b884f660f37c9d81ba0dc8947d13c3d3a08a`，应用版本 `0.1.20`。
- 最初工具修复分支：`codex/windows-native-review`；只承载 Windows 验证入口和合同测试修复。
  后续产品问题使用上文列出的独立分支。
- 用户确认 Ubuntu Wayland 已调试正常；本轮未重跑该环境，不扩展为 Linux 全矩阵验收。
- 基线 GitHub CI：<https://github.com/51hhh/Clippy/actions/runs/35792281966>，
  三项默认原生检查及 Ubuntu、Windows、macOS ARM/Intel 四项录屏原型检查均为 success。
  这是基线证据，不能替代后续修改 SHA 的检查或 Windows 桌面 QA。

## Requirements

1. 区分 dev、发布 tag 和最新功能分支，记录关键提交带来的实际能力及 feature 门控。
2. 在 Windows 本机运行完整前端测试、类型检查、静态合同和生产构建；文件 URL 使用系统路径转换，
   源码合同兼容 LF/CRLF；负例修改必须实际生效。vendor 和 Cargo 锁文件按仓库规则保留 LF，
   原始 SHA-256 校验不得归一化输入。保留现有安全合同的全部负例，不能删除失败测试或弱化校验。
3. 提供原生 PowerShell 门禁，覆盖 Python 纯合同、默认 Rust、vendor WGC、前端和可选 Windows
   双轨 QA 检查。缺工具、外部命令非零、显式部分检查和跳过项不得被报告为完整通过。
4. Windows 原生 CI 增加前端检查，补上只有 Ubuntu 执行前端导致的宿主路径盲区。
5. 审查混合 DPI/负坐标、普通与管理员目标粘贴、DACL/原子配置、WGC/WASAPI、设备目录、
   录屏控制窗排除、暂停/恢复和崩溃分段恢复；每个结论区分复现缺陷、静态疑点和未执行验收。
6. 保持录屏 `recording-windows-av-qa` 非默认；正式 release 的能力不因 review 提前开放。
7. 产品行为修复使用独立分支、自己的回归测试与 CHANGELOG；本分支仅验证工具变更，无用户界面行为变更。
8. OCR 质量工具的诊断目录在 Windows 使用当前用户专用、禁止宽松继承的 DACL；创建失败不得
   落回普通目录。符号链接拒绝合同不要求普通 Windows 用户具备创建真实符号链接的权限。

## Acceptance Criteria

- [x] 最新分支、完整 SHA、关键节点和基线同 SHA CI 已核对。
- [x] Windows 前端红基线已取得：73 个文件，1265 项通过、9 项失败，失败均为两组合同测试路径错误。
- [x] 两组合同测试与完整前端测试在 Windows 通过，类型、JS lint、IPC/HTML、供应链和生产入口通过。
- [x] PowerShell 门禁的缺依赖、非零退出码和部分检查不能虚报完整成功；入口文档与脚本一致。
- [x] Windows OCR 诊断目录及新建子文件的 DACL 原生检查通过；质量测试不依赖 POSIX mode 或符号链接特权。
- [x] SHA `42e52c064aba36bb7e93a5e68a0cfb54f65c5b7b` 的三项原生与四项录屏原型 CI 均 completed/success；不替代后续独立 W10 修复的 CI。
- [x] 默认及 `recording-windows-av-qa` 在 Windows 本机完成 Rust check/clippy/test。
- [ ] Windows 10/11 混合 DPI、多屏、权限、安装更新及有声录屏桌面 QA 绑定相同包/SHA。
- [ ] 已确认的产品缺陷修复后复测；尚未执行、外部工具缺失和静态疑点保留未完成状态。

## Out of Scope

- 自动合入 dev、改写历史、发布版本或启用录屏默认 feature。
- 以 Ubuntu Wayland、GitHub runner 或合成源测试替代 Windows 桌面证据。
- 重做 UI，或在 Windows review 中扩展 OCR 权重交付、智能擦除、任意脚本等其它产品需求。
- 未经确认地把静态 DPI 疑点写成已复现缺陷或做跨域坐标重构。

## Tasks

| ID | 优先级 | 工作与验收 | 状态 |
|---|---|---|---|
| W01 | P1 | 修复文件 URL 路径；保留 9 项合同负例/正例并完成 Windows 前端门禁 | 本机已通过 |
| W02 | P1 | PowerShell 门禁与 Windows CI 前端检查；缺工具/失败/部分运行严格区分 | 本机入口与 42e52c0 Windows 前端/OCR/Rust CI 已通过 |
| W03 | P1 | Rust MSVC、C++ SDK、WebView2；录屏另需 MSYS2 make/diffutils/perl/nasm、MSBuild、CMake、LLVM tools/libclang；默认与 QA 图分别验证 | 工具已安装，默认与录屏 QA 本机门禁通过 |
| W04 | P1 | 100%/125%/150% 多屏与负坐标：冻结帧、跨屏窗口候选、覆盖层、Pin、guide、长截图自动滚动、WGC 选区 | 候选/焦点见 W29/W30，原始物理边界和建窗请求见 W31；图片 Pin 来源/请求见 W32；实际 DPI/热插拔、工作区保存/恢复与工具条合同见 W33；WGC 与真机矩阵仍未验，本机单屏。长截图失败清理指针合同见 W19，真实接管待验 |
| W05 | P1 | 同权限自动粘贴一次、高完整性目标 copy-only、目标销毁/复用、用户接管；DACL 与配置连续覆盖 | 45769c9 普通权限文本/图片完整用例实际通过；管理员、销毁复用与用户接管桌面待验证。私有文件准备失败时序见 W18，富文本片段边界见 W20，首次按键前目标复核见 W23 |
| W06 | P1 | QA 包设备默认/非默认/同名/拔出、双源混音、暂停恢复、控制窗排除、强杀恢复、30 分钟 A/V 漂移 | WASAPI 正常停止尾部见 W24，WGC 关闭/初始化清理见 W25/W27，双轨桥接线程回收见 W26，WGC 应用帧桥启动回滚见 W35；真实设备、混音及其余场景仍待真机验收 |
| W07 | P2 | NSIS/MSI 安装升级卸载、WebView2、自启动、托盘/快捷键、系统凭据与更新 | 官方 QA 包身份已核对，MSI 只读检查通过；NSIS 安装落盘/启动子步骤已核对，完整 MSI/升级/卸载/updater 未验收；本机自签名链不受信任，未更改信任 |
| W08 | P1 | 每个产品修复单独分支，更新对应需求/CHANGELOG；同 SHA 三平台 + 四原型 CI，回归 Ubuntu Wayland | 42e52c0、45769c9 与 WinPS 的 b2fd247 各自七项 CI 通过；后续十九项产品修复本机通过，新 SHA CI、Linux 本地完整门禁及 Wayland 回归保留未完成 |
| W09 | P1 | OCR 质量工具 Windows 私有诊断目录与符号链接拒绝合同；失败关闭，检查子文件继承 | 实际 DACL/等价 SDDL 及 10 类失败关闭负例通过；本机 33 项质量合同与 42e52c0 跨平台 CI 通过 |
| W10 | P2 | 审查 webm-sys 的 C++ 编译参数在 MSVC 上产生 D9002；按真实编译器族选择 flag，保留固定来源与许可证 | 独立 WIN-WEBM-MSVC-01 / PR #14；本机完整 QA 绑定 e4ccc46，45769c9 七项 CI 与完整 QA workflow 全成功，新 Windows 包来源/哈希/签名身份已核对；真实桌面未验证 |
| W11 | P1 | 新 Windows runner 使用 CRLF 检出时的 IPC 负例与结构回归；保留两种换行的正/负合同 | 独立 CRLF checkout 1284 项通过，fe37aec Windows 前端 CI 已通过 |
| W12 | P1 | vendored xcap 保持固定 LF 字节并运行原始 SHA-256 校验；不能归一化哈希输入或跳过检查 | 独立 CRLF checkout 前端门禁 11 项通过、0 失败；字节篡改仍被拒绝，fe37aec Windows 前端 CI 已通过 |
| W13 | P1 | Windows 原生进程/locale 测试预算覆盖实测初始化；保留子进程硬超时、全部断言与普通单元测试默认预算 | CI 暴露两项 5 秒超时；限定测试组补齐预算后，Node 24.21.0 + CRLF 全前端门禁通过，fe37aec Windows 前端 CI 已通过 |
| W14 | P1 | 浏览器 OCR 捕获来源 HTML/脚本固定 LF，保留原始字节哈希和来源记录；新 CRLF clone 与篡改负例验证 | fe37aec CI 复现来源哈希失配；28186af 全新 CRLF clone 来源哈希通过，额外 LF 负例被拒绝；42e52c0 CI 通过 |
| W15 | P1 | OCR 取消/回收夹具区分启动与执行预算；慢启动、取消、后继排队、kill/wait 与许可顺序，macOS 实际 PID 回收 | 独立 OCR-PROC-CANCEL-01 / PR #15；Windows 探针与 7982866 完整七项 CI 通过，Unix 用例实际成功；继承修复的 #14 七项另作证据，桌面未验证 |
| W16 | P1 | Windows PowerShell 5.1 版本检查的引号传输；保留失败和版本边界合同 | 独立 WIN-PS-GATE-01；ef78a1f 本机完整门禁通过，文档后继 b2fd247 七项 CI 通过；新 SHA 与桌面另验 |
| W17 | P2 | 小图 Pin 完整工具栏及上下间隙；创建、缩小与恢复位置共用高度合同 | 独立 WIN-PIN-TOOLBAR-01；旧包实际失败、前端回归先失败后通过，7aa6cf6 本机完整门禁通过；修复后桌面和新 SHA CI 未验 |
| W18 | P2 | 私有文件权限准备失败不得先写内容或截断原文；真实文件故障注入与 Windows DACL | 独立 WIN-PRIVATE-WRITE-01；Windows 红绿及六项定向合同通过，f788b1f 本机完整默认/QA 门禁通过；跨账户、路径竞争及新 SHA CI 未验 |
| W19 | P2 | 自动长截图提前失败的光标恢复不能抢回用户已移动位置，查询失败关闭恢复 | 独立 WIN-LONGSHOT-CURSOR-01；同一生产 guard 红绿及八项定向合同通过，d8dff80 本机完整默认/QA 门禁通过；真实接管、X11/macOS 原生图及新 SHA CI 未验 |
| W20 | P1 | Windows CF_HTML 片段范围受实际字节与 UTF-8 边界约束，默认门禁不能遗漏依赖库合同 | 独立 WIN-CF-HTML-01；旧校验离线红基线、安全解析九项合同及 50b7778 完整本机默认/QA 门禁通过；CI 入口已接线，远程新 SHA、真实互操作和其它原生图未验 |
| W21 | P1 | Windows PNG / DIB 在整图像素分配前执行已有预算，保留合法 4K/8K 与小图像素 | 独立 WIN-CLIP-IMAGE-BUDGET-01；红基线 3 passed / 2 failed，预算七项及 531d791 完整 Windows 默认/QA 门禁通过，新 SHA CI 与桌面未验 |
| W22 | P2 | Windows DIBV5 显式像素偏移，防止小图读取失败与尾部掩盖错图 | 独立 WIN-DIBV5-PIXEL-01；红基线 2 passed / 2 failed，原 Chrome/Firefox 及新增像素/文件视图合同、25fb5d7 完整 Windows 本机默认/QA 门禁通过；真实提供者和新 SHA CI 未验 |
| W23 | P1 | Windows 首次按键前复核当前目标，不能沿用激活/后端初始化前的窗口身份与焦点 | 独立 WIN-PASTE-RECHECK-01；同一生产入口红基线 2 passed / 4 failed，六项离线合同及 14bf616 完整 Windows 本机默认/QA 门禁通过；真实接管、系统竞争、macOS 原生图和新 SHA CI 未验 |
| W24 | P1 | 正常 WASAPI Stop 保留已复制 PCM 和有限 endpoint 尾包，Pause 清空，故障关闭 | 独立 WIN-WASAPI-STOP-TAIL-01；旧控制协议红基线 9 passed / 4 failed，13 项真实 Cargo 合同与 a463c3b 完整 Windows 默认/QA 门禁通过；真实设备/混音与新 SHA CI 未验 |
| W25 | P1 | WGC Close 每轮均尝试两个资源，按成功状态幂等，失败允许 Drop 重试 | 独立 WIN-WGC-CLOSE-01；红基线 1 passed / 5 failed，六项 vendor Cargo 合同与三个原始字节篡改负例通过；03b4cb8 完整 Windows 默认/QA 门禁通过；系统最终释放、真实 WGC 与新 SHA CI 未验 |
| W26 | P2 | 双轨编码退出不因视频桥接 panic 遗漏音频 join，两条回收后保留既有错误 | 独立 REC-AV-BRIDGE-JOIN-01；真实受控线程红基线 2 passed / 2 failed，四项 QA Cargo 回归与 091b5cb 完整 Windows 默认/QA 门禁通过；共享其它宿主图、新 SHA CI、实际设备 panic 未验 |
| W27 | P2 | pool 创建后、完整 WgcRuntime 前的两处失败先 Close，成功转移所有权 | 独立 WIN-WGC-INIT-ROLLBACK-01；红基线 1 passed / 3 failed，四项 vendor Cargo 合同与 61d6823 完整 Windows 默认/QA 门禁通过；真实 API 失败、系统最终释放与新 SHA CI 未验 |
| W28 | P1 | Windows 构建号仅解析成功返回的有界字节范围，缓冲区完全初始化 | 独立 WIN-REGISTRY-BUFFER-01；安全旧单位模型 4 passed / 4 failed，八项真实 vendor 合同与三项篡改负例通过；8c6fbfe 首次严格 lint 失败已修复并保留，bb38cc6 完整门禁 30 passed / 0 failed；新 SHA CI、实际注册表与桌面待验 |
| W29 | P1 | Windows 跨屏物理窗口按每帧比例转换/裁剪，保留分数边界与 Z 顺序 | 独立 WIN-WINDOW-SCALE-01；旧函数抽取协议 2 passed / 6 failed，八项 MSVC 回归与 78bd83f 完整默认/QA 门禁通过，30 passed / 0 failed / 1 Linux smoke skipped；首次包装器退出码缺失不计通过、原记录保留，修正后同 SHA 重跑退出 0；真实多屏与新 SHA CI 未验 |
| W30 | P1 | Windows 物理光标按各帧比例选择覆盖层键盘归属，保留未知光标/无效元数据兜底 | 独立 WIN-OVERLAY-FOCUS-01；旧生产 reveal 红基线 2 passed / 6 failed，八项绿回归与 f577996 完整默认/QA 门禁通过、子进程 exit 0，30 passed / 0 failed / 1 Linux smoke skipped；原断言审计通过，真实 set_focus、原生建窗、多屏与当前 SHA CI 未验 |
| W31 | P1 | 冻结原始物理边界贯穿 Windows 覆盖层/guide、光标、窗口候选、长截图指针与重捕获身份 | 独立 WIN-NATIVE-MONITOR-01；MSVC 实际旧算法红基线 1 passed / 15 failed、十六项绿回归；首次完整门禁 28 passed / 2 lint failed 保留，修正后的 2f0e225 完整默认/QA 门禁 exit 0，30 passed / 0 failed / 1 Linux smoke skipped。实际窗口/DPI、多屏、Pin/WGC 原点身份与当前 SHA CI 未验 |
| W32 | P1 | 冻结物理来源贯穿截图/长截图及历史图片 Pin，单次原生规划与两阶段物理请求 | 独立 WIN-PIN-ORIGIN-01；旧生产输出及提取布局/请求协议红基线 3 passed / 15 failed，同组十八项 MSVC 离线回归和 888127a 完整默认/QA 门禁通过、原生子进程 exit 0；30 passed / 0 failed / 1 Linux smoke skipped。真实窗口/DPI、工作区/工具条/WGC、新 SHA CI 与其它宿主未验 |
| W33 | P1 | 原生 owner/工作区贯通 Pin 保存、恢复及客户区工具条边界 | 独立 WIN-PIN-WORKAREA-01；提取旧生产协议红基线 1 passed / 15 failed，同组十六项 MSVC 回归和 4a58101 完整默认/QA 门禁通过，原生子进程 exit 0；30 passed / 0 failed / 1 Linux smoke skipped。真实 OS owner/窗口/DPI/热插拔、多屏/Windows 10、WGC 与当前 SHA CI 未验 |
| W34 | P2 | Windows Pin 实时 DPI 渲染判据、首读/事件竞争与 caller-bound 只读查询 | 独立 WIN-PIN-LIVE-DPI-01；原 App 十二项红基线 2 passed / 10 failed，同组全绿、三项 API 与六项 MSVC 纯数据读取合同通过；e339042 完整 Windows 默认/QA 门禁确认原生子进程 exit 0，30 passed / 0 failed / 1 Linux smoke skipped。真实事件/成像、多屏/Windows 10 与新 SHA CI 未验 |
| W35 | P2 | WGC 应用帧桥启动回滚与销毁拥有取消/join，不依赖外部 sender 释放 | 独立 WIN-WGC-BRIDGE-ROLLBACK-01；提取旧协议红基线 3 passed / 4 failed，同组七项真实线程合同全绿；7922457 完整默认/QA 门禁 exit 0，30 pass / 0 fail / 1 Linux skip；实际 WGC/Close/系统释放、硬件与新 SHA CI 未验 |

W04–W07 使用 `docs/native-qa.md` 和 `scripts/manual-qa.mjs` 的 Windows profile。
安装包证据与本地源码构建分开，模板初始 `not_run` 不能计作通过。

## Verification

本机：Windows 11 Pro for Workstations x64，build 22000；Node 22.22.0，Python 3.12.7。
用户已授权安装工具链。Rust 1.98.1 MSVC、VS 2022 C++ Build Tools/Windows SDK、MSYS2 已安装；
WebView2 Runtime 154.0.4258.37 已核对，Rust LLVM tools 与 LLVM 23.1.2 libclang 已安装，
VS 附 CMake 3.31.6-msvc6 已补入当前会话 PATH。原生默认与录屏 QA 检查通过。

`ci-windows.ps1 -FrontendOnly`：11 项检查通过，0 失败，3 组显式跳过；这是完整前端范围，
属于整体部分门禁。Vitest 74 个文件、1277 项通过；包含 3 项真实 PowerShell 退出码合同。
Python 质量合同 31 项通过，视觉段落 3 项通过，智能擦除证据校验通过。
默认完整 Windows 范围：20 项检查通过、0 失败、2 组显式跳过；Rust 1040 项通过、5 项忽略，
check、严格 clippy、vendor WGC 严格 clippy 通过。前端追加缺依赖回归后为 74 文件、1278 项通过。
录屏 `-RecordingQa` 最终完整 Windows 范围：23 项检查通过、0 失败、1 组 Linux smoke 显式跳过。
QA Rust 1093 项通过、5 项忽略；前端最终为 74 文件、1280 项通过，含 6 项门禁退出码/前置依赖合同。
默认与 QA 测试大量重叠，不累加为独立覆盖数。Python 质量 31 项、视觉段落 3 项通过。
独立 `core.autocrlf=true` checkout 修复 W11/W12 后：完整前端范围 11 项检查通过、0 失败、
3 组显式跳过，74 文件 / 1284 项通过。主 Rust 源码保持 CRLF，xcap 和 Cargo 锁文件按属性检出 LF；
刻意在固定文件追加一个 LF 被原始 SHA-256 校验拒绝，复原后校验成功。
全新 CRLF clone 在 `47f1ee7dcd5f234d3bc5756cebe6202de2f5fc47` 首次检出即保持 vendor/锁文件 LF，
原始哈希校验成功。本机同 SHA 使用 `recording-windows-av-qa`、debug、`--no-sign` 构建 MSI/NSIS，
两包成功且为 NotSigned；绑定 SHA 的 LOCAL-BUILD.json 和 SHA256SUMS.txt 保存在 ignored target 目录。
这些仅属诊断构建证据，不计入测试通过数，也不替代官方 Native QA 包、签名或安装验收。
用户确认目前仅有当前 Windows 11 单屏，暂无多屏或 Windows 10 验收环境。
CI SHA `c9e504c` 原五项 CRLF 合同已通过，但 PowerShell/首次 locale 两项触发默认 5 秒超时。
W13 补丁仅给 Windows 原生测试组有界预算，PowerShell 子进程仍在 20 秒硬超时后失败；无重试/删断言。
与 CI 同版本的 Node 24.21.0 在 workspace 内隔离下载并验证官方 SHA-256，CRLF 前端门禁为
11 项通过、0 失败、3 组显式跳过；74 文件 / 1284 项通过。系统 Node 版本未替换。
当时 Linux 本地完整门禁、修改后 CI、官方 QA 安装包、Windows 10 和桌面交互尚未执行；后续结果见下文。

详细审查证据见 `docs/reviews/2026-10-01-windows-native-review.md`。所有新结果按层级追加，
没有实际执行的项不勾选。

W09 后续：fe37aec 的 Windows 前端 CI 通过，OCR 实际 DACL 与手写 SDDL 比较失配，Rust 未执行。
改为二进制 ACE/保护位严格核对，本机 33 项质量 + 3 项视觉段落通过；等价 SID/AI 正例和
10 类真实创建失败关闭负例均保留。当时修改后同 SHA CI 与完整 Windows 门禁仍待复验。

W09/W14 最新源码 28186af 的完整 Windows 默认门禁：20 passed / 0 failed / 2 skipped，
Rust 1040 / 5 ignored、前端 74 文件 / 1284 passed、Python 33 + 3 passed。
全新 CRLF clone 状态干净；两个浏览器语料的额外 LF 负例均 exit 2，验证器与来源登记未改。
桌面自动化运行时再次初始化失败，没有 UI 观测；Windows 11 单屏人工 QA、Windows 10、多屏保持未完成。

### 同 SHA 远程检查与 Windows QA 包

SHA `42e52c064aba36bb7e93a5e68a0cfb54f65c5b7b` 的
[run 36816386762](https://github.com/51hhh/Clippy/actions/runs/36816386762) 七项全部 completed/success。
三项规定的原生结果由仓库评估器核对 authenticated check-runs；Windows 日志为前端 74 文件 /
1284 passed、Python 33 项质量 + 3 项视觉段落、Rust 1040 passed / 5 ignored。
这是该 SHA 的 CI 证据，不把后续证据文档提交或独立 W10 SHA 写成已验证。

同 SHA 的 [Native QA run 36817675012](https://github.com/51hhh/Clippy/actions/runs/36817675012)
在三项原生检查成功后启动。Windows job 110226195519 已 success；官方 MSI/NSIS 已下载，
QA-BUILD 的完整 SHA、版本、平台、QA feature、自签名用途与 SHA256SUMS 均核对一致。
CI 临时信任环境的 Authenticode Valid/预期签名者检查通过；本机两包状态为 UnknownError，
消息为证书链终止于不受信任根，签名 thumbprint 与 CI 预期相同。未导入证书或修改本机信任。
完整包身份和哈希见审查记录；安装包构建/下载不加入测试通过数。
该 QA workflow 最终 completed/success，四个平台 bundle 与 Ubuntu 24 X11 smoke 全部成功，
六份产物 manifest 均绑定完整 SHA；这些不替代 Wayland 或 Windows 桌面验收。

W10 的 437949b CI 最终为六项 success / 一项 failure，失败是 macOS 原生 OCR 取消/回收测试
未观察到 PID 标记（1054 passed / 1 failed / 5 ignored）。该模块未由 C++ 补丁修改；原夹具
执行预算 250 ms、启动观察 10 s 存在竞态。W15 使用阶段标记、慢启动和真实回收探针单独修复，
不改生产 OCR 预算或取消语义；Windows 探针与 Unix 原生合同分别记录。
7982866 的 Ubuntu/macOS 已实际执行并通过该合同，独立 #15 的 run 36821712880 完整七项 success。
经用户授权同步 #15 后，#14 / 45769c9 的七项全部 success，范围仍六个 WebM 文件；新 SHA 官方
QA run 36823747034 全部 success，Windows 包来源/哈希/签名身份与 MSI 只读元数据已核对，
39 项模板均 not_run。不将旧包/本机门禁替代新包或桌面验收，也不将文档提交 SHA 写成源码 CI 已验证。

官方 Windows 11 模板与本地生成模板字节相同，39 项均 not_run。合成 Unicode、富文本和透明 PNG
夹具已准备且哈希核对完成；用户表示暂不能手动执行，安装、桌面与录屏真机验收继续未完成。
Windows 10、多屏/负坐标/混合 DPI、真实音频、升级/updater、Linux 本地完整门禁与 Wayland 回归
不由 CI 或包校验替代。没有合入 dev、发布或默认开放录屏。
