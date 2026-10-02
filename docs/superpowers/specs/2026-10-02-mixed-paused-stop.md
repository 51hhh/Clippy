# REC-MIXED-PAUSED-STOP-01 — 双源暂停后的正常停止

## Goal

基线 `2f410bcfdeefc3c6f4f1b4814bca74197a820d06`，生产源码 `b0b51f1`。
承接 `WIN-NATIVE-01`、`PX-REC-AUDIO-MIX-01` 第7/8项及 `REC-MIXED-CONTROL-SKEW-01`。
用原实现和原 API 确认暂停后直接停止是否错误生成暂停期间的静音、使原 worker/完整 A/V owner 失败。
确认后使正常停止只封闭原有媒体时间线，保留暂停前已接受的 PCM/恢复分段和资源回收。

## Requirements

1. 成功暂停的双源已清除未提交输入；正常 Stop 返回两路原生停止控制下界，不得凭暂停时长
   制造 PCM 或扩张 stopped-ready 输出。起始尚无 PCM、两路 Stop 时刻不同也遵守此合同。
2. 暂停后 Stop 仍必须尝试停止两源，分别验证原生控制时刻、有限尾块格式/序号/区间/预算和真实早停。
   暂停期间意外返回的有效 PCM 必须明确失败，不能静默忽略；真实源失败保留源身份和原错误。
3. 不放宽原 worker 的 IgnoredWhilePaused 拒绝，不改公共 A/V 暂停扣时、输入 PTS/PCM/增益、
   Exact、平台精度或 W69/W70 活跃状态首包/有限尾部策略。原完整模块和原期限保持。
4. 同字节原 API 先在未改生产实现上运行，覆盖源、原 worker、完整 A/V owner，记录实际
   根错误、已接受 PCM 前缀、两源 Drop、journal/媒体；确认问题后才选择生产修复策略。
5. 修复后运行完整录屏领域和干净源码 Windows 默认/QA 完整门禁，核对真实 codec 文件。
   更新 CHANGELOG 与审查；合成源/文件、本机测试、release、同 SHA CI、设备验收分层，不累加通过数。

## Acceptance Criteria

- [x] 两路停止顺序和无首包情况下暂停后 Stop 不产生 PCM，返回实际控制下界。
- [x] 原 worker 正常封尾并保留完整已接受 PCM/两源 Drop，真正的暂停中入队继续拒绝。
- [x] 源停止失败、控制倒退、异常格式/真实尾块仍失败；正常/错误完整 A/V owner 的原文件保存。
- [ ] 同字节原实现对照、旧模块/期限、完整领域及干净源码 Windows 默认/QA 门禁通过，记录同步。
- [ ] 当前源码 release/同 SHA 跨平台 CI、原生设备/长时同步和桌面验收完成。

## Out of Scope

不启动应用/设备/桌面，不安装/签名/新增工具/证书/推送/PR/合入/发布。
默认录屏入口保持关闭；其它宿主/设备异常和时钟漂移另验。
W53/W59/W63/W67 历史失败根因继续未明，旧日志/失败材料和39项桌面记录保持。
W71同SHA release文件为前置源码证据，不计本轮新测试或新修复。

## 原 API 诊断与选定策略

原十二项同字节规范基线为6 passed / 6 failed，Cargo101、包装器/终端1；原119份录屏文件和
原完整模块/期限保持。原实现暂停后产生96960帧静音，无首包时97920帧；worker保留960/0帧前缀，
Stop返回Pipeline(AlreadyPaused)。原完整owner同根AudioCapture(Pipeline(AlreadyPaused))、interrupted，
两个音源/一个视频源Drop和原journal/媒体保存。真正停止失败等六项保护通过，不计设备验收。
首版新AV夹具访问私有字段导致编译E0616，零项执行；修正为夹具工厂参数后才建立上述规范基线。
首版日志/夹具单列保存，不计产品缺陷。

停止仍先尝试两路并检查原生时刻、有限尾块与各自实际末尾。已暂停时不生成stopped-ready静音；
若有限尾块包含实际PCM则明确失败，不静默丢弃，不放宽原worker的暂停中入队拒绝。
返回真实派生Stop下界，由原pipeline/common A/V时间线扣除暂停；活跃状态的W69/W70排尾策略保持。

## 领域与文件核对

同十二项夹具字节保持，修复后完整领域491 passed / 0 failed = 原479 + 新12。
源暂停后停止输出0帧，原worker正常封尾20ms、逐样本保留960帧已接受前缀，无首包时0帧。
原完整owner为complete20ms/2视频帧/960输入及有效PCM；音源Drop2、视频Drop1。
真正麦克风停止失败仍为同源AudioCapture(Source(...))、interrupted，两路Stop均已尝试。
原119份文件的旧完整模块/期限保持，原worker单源暂停中PCM仍报Pipeline(AlreadyPaused)。
独立ffprobe/ffmpeg仅检查合成文件：2 VP9/2 Opus包、pre-skip312/discard648、960有效PCM、20ms，
解码信号RMS约0.353；不播放、不开设备。完整门禁待干净源码提交，最后复合验收保留未完成。
