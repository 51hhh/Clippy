# WIN-NATIVE-01 / W55 — 缓存帧与实时 PCM 预算

需求 `REC-PENDING-FRAME-PCM-01`；基线 `bdffab38b6130a155203fea2cee5d92a26c3db23`，
原生产 `6518661`；独立分支 `codex/recording-pending-frame-budget`。规格
[`2026-10-02-recording-pending-frame-budget.md`](../superpowers/specs/2026-10-02-recording-pending-frame-budget.md)
在生产修改前创建，并先写清容量/分段合同。本轮仅代码和已有 Windows 原生 CLI。

## 已复现的问题

原 WGC bridge 线程接收两个合成 xcap Frame，原采集 worker 仍以 1 FPS 交付。
第二帧留在原缓存内，使源下界不能跨过该帧。PCM 使用同一 RecordingSessionClock，
每块 48 frame（1 ms）等时钟达到块末尾才交付，不快进 PTS。实际 AudioCaptureWorker
在约 1.05 秒返回 `Pipeline(Backpressure)`，初始红 native exit 101，0/1。
这证明原 worker/codec 路径的组合缺陷；没有连接 WGC/WASAPI 设备。

## 修复行为

未封闭视频 slot 等待时，按每条 interleaver 剩余包槽 × 960，扣除 Opus 内部分 PCM，
取全局/分段容量最小值；最多消费到下一精确 CFR 边界。耗尽即等待视频事件；原
三十二包 enqueue guard、三帧和一秒 PCM 预算不变。源下界、共享时钟、真实 FPS、
native captured/input 与时间戳没有提高或重写；尚未编码的 slot 仍可由真实帧替换。

到达周期边界，只有真实音频 head 证明还有后续媒体时才先按 sample 切分，再消费新
slot 的 PCM；保留占位图像和首 keyframe。恰在边界 Stop 不创建空尾段；EOS 先编码
已消费 PCM 对应的 slot，排空包队列后补齐尾部。视频实际发出 packet 才确认新分段
包含视频。已经由真实 head 证明的 PCM 空洞也受相同容量与 CFR 边界约束。
仅明确声明源下界、且已经建立真实视频 epoch 的 AV 路径启用；其它来源沿用旧等待。

## 验证证据

首轮新增九项通过，含实际 bridge/worker/实时 PCM、包槽耗尽仍失败关闭、部分 Opus
PCM 容量、真实同 slot 替换、精确/边界 Stop、15/30 FPS 分段严格 reader。
新增十三项最终覆盖暂停/恢复、Error/Drop、确认空洞；旧领域全部通过。原完整测试模块正文与既有
严格 reader 不改；新 API 合同与旧实现可执行的相同运行期夹具分开记录。

第二轮领域 376/1；唯一失败是新暂停夹具把恢复后帧 PTS 写成与 resume 控制时间
完全相同，原 timeline 严格递增检查拒绝。新帧/PCM 改为恢复后一纳秒，保留原生产
时间合同与全部旧测试。第三轮也为 376/1，新 PCM 尾部随之晚一纳秒，夹具的 Stop
边界还停留在旧值，触发原 FinishBeforeBufferedAudioEnd；Stop 和期望总时长同步
延后一纳秒，不改原生产容差或期限。两轮原阶段日志保留，实时背压用例均通过。

同字节运行期夹具旧实现0/1（实际Backpressure）、修复领域377/0=原364+新13。
干净 `120b3add51e0d569cdc5e4442f0a4577c12f7b88` 完整 Windows 门禁33 passed / 0 failed / 1 Linux smoke skipped；
默认Rust1227/QA1343各5ignored，两图不累加；前端81文件/1403passed。
新13仅QA，已含在Rust总数内；原五个完整测试模块、Windows生产/原测试与严格reader保持。
红绿运行期/bridge/helper字节相同，无接口stub；十八份Rust输入绑定干净SHA。
最终旧实现重放与finally恢复、最终领域及完整门禁的实际日志/输入哈希已保留。

证据目录 `src-tauri/target/recording-pending-frame-budget-contract/` 保留原 W54 状态、
初始红、阶段绿、最终旧实现重放及恢复、最终领域、日志/输入哈希与冻结 SHA 门禁。
文件/Git 核对不计测试通过数；默认与 QA 两图重叠，新增项已经包含在 Rust 总数中。

## 未完成项

桌面操作继续停止。实际 WGC/WASAPI 长时录屏、安装器/无 CRT 启动、当前 SHA
三宿主/codec CI、其它宿主原生门禁、Windows 10/混合 DPI 多屏/负坐标与发布未验。
Linux/macOS 仍不声明空闲源下界，不能借共享图通过称为其静止画面录屏已验。
本 SHA release 未构建，旧安装 `45769c9` 和历史 QA EXE `1c66112` 不含本修复。
W53 原 AVI 周期提交测试一次 30 秒超时原因未定位；旧失败记录保留。
panic 只覆盖 unwind，release panic=abort 不承诺 Drop。旧桌面记录仍为
2 pass / 1 fail / 36 not_run，整体目标未完成。
