# WIN-WGC-BRIDGE-ROLLBACK-01 — Windows WGC 帧桥启动回滚

## Goal

让 Windows WGC 启动失败回滚与成功帧源销毁都拥有并回收桥接线程，避免构造帧源之前的错误
丢弃 JoinHandle。对应 WIN-NATIVE-01 / W06 的 W35，基线 f8409b7；桌面操作保持停止。

## Requirements

1. bridge 线程成功创建后立即进入拥有取消令牌和 JoinHandle 的 owner。recorder.start 失败
   返回前取消并 join 该线程；原 Initialize 错误不能被清理中的线程 panic 替代。
   成功则把 recorder/bridge owner 一起转移给真实 WindowsWgcRegionFrameSource。
2. 桥接线程取消不依赖所有帧 sender 已释放；真实接收循环用有界等待观察取消，在退出时关闭
   FrameBridge 并释放接收端。既有零容量回调通道、最新一帧、单调时间戳和 Frame 类型保持。
3. 正常销毁在原生 stop/drop 之前发出线程取消，随后 join；暂停/继续/stop_capture 不取消
   永久帧桥，保留既有停止/恢复协议。原生 Close/Drop 的成功与时限不能由纯线程合同证明。
4. 验证真实生产 start/transfer 和 frame forward 数据入口，使用实际 std Thread/JoinHandle、
   Receiver<xcap::Frame>、FrameBridge 与 RecordingSessionClock。可控 start callback 模拟
   原生错误，不构造显示器、WGC/窗口对象或调用鼠标输入；不以这些合同替代真实 WGC 结果。
5. 回归覆盖启动失败仍等待线程退出、保留 sender 的取消、原错误与 bridge panic、成功转移、
   接收端关闭、真实帧转发/时间戳和正常销毁。旧生产所有权/接收循环协议提取红基线保留；
   同组断言修复后通过，不能把提取入口称为未经改动的原 connect 或桌面复现。
6. 干净 SHA 完整 Windows 默认/QA 门禁、原断言/新测试字节/源码/日志哈希核对；同一需求 ID
   同步 spec、总计划、审查和 CHANGELOG。供应链 patch、非 Windows 平台源、feature 默认值保持。

## Acceptance Criteria

- [x] start 失败取消并 join 已启动的桥接线程，原错误保持。
- [x] sender 被额外持有时取消仍使真实帧循环退出；退出关闭 FrameBridge。
- [x] 成功转移与正常销毁拥有同一线程 owner，暂停/恢复/stop_capture 语义保持。
- [ ] 提取旧协议红基线、同组新回归和干净 SHA Windows 默认/QA 门禁核对完成。
- [ ] 同 ID 文档同步，实际 WGC/系统释放/硬件与当前 SHA CI 仍保留未验证。

## Out of Scope

不修改 vendor WinRT Close/初始化回滚，不保证 native Close 的时限或最终系统释放，不改录屏
monitor-local crop、媒体/音频/编码格式，不操控桌面/安装包、不运行 Linux/WSL、不推送/合入/发布。
Windows 10/11 真机、设备/多屏/热插拔/长期漂移及当前 SHA 原生 CI 保留未验。

## Review evidence

WindowsWgcRegionFrameSource::connect 先 spawn 桥接线程，再 recorder.start()?。此时尚未构造
完整帧源，错误会丢弃 JoinHandle；完整帧源的 Drop 才有显式 join。原 recv() 只因 sender 全部
释放才退出；不能把 xcap recorder 的字段析构当作桥接线程已经退出的证据。
WGC runtime 关闭/初始化回滚见 WIN-WGC-CLOSE-01 与 WIN-WGC-INIT-ROLLBACK-01，本需求只补齐
应用帧桥 owner/取消/转移协议；实际原生错误和系统释放仍未观测。

## Validation

Windows MSVC 提取旧生产所有权/接收协议红基线 exit 101：3 passed / 4 failed / 0 ignored /
1135 filtered。接收循环正文与 f8409b7 逐 token 一致（归一化空白）；不是未经修改的原 connect。
同一七项回归原始字节不变，修复后 exit 0，7 passed / 0 failed / 0 ignored / 1135 filtered。
使用真实 std 线程/通道和生产 start/transfer/shutdown/forward 入口，原生错误由 callback 注入，
没有创建 WGC/窗口、枚举显示器或发送输入。七段既有帧/裁剪/暂停恢复等正文与十份关联文件保持；
vendor 和默认 feature 未改。完整干净 SHA Windows 默认/QA 门禁待执行；真实 Close 时限与系统
释放、实际 WGC/设备/硬件矩阵及新 SHA CI 保留未验。
证据目录 C:\win\Clippy\src-tauri\target\windows-wgc-bridge-rollback-contract。
