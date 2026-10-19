# REC-LIBRARY-READY-01 — 结果库就绪回调的生命周期

## Goal

结果库加载完成或显示加载错误时，只有仍活动的组件 effect 可以请求显示窗口。
已卸载、被后继服务取代或 StrictMode 退休的加载不能再发送 ready。
对应 WIN-NATIVE-01 / W42，基线 a2c31d2；沿用原结果库隐藏创建与 ready 显示/聚焦合同。
未改动的生产组件与 deferred list 已复现退休后的 ready 缺口。

## Requirements

1. ready 绑定发起 load 的 effect 生命周期，cleanup 后成功与失败加载均不能调用原 services.ready。
   不能只检查共享 mounted：后继 effect 或 StrictMode 重放会重新使它为 true。
2. 活动 effect 正常加载完成后调用 ready 一次；加载失败也显示原错误/重试页并调用 ready，
   不能因加载错误让隐藏窗口一直不可见。加载未完成时不提前 ready。
3. 当前 ready Promise 拒绝继续按原合同处理，不新增未处理错误或遮蔽已加载内容。
   被取代的旧加载不能重复 ready，也不能污染当前数据、加载/错误状态。
4. 保持原 load generation、播放/动作状态与卸载清理、IPC/权限、Rust 隐藏/显示/聚焦逻辑、
   StrictMode 入口、文案/依赖与默认录屏门控；不改变已经发出的原生 ready 请求。
5. 生产 App 原字节红基线，八项 deferred list/StrictMode jsdom 回归同组修复后通过；
   旧组件/API/播放生命周期三文件原字节与旧断言保持。ready 是替身调用，不是实际聚焦证据。
6. 干净 SHA 完整 Windows 默认/QA 门禁与证据/同 ID 文档同步，图谱重叠不累加。
   真机窗口/焦点/播放、其它宿主、当前 SHA CI 与安装更新保留未验。

## Acceptance Criteria

- [x] 原组件在退休加载完成后确实调用 ready，红基线与观测范围保留。
- [x] 卸载后的成功/失败、服务取代后的成功/失败与 StrictMode 旧 effect 不再 ready。
- [x] 活动成功、活动加载失败和当前 ready 拒绝三项原显示/错误行为保持。
- [x] 同组八项与旧 32 项/三文件原字节、生产限定改动和关联文件核对。
- [x] 干净源码完整 Windows 门禁与文档/证据同步，真机与其它平台保留未验。

## Out of Scope

桌面操控停止。不启动应用/浏览器、原生 show/set_focus、不安装、推送/PR/合入/发布。
StrictMode 测试是开发/test effect 重放，不称为发布包实际重复聚焦。不能撤销此前已发送的
ready 原生请求，也不扩展为所有动作或窗口销毁/重建策略。

## Verification

原 App.tsx 保持原字节，生产组件/jsdom 红基线 35 passed / 5 failed（旧 32 全绿，新增
八项 3/5），native Node/终端 exit 1。四项退休完成实际调用旧 ready 一次，StrictMode
dev/test 重放实际调用两次；不是原生 focus 或发布 effect 重放观察。原隐藏创建/ready
的 show/set_focus 只读源码核对，不调用该 API。

当前 effect 使用自己的 retired 闭包标记，cleanup 置 true；共享 mounted 的后继重置
不能恢复旧 ready。正常加载与错误页仍由当前 finally 通知一次，ready 拒绝继续处理。
同组八项原字节修复后通过，包含旧 32 项共 40 passed（4 文件），Node/终端 exit 0，
无未处理错误。旧三文件原字节、十三份关联文件保持；去除限定 guard 后 App 全部原正文
保持，无其它状态/播放/IPC/后台/StrictMode 入口/依赖/文案修改。

干净源码 19078092d739781dc14713f35dfa3da02ab980ac 完整 Windows 默认/QA 门禁确认原生
child/终端 exit 0，30 passed / 0 failed / 1 Linux smoke skipped。默认 Rust 1165、QA Rust
1228（各 5 ignored，重叠不累加），无新增 Rust 用例；旧媒体十三项/导出身份七项各图及
九条 manifest/导出/合并按原图通过。前端 79 文件 / 1323 passed，新八项在前端总数内。
简略全量汇总与同一默认配置发现清单 79/1323（含新八项/旧 32）、定向绿日志和测试/源码
哈希核对；发现 exit 0，不重复执行测试主体。Python 33 + 3、独立 vendor 十八项和
剪贴板二十四项、check、严格 lint、供应链、构建/入口通过。两份源码/原始日志/checked
helper/门禁脚本哈希与门禁前后干净检出核对，门禁后只改四份 Markdown，生产/测试保持。
保存实际 QA/全未运行模板原字节保持，未装更新包。完整证据
C:\win\Clippy\src-tauri\target\recording-library-ready-native-qa-1907809；红基线/原合同/
发现清单/后续快捷键源码证据 C:\win\Clippy\src-tauri\target\recording-library-ready-contract。
真实窗口/focus、其它宿主/当前 SHA CI 未验，桌面操控停止。Tauri Shared 注册结果记账
留为下一步受控回调候选，尚未复现；不调用真实 register/unregister 或系统输入。
