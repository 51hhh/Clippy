# REC-PENDING-FRAME-PCM-01 — 低 FPS 缓存帧与 PCM 预算

## Goal

以 W54 文档 `bdffab3` / 源码 `6518661` 为基线，独立复现 Windows WGC 缓存帧限制
下界、1 FPS 采集节流与一秒 PCM 队列的组合。只有真实原生 CLI/worker/codec 证据
证实缺陷后才修改生产；合成 bridge 输入不代表实际 WGC/WASAPI 设备测量。

## Requirements

1. 保留原共享时钟、WGC receive/stamp/replace 与缓存下界合同。实际 bridge 线程接收
   两个合成帧，采集 worker 仍按 1 FPS 交付；PCM 用同一会话时钟等待到样本末尾再交付，
   不用快进媒体时间伪称实时故障。回归必须取得实际采集 worker 原错。
2. 若证实背压，消费不得越过尚可由真实帧替换的 slot，也不得把源下界提高到缓存旧帧
   之后。不能提前计 native captured/input、提高真实采集率或增加帧/包/PCM 预算。
3. 可以利用原有编码包预算，但在实际修改前写清容量合同，保留三十二包失败关闭、严格
   WebM 顺序、sample/CFR 分段、真实帧替换、Stop/暂停/恢复/Error/Drop 与报告核对。
4. 保留旧测试正文和 W54 所有原始阶段证据；每个真实恢复分段及最终输出通过严格
   reader。相同夹具重放旧行为，与新 API 只能绿色执行的测试分开列明。

## 已复现与容量合同（生产修改前）

`red-RESULT.json` 的实时夹具在 1.05 秒取得实际音频 worker 的
`Pipeline(Backpressure)`。缓存的第二个原帧仍限制源下界；没有改变源时间戳、FPS 或预算。

未封闭 slot 等待时，PCM 最多前进到下一 CFR slot 的 sample 边界，且每个 Opus encoder
可接收的 frame 数不得超过 `剩余包槽 × 960 − encoder 内已有的部分 frame`（下限零）。
取全局/分段两条容量的最小值；三十二包原有 enqueue 检查仍失败关闭。容量耗尽后等待
视频事件，不继续消费音频，也不提前编码可替换的视频 slot。只有显式源下界与真实 epoch
存在时启用；无下界来源仍沿用原等待合同。

若下一未封闭 slot 已达到分段周期，须先在它的起点按 sample 游标切分，再接收该 slot
内的 PCM；保留可替换图像并强制首 keyframe。只有真实音频 head 证明还存在后续媒体时
才预切分，避免恰在边界 Stop 创建空尾段。已证实的 PCM 空洞也受相同容量/slot 边界限制。
EOS 可以编码已经消费 PCM 所在的最终 slot 后排空，不能在包槽已满时盲目补齐 PCM。

## Acceptance Criteria

- [x] 1 FPS 的真实 bridge/capture/AV/PCM 实时组合已独立复现或得到可重复的否定证据。
- [x] 如复现缺陷，相同运行期夹具旧实现红、新实现绿，原预算与时间/统计合同保持。
- [x] 不对齐 CFR/分段、后到真实帧、暂停/恢复/Stop/Error/Drop 和严格文件检查通过。
- [x] 干净 source SHA 的完整 Windows 默认/录屏 QA 门禁通过；图重叠/ignored/skip 单列。
- [ ] 当前 SHA 三宿主/codec CI 与实际设备/安装器/多屏验收通过。

## Out of Scope

桌面、应用与设备继续停止；不安装工具、改证书、推送、建 PR、合入或发布。Linux/macOS
原生源仍无空闲下界，不借共享图称为实际静态录屏已验。W53 未修改 AVI 测试的一次
30 秒超时原因保留；其它全局验收与新 SHA release 均未完成。

## Verified source

同字节运行期夹具旧实现0/1（实际Backpressure）、修复领域377/0=原364+新13。
干净 `120b3add51e0d569cdc5e4442f0a4577c12f7b88` 完整 Windows 门禁33 passed / 0 failed / 1 Linux smoke skipped；
默认Rust1227/QA1343各5ignored，两图不累加；前端81文件/1403passed。
新13仅QA，已含在Rust总数内；原五个完整测试模块、Windows生产/原测试与严格reader保持。
红绿运行期/bridge/helper字节相同，无接口stub；十八份Rust输入绑定干净SHA。
最终旧实现重放与finally恢复、最终领域及完整门禁的实际日志/输入哈希已保留。
证据：`recording-pending-frame-budget-native-qa-120b3ad/RESULT.json`、
`recording-pending-frame-budget-contract/COMMITTED-CONTRACT-AUDIT.json`、`REVIEW-CLOSURE.json`。
设备/跨宿主/当前CI/安装器/多屏/发布及W53 AVI旧超时仍未验。
