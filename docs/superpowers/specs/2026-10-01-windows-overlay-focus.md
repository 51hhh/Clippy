# WIN-OVERLAY-FOCUS-01 — Windows 覆盖层按物理光标选择键盘焦点

## Goal

修复 Windows 截图/录屏覆盖层把物理桌面光标直接与逻辑矩形比较，选择错误显示器接键盘的问题。
对应 WIN-NATIVE-01 / W04 的 W30；基于 6a9a84e，桌面操作保持停止。

## Requirements

1. Windows 的 AppHandle::cursor_position 返回 PhysicalPosition<f64>，mark_capture_overlay_ready
   不改变其单位；reveal 必须按每块冻结帧的 scale_x/scale_y 将物理点投影到该帧局部逻辑空间。
   命中左/上边包含、右/下边排除，不用一个显示器或当前调用窗口比例处理其它屏幕。
2. 光标所在覆盖层独占正常路径键盘焦点，与首帧就绪顺序无关；拿不到光标、未命中或元数据
   无效时，沿用第一个就绪覆盖层的兜底，至少有一块能接 Esc。会话/label 校验和 focus_assigned
   状态机保持；无效/非有限比例及空帧不能形成猜测归属。
3. 生产会话保证 frames 与 overlays 同顺序、一帧一个覆盖层；回归通过实际 CaptureManager::reveal
   和实际领域类型构造会话，不调用 begin 中的窗口枚举、不创建窗口、不读光标或发送输入。
   既有双屏测试夹具补齐与覆盖层相符的两帧几何，保持全部原有断言。
4. 单屏/100%、等缩放双屏、混合 DPI 两种左右排布、负坐标、上下排布、各轴比例、边界及
   无效值/未知光标/就绪顺序有独立字面预期。旧 reveal 运行同组测试取得原生 MSVC 红基线。
5. Windows 之外的现有光标/逻辑几何路径不改；默认与录屏 QA 图都运行回归，完整门禁绑定
   干净源码 SHA。CHANGELOG/计划/审查使用同一 ID，桌面与当前 SHA CI 保留未验证。

## Acceptance Criteria

- [x] 原 reveal 在等/混合缩放双屏里把物理光标误分给另一块覆盖层。
- [x] 修复后生产 reveal 合同通过；新测试原始字节/断言保持，既有断言与兜底/会话错误保留。
- [x] 干净源码完整 Windows 默认/QA check、严格 lint、测试、供应链和前端构建通过。
- [x] 原始红绿与门禁证据分层记录，真实桌面、多屏、其它宿主与当前 SHA CI 保留未验。

## Out of Scope

改变显示器原点归一化、保存原始物理原点、覆盖层/guide 原生建窗位置、Pin 和长截图指针、
DPI 变更事件/热插拔竞争、实际系统 set_focus 结果、桌面操作、安装包、Linux/WSL、合入 dev 或发布。
这项焦点判定修复不代表 W04 整体完成；逐帧原点舍入仍待权威物理原点合同处理。

## Review evidence

实际锁定 Tauri 2.10.3 app.rs::cursor_position 返回 PhysicalPosition<f64>；capture/mod.rs 原样
传 x/y 给 manager.reveal。原 reveal 却用 OverlaySpec::contains 比较 frame.x/y/logical_width/height。
等缩放 150% 双屏，物理原点为 0/1920、逻辑原点为 0/1280；物理光标 x=1800 在左屏，却落入
右覆盖层的逻辑 [1280,2560) 范围。左屏先就绪时原判定不给左屏键盘，右屏就绪才抢焦点。
这是源码单位合同缺陷，尚未在真实双屏观察。

另查锁定 Tao 0.34.8 Windows window.rs：建窗会逐屏将逻辑位置乘该屏 DPI，再按枚举顺序取
第一个包含点的显示器；set_outer_position 则用窗口当前比例。逐屏归一化全局原点可重叠，
所以原生覆盖层/guide 位置与原点舍入仍是独立 W04 风险，不能用焦点修复关闭该项。

## Validation

实际应用 Cargo 图的 Windows MSVC 红基线 exit 101：2 passed / 6 failed / 0 ignored，1071 项过滤。
调用原生产 CaptureManager::reveal，无算法辅助模型/类型桩；两块几何帧与 spec 同序对应。
原始源码快照、日志、锁定 Tauri/Tao 源码及调用点哈希位于 windows-overlay-focus-contract/RESULT.json。
同一八项测试文件原始字节不变，修复后 MSVC 绿回归 exit 0：8 passed / 0 failed / 0 ignored，
1071 项过滤。原 reveal 正文与基线一致；既有整个测试模块除双屏夹具替换外原文不变，
新八项测试文件红绿 SHA-256 相同，未弱化断言。

干净被测源码 f5779966f2adaca65d75dd0a0f6affece6e4fc6c 完整 Windows 默认/QA 门禁确认
子进程 exit 0：30 passed / 0 failed / 1 skipped（Linux smoke）。默认 Rust 1074 / 5 ignored，
QA Rust 1131 / 5 ignored（重叠不累加）；两个图分别执行新增八项焦点及既有八项窗口投影回归，
已包含在各自总数。前端 75 文件 / 1292 passed、Python 33 + 3；独立 vendor 十八项及
剪贴板二十四项不包含在应用 Rust 总数。check/严格 lint/供应链/构建/入口、日志原始哈希、
验证后干净检出通过。最终证据 windows-overlay-focus-native-qa-f577996/RESULT.json。
沿用已验证退出 0/17 的无界面 Process 包装器，原始哈希保持，未用缺失退出码推断通过。
后继文档仅四份 Markdown，生产/CI/门禁未变。

W04 剩余源码算例（非测试通过）：2560 / 1.5 取整成 1707，乘回 2560.5，再取整会偏一像素；
Tao 多屏逻辑建窗点的首个命中也有歧义。原始物理原点、原生建窗/guide 与真实 DPI 事件仍待
独立合同和桌面验收；记录 windows-overlay-focus-contract/W04-REMAINING-GEOMETRY.json。
真实系统 set_focus、Windows 10/双屏/负坐标、新 SHA CI、其它宿主与桌面仍未验。
上一轮验证不能替代本次；未运行 Linux/WSL、安装新包、合入 dev 或发布。
