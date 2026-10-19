# WIN-CONTROL-ROLLBACK-01 — 录屏控制窗启动回滚清理结算

## Goal

启动失败回滚不能忽略控制窗销毁请求错误并提前开放新的控制窗；与正常关闭共用返回结果结算。
对应 WIN-NATIVE-01 / W06 的 W37，基线 7ca01a0；桌面操作保持停止。

## Requirements

1. 回滚只认 begin_close 返回的精确 session/label；错误或过时 session 不销毁其它控制窗。
2. 销毁请求返回失败必须按失败结算，保留现有 TerminalFailed 隔离，后续 reserve 仍返回 Busy；
   不用重复回滚解除隔离。该状态按既有合同需要进程恢复，不新增自动修复或重试策略。
3. 销毁请求发送成功或窗口已经不存在时按成功结算；旧 caller/label 不得控制随后新建的会话。
   沿用普通关闭协议；请求发送成功不能证明原生窗口已销毁，不新增完成确认协议。
4. 普通关闭与回滚共用结果结算。销毁/结算失败留下日志，原始 build/prepare 错误仍返回给启动调用者，
   不能被次要销毁错误覆盖。普通关闭的原销毁错误及 registry 错误处理保持。
5. 从实际宿主提取回滚/结算入口，使用真实 RecordingControlRegistry/RecordingToken 与 callback
   验证失败、成功、重复回滚和过时会话。提取旧回滚协议红基线保留忽略销毁结果、settle(true)
   的实际顺序；不是未经改动的原 Tauri 宿主或真实原生销毁失败。
6. 原 registry、生命周期、捕获排除、控制窗几何、停止/恢复及配置/权限/feature 合同保持。
   默认录屏门控不开放；共享清理路径仅在本机 Windows 编译/数据测试，不能代表其它宿主通过。
7. 干净 SHA 完整 Windows 默认/QA 门禁、新测试字节/旧断言/源码/日志核对；同 ID 同步 spec、
   总计划、审查与 CHANGELOG，真实窗口销毁/录屏像素/设备/其它宿主及当前 SHA CI 留未验证。

## Acceptance Criteria

- [x] 销毁请求失败按失败结算并阻止替换，重复回滚不解除隔离。
- [x] 成功回滚可重新 reserve，旧 caller 与错 session 不影响新会话。
- [x] 普通关闭和回滚的清理结果/原错误正确，启动原错误保持。
- [x] 提取旧回滚红基线与同组回归、旧断言和干净 SHA Windows 默认/QA 门禁已核对。
- [x] 同 ID 文档同步，真实窗口/捕获/设备和其它宿主/当前 SHA CI 保留未验。

## Out of Scope

不运行或操控桌面，不安装新包、不运行 Linux/WSL、不推送/合入/发布。不修改原生捕获排除 API、
几何后备策略、Windows 版本门槛、录屏格式/设备/恢复算法；不把清理隔离视为实际窗口已销毁。
Windows 10/11 WGC 控制窗像素、真实 destroy 请求失败/原生窗口寿命、强杀/长期 A/V 漂移
和当前 SHA CI 仍待验。

## Review evidence

TauriRecordingDesktopActions::rollback_prepared_control 在 begin_close 后忽略 window.destroy 的
返回值，并总是 settle_close(label, true)。普通 close_control_window 按 destroyed.is_ok()
结算；registry 既有 failed_window_close_blocks_replacement_until_process_recovery 明确要求
失败后阻止替换。启动回滚绕过了这条现有合同。

控制窗在隐藏态设置捕获排除并检查失败，ready/bind 后才 reveal。版本门槛符合
[Microsoft SetWindowDisplayAffinity 说明](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity)：
Windows 10 2004 起支持 WDA_EXCLUDEFROMCAPTURE，旧版本会按 WDA_MONITOR 处理。
原生 API 成功与源码存在不能代替控制窗未出现在 WGC 像素中的真机验收。

## Validation

Windows MSVC 提取原启动回滚协议红基线 exit 101：22 passed / 2 failed / 0 ignored /
1131 filtered。既有十七项控制窗/registry/排除能力合同全绿，新七项为 5 passed / 2 failed；
不是未经改动的原 Tauri 宿主或原生失败复现。同组新测试原始字节不变，修复后 exit 0：
24 passed / 0 failed / 0 ignored / 1131 filtered，使用生产回滚/结算函数与真实 registry/token，
销毁结果由 callback 注入，没有创建窗口、捕获或查询显示器/光标。正常关闭结算正文和原生
销毁 adapter 的表达式保持，DesktopActions 正文、三十七段既有宿主函数与十七份关联文件核对。
启动 build/prepare 返回原 result 的路径保持，回滚仅记录次要清理错误。干净源码
a52ecaa8caa7c44208c96b7c15dfe07794c897fe 完整 Windows 默认/QA 门禁确认原生子进程及终端工具
exit 0：30 passed / 0 failed / 1 Linux smoke skipped。默认 Rust 1150 / 5 ignored，QA Rust
1207 / 5 ignored，图谱重叠不累加；新增七项、旧十七项及准备错误/停止释放合同每图通过。
前端 77 文件 / 1307 passed，Python 33 + 3，独立 vendor 十八项、剪贴板二十四项通过；
check、严格 lint、供应链、构建/入口通过。源码快照、新测试/旧文件、原始日志、checked helper
和门禁脚本哈希、门禁前后干净检出核对；完整门禁证据
C:\win\Clippy\src-tauri\target\windows-control-rollback-native-qa-a52ecaa。

固定 tauri 2.10.3 / tauri-runtime-wry 2.10.1 源码核对：destroy() 转发 dispatcher，Wry 仅将
WindowMessage::Destroy 发送到事件代理；Ok 表示请求发送成功，不证明 HWND 已销毁，Err
也不是 Win32 DestroyWindow 的结果。本次只纠正回滚对该请求结果的处理，普通关闭协议保持。
真实请求失败/原生窗口寿命/Destroyed 时序、控制窗像素/停止/设备/强杀恢复及其它宿主/
当前 SHA CI 仍未验。持久化恢复、merge/delete 所有权与 Windows 导出路径身份待续审，
未将尚未复现的疑点列为新缺陷。
源码/Git 归属、红绿、原合同及此前状态保存在
C:\win\Clippy\src-tauri\target\windows-control-rollback-contract。
