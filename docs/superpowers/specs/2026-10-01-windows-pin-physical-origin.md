# WIN-PIN-ORIGIN-01 — Windows 图片 Pin 保留冻结物理来源

## Goal

修复混合 DPI 下截图/长截图及复制后从历史创建的图片 Pin 用全局逻辑点猜显示器，导致回错屏、
尺寸和位置偏移的问题。对应 WIN-NATIVE-01 / W04 的 W32，基线 e1c7191；桌面操作保持停止。

## Requirements

1. Windows 普通截图输出的来源由 caller 绑定的冻结帧与实际 floor/ceil/clamp crop 产生，不能
   相信前端提交的全局逻辑原点或物理元数据。长截图来源包含真实 signed union offset。
   原始物理显示器边界与物理内容原点随输出 claim、重试、复制像素指纹和 Pin 条目保留。
2. PinOrigin 的公开 JSON 保持 x/y/width/height；物理来源仅是后端私有字段，禁止反序列化
   前端提供的物理位置。非 Windows 行为保持；源元数据不完整不能猜测原始物理位置。
3. 一次图片 Pin 规划用同一份原生显示器/工作区/DPI 快照；按原始物理显示器身份选屏，
   以实际 PNG 像素尺寸计算 CSS 内容尺寸，原尺寸只缩不放。窗口 gutter/工具条下限保持。
   不因逻辑空间重叠或显示器枚举顺序换屏；未知来源或原屏已移除按光标/主屏正常创建，
   继续遵守来源登记失败不能阻止 Pin 的既有合同。
4. 图片 Pin 隐藏创建与 reveal 使用同一规划的物理位置/尺寸请求，不依赖窗口当前 DPI，
   reveal 不再用逻辑请求覆盖已确认的位置。Windows resize 已算出的物理位置不再转换为
   逻辑点重复提交；置顶状态保持 entry.above。真实异步 DPI 事件仍需桌面验收。
5. 回归覆盖两种混合缩放、非整除/负/上下原点、各轴、crop 边缘、实际 PNG 尺寸、复制后
   PNG 重编码、输出重试、前端伪造/省略来源、无效/失去原屏、两阶段与不同当前 DPI。
   真实领域类型/生产纯数据入口和锁定 Tauri/dpi 请求类型；不创建窗口、枚举桌面或发送输入。
   保存旧来源/布局协议的 MSVC 红基线，同组断言修复后通过，再跑干净 SHA 默认/QA 门禁。
6. 规格、CHANGELOG、计划和 review 使用同一 ID；本机门禁、远程 CI 与桌面分层记录。

## Acceptance Criteria

- [x] 冻结物理来源贯穿普通/长截图、输出重试、像素登记与图片 Pin；IPC 私有字段不可伪造。
- [x] 原始目标屏和 PNG 像素尺寸决定图片 Pin 规划，逻辑重叠/枚举顺序不影响结果。
- [x] 创建/reveal 物理请求一致，resize 不再回到逻辑位置；未知/失去来源仍可正常 Pin。
- [x] 已保存旧协议红基线；修复回归、既有测试和 Windows 完整默认/QA 门禁通过，证据完整。
- [x] 文档同步，实际多屏/Windows 10/DPI/跨平台原生图及当前 SHA CI 保留未验。

## Out of Scope

存量 Pin workspace 的持久化坐标迁移、工具条屏幕交集查询的其它路径、文字/HTML Pin 的来源
语义、WGC 源身份、实际窗口/DPI 热插拔结果、桌面操作、安装新包、Linux/WSL、推送/新 PR、
合入 dev 或发布。W04 全部路径与真机矩阵仍保留；本项覆盖新建图片 Pin 的完整来源与窗口请求链。

## Review evidence

主屏物理 [0,2560) 为 100%，右屏物理 [2560,4480) 为 150%；右屏逻辑原点约 1707。
右屏局部 x=200 的全局逻辑 x=1907 同时落在主屏工作区，logical_work_area 取首个命中。
origin_content_size、device/buffer scale、初次物理 anchor 和 reveal 的 LogicalPosition 都
依赖这个猜测。前端来源只是 payload.logicalX + selection.x，原始物理身份没有进入 PinOrigin。
resize 已提交 PhysicalPosition 后又经 keep_pin_above 提交逻辑位置，DPI 变更期间可重新换算。

## Validation

2026-10-02：Windows MSVC 旧生产 claim/frame adapter 与提取的旧布局/请求协议红基线
3 passed / 15 failed / 0 ignored / 1095 filtered；修复后同组十八项通过。提取的布局是生产入口
共用的纯数据内核，不把它描述成未修改的原窗口函数或实际窗口结果。三份新回归文件及 resize
断言保持红绿一致。初次脚手架 PhysicalRect PartialEq 编译错误及首次 PNG Fast/NoFilter
重编码字节相同的夹具失败分别保留；改用 Best 压缩后重新取得红基线，未删减断言。

干净源码 `888127a226dab8737b219b879b7ca1add0875358` 完整默认/录屏 QA 门禁确认原生子进程
exit 0，30 passed / 0 failed / 1 skipped（Linux smoke）。默认 Rust 1108 / 5 ignored，QA Rust
1165 / 5 ignored；重叠不累加，每图十八项及此前物理边界十六项、焦点/候选各八项包含在总数。
前端 75 文件 / 1292 passed，Python 33 + 3，独立 vendor 十八项及剪贴板二十四项通过。
check、严格 clippy、供应链、构建、入口和日志哈希均通过。10 个既有文件 465 个 assert 宏 token
保留（仅归一化空白及新增 cfg 私有默认字段），新回归原始字节红绿一致；四份锁定原生库源码哈希核对。
同 SHA 门禁前后检出干净，后继仅四份 Markdown。完整结果及审计：
`C:\win\Clippy\src-tauri\target\windows-pin-origin-native-qa-888127a\RESULT.json`。
红绿/源码/断言证据目录：
`C:\win\Clippy\src-tauri\target\windows-pin-physical-origin-contract`。
本次新增回归不创建窗口、调用显示器/光标 API 或输入；使用实际 Tauri/dpi 类型和原生 MSVC 图。
实际窗口/DPI、Windows 10/多屏、工作区迁移、工具条交集、WGC、新 SHA CI 与其它宿主仍未验。
