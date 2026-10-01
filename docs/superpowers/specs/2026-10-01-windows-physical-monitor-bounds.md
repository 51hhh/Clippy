# WIN-NATIVE-MONITOR-01 — Windows 冻结帧保留物理显示器边界

## Goal

修复逐屏逻辑原点取整后反算 Windows 物理桌面坐标的误差，以及覆盖层/guide 使用逻辑建窗点
导致目标显示器歧义的问题。对应 WIN-NATIVE-01 / W04 的 W31，基线 4630b38。
用户要求保持桌面操作停止；本轮只做代码、离线回归与 Windows 原生编译验证。

## Requirements

1. Windows xcap 原始显示器 x/y/width/height 在逻辑归一化前保存，并随 FrozenFrame、
   CapturedMonitorFrame 传递；不得由取整后的逻辑原点、当前窗口 DPI 或枚举顺序推算。
   原始物理尺寸与冻结像素不一致、边界为空/溢出、权威元数据缺失时拒绝猜测。
2. 截图和 QA 录屏选区覆盖层、长截图 guide 使用同一冻结物理边界构造 PhysicalPosition /
   PhysicalSize 请求；Windows 隐藏建窗不提交有歧义的逻辑位置提示。其它平台保留原定位路径。
3. Windows 光标归属直接命中原始物理半开边界；DWM 窗口候选先减原始物理原点，再逐轴
   除冻结比例。继续保留既有无效光标兜底、局部裁剪、候选顺序和会话/label 校验。
4. Windows 长截图滚动点为原始物理原点加冻结物理 crop 中心；重捕获签名包含原始物理边界，
   即使逻辑取整结果未改变，物理边界漂移也拒绝复用。产物 Pin 的既有逻辑来源 IPC 不在本修复内。
5. 使用真实领域类型、实际生产入口与锁定 dpi/Tauri 类型做离线回归；覆盖正/负非整除原点、
   125%/150%、上下/左右布局、各轴比例、物理边缘、无效/缺失边界及逻辑不变的物理漂移。
   旧算法取得 MSVC 红基线，修复运行同组原断言；不启动窗口、读取屏幕、光标或发送输入。
6. 完整 Windows 默认/录屏 QA 门禁绑定干净源码 SHA；CHANGELOG、计划、review 引用同一 ID。
   Windows 10/多屏、DPI 事件/热插拔、实际窗口结果、当前 SHA 跨平台 CI 保留未验证。

## Acceptance Criteria

- [x] 原始物理边界从逐屏冻结到消费者完整保留；不匹配/缺失边界失败关闭。
- [x] 覆盖层与 guide 的请求为物理类型，不受假定的当前/目标 DPI 影响；Windows 无逻辑位置提示。
- [x] 光标、窗口候选、长截图滚动点及重捕获身份使用同一权威物理边界。
- [ ] 实际旧算法红基线与修复后绿回归保留，既有回归和 Windows 完整门禁通过。
- [ ] 文档按代码和证据同步；未完成的实际桌面与跨平台验收仍保留。

## Out of Scope

桌面操作、实际 set_position/set_size/set_focus 和 WM_DPICHANGED 结果、显示器热插拔竞争、
Pin 逻辑来源 IPC 的迁移、WGC 新枚举的原点身份比较、安装新包、Linux/WSL、推送/新 PR、
合入 dev 或发布。本合同不关闭 W04 的真机矩阵，也不以原生编译代替桌面验收。

## Review evidence

现有 normalize_monitor_geometry 将物理原点 2560 在 150% 下取整为逻辑 1707；乘回是 2560.5。
负原点有同类误差。窗口候选、光标边界、长截图滚动点均只能看到归一化后的原点。
锁定 Tao 0.34.8 的 Windows 建窗按各屏 DPI 转换逻辑位置并选首个命中屏，set_outer_position
使用当前窗口比例；逐屏归一化的全局逻辑位置可同时命中两屏。Tauri 2.10.3 set_position /
set_size 接受锁定 dpi 的 Physical 变体，因此保留原始物理边界可以消除请求单位歧义。

## Validation

应用真实 Cargo/MSVC 图：旧算法 exit 101，1 passed / 15 failed / 0 ignored，1079 filtered。
先补内部元数据并抽取原有纯数据入口；原 focus/probe/scroll/signature 正文与基线字节一致，
没有算法模型或类型桩。新增五份回归文件原始字节保持，修复后 exit 0，16 passed / 0 failed /
0 ignored，1079 filtered。回归没有调用显示器/窗口枚举、截图、窗口建造、光标或输入 API。
初次绿色编译报告 Windows 不再使用逻辑 contains 方法；该方法仅保留其它平台，严格门禁待验。
红源码/日志和 SHA-256 保存在 src-tauri/target/windows-physical-monitor-contract/RESULT.json。
现有测试仅补内部物理元数据、辅助构造可见性，保留原断言；guide 夹具同时补齐移动后的物理事实。
Windows 默认/录屏 QA 完整门禁、当前 SHA CI、其它宿主与实际桌面待记录或待验，不能计为已通过。
