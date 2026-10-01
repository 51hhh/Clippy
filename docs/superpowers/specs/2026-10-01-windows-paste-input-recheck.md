# WIN-PASTE-RECHECK-01 — Windows 输入后端初始化后的粘贴目标复核

## Goal

在激活/等待和输入后端初始化之后复核目标，避免此期间目标窗口销毁、
HWND 所有者变化或用户切换焦点后仍发送 Ctrl+V。作为 WIN-NATIVE-01 / W05 的独立 W23 修复，
基于 949a2d9；不操控桌面或宣称实际错误粘贴已经发生。

## Requirements

1. 保留目标捕获失败清空旧值、原始 HWND/PID 校验、完整性边界、SetForegroundWindow 和 500ms 截止。
   Windows 输入后端初始化完成后，在首次按键之前重新检查窗口仍有效、PID 仍相同且前台仍为目标。
2. 复核失败返回现有 NativeTargetInvalid / NativeFocusNotRestored；内容仍留在剪贴板，command 层
   继续结构化 copy-only 降级，不重新抢焦点、不尝试向其它窗口注入、不打印剪贴板内容。
3. 生产初始化、复核和注入经过同一个有明确顺序的入口；故障注入在初始化期间改变窗口快照，
   必须证实后续注入没有发生，初始化失败也不触发复核或注入。
4. 测试全部使用离线回调与窗口快照，不初始化 Enigo、不调用 Win32 窗口 API、SendInput 或桌面。
   正常成功路径及初始化/复核错误保持原结构化结果，释放修饰键的既有逻辑保留。
5. macOS 共用初始化入口的原行为保持，Mac 验证回调为空；X11/Wayland 不改。CHANGELOG、
   总计划、规格和本机源码 SHA 证据同步，原生 CI 与真实用户接管另外验证。

## Acceptance Criteria

- [x] 保持旧初始化/注入行为的离线红基线在窗口销毁、PID 变化或焦点变化情况下失败。
- [x] 修复后同一生产入口拒绝这些状态，不触发注入；正常顺序与初始化失败合同通过。
- [ ] 干净源码 SHA 的完整 Windows 默认/录屏 QA 本机门禁通过，两个 Rust 图分别记录。
- [ ] 文档和 CHANGELOG 同步，桌面接管、新 SHA CI 与 macOS 原生图保留未验证。

## Out of Scope

对系统焦点与 SendInput 的原子绑定、最后复核后仍可能发生的 OS 输入竞争、重写键盘后端、
更改权限/焦点恢复策略、物理键状态接管、macOS 新窗口判定、Linux 验收、桌面操作、安装新包、
合入 dev 或发布。

## Review Evidence

Windows paste 的前台轮询成功后调用 inject_paste，后者先 Enigo::new 再直接按键。
现有 HWND/PID 和完整性检查发生在激活目标之前；输入后端初始化以后没有新快照校验。
这是源码可达的检查间隔，实际桌面注入期间的原子性不由离线合同证明。

微软 [GetWindowThreadProcessId 合同](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowthreadprocessid)
明确失败不改变 PID 输出；此处初始化为零，查询失败按无效目标拒绝。
[IsWindow 文档](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-iswindow)
明确句柄可能在检查后销毁/复用，因此快照复核不是系统原子性保证。

## Verification

Windows MSVC 离线 harness include 完整生产 native.rs 的行为保留提取版，链接锁定 Enigo 0.6.1；
仅外围错误类型/平台 facade 为测试 stub，原生权限入口调用即 panic。测试使用 fake 初始化/注入
回调和窗口快照，不执行 Enigo、窗口 API 或 SendInput；旧入口六项为 2 passed / 4 failed，exit 101。
销毁、未知 PID、焦点变化仍进入注入回调，成功顺序缺复核。修复后同一六项 6 passed，exit 0。
原有修饰键清理语句与错误分类保留；真实 Cargo 全图尚待验证，harness 不替代该门禁。
循环负例在红状态的首个断言即停止，不能声称每个循环输入各自已红复现；绿状态执行全部输入。

待记录完整本机门禁与源码 SHA。桌面停止，旧安装包保持不变。
