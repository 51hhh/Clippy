# REC-AV-STARTUP-GATE-01 — 双轨会话初始化前的采集释放

## Goal

基于 `cc5f7af` 审查双轨会话的启动顺序，避免视频 factory 尚未完成时音频已进入有界 PCM
pipeline。对应 WIN-NATIVE-01 / W50，独立分支 `codex/recording-av-startup-gate`。

## Requirements

1. 原 `AvRecordingSession` 先初始化并运行音频 worker，再初始化视频 worker；encoder 等待
   首视频帧，音频队列只有一秒 PCM 预算。用真实会话/worker 及受控 source 检查启动期间
   是否提前轮询音频，不把受控延迟当作已经观察到的 WGC/WASAPI 真机失败。
2. 两个 factory 均在各自采集线程成功初始化后，owner 才释放两条采集循环。单轨原入口
   保持立即采集；不增加 PCM 容量、不隐藏运行期背压、不改变共享时钟或暂停/停止合同。
3. factory 返回错误保留原错误；unwind 测试配置的 factory panic 保留原分类。release 的
   `panic=abort` 不承诺 panic 捕获或 Drop 回收。等待期间通道关闭或 worker Drop/Stop 能取消等待，
   同步回收 source 与线程，保留线程亲和。不能把初始化屏障变成新的 Drop/join 死锁。
4. 这是 worker 向 pipeline 的释放协调，平台 source constructor 的原生流/回调启动不改；
   不声称设备在同一物理时刻启动或原生首包行为已验证。现有 native 缓冲与首视频 epoch
   对齐保留，运行期背压仍失败关闭。
5. 原 AV 会话完成/暂停/失败、单轨与 worker 用例保持；新增失败关闭与等待取消回归，
   对新干净 SHA 跑完整 Windows 默认/QA 门禁和隔离 unsigned/unbundled QA release 编译，
   核对实际 payload/来源，记录真正退出和跨宿主未验证项。

## Acceptance Criteria

- [x] 用原生产入口复现视频初始化未结束时提前音频轮询；原三项 AV 合同保持通过。
- [x] 双 factory 就绪前没有 pipeline 采集，成功后释放两轨，错误/panic 不提前轮询音频。
- [x] 等待期间释放、通道关闭、Drop/Stop 取消和 source 同线程回收有对应回归。
- [ ] 原用例保留，新 SHA 完整 Windows 默认/QA 门禁、隔离 QA release 与源码/日志核对通过。
- [ ] 当前 SHA 其它宿主/原生 CI 与 Windows 真机延迟初始化、设备/录屏通过。

## Out of Scope

不恢复桌面、运行应用、打开设备或安装包，不安装新工具或改系统信任；不推送、PR、合入、
发布或运行 Linux/WSL。不修改默认录屏 feature、容器/预算/时钟或原生 source constructor。
Windows 10、多屏、真实设备/权限/热切换/长时漂移与其它宿主仍待验。

## Verification

原生产启动顺序的同组六项为 3 passed / 3 failed：原 AV 三项通过，新三项均观察到视频
factory 等待期间音频已轮询五次。先准备两个线程亲和 source，再通过独立释放通道启动
worker 循环，生产修复后同六项通过，新三项正文未改。这个回归证明提前轮询，队列预算
耗尽风险由一秒 PCM 容量/首视频等待推导，不声称已经在设备上复现溢出。

两个实际 worker 的释放、断开、Drop/Stop、非 Send source 同线程回收八项补充回归通过；
录屏领域 301 passed，新十一项在内。完整新 SHA 门禁/release 编译待核对。原生 source
constructor、实际首包/缓存、设备启动与阻塞中的 factory 中断能力不在本轮验证范围。
