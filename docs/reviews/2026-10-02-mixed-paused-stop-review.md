# REC-MIXED-PAUSED-STOP-01 / W72 — 混音暂停后的正常停止

基线 `2f410bcfdeefc3c6f4f1b4814bca74197a820d06`；规格先行 `6fcd9f5a50ab8635fa16338c9e4c3f8904b6b876`，原 API 诊断/策略 `a1f0c4c046df97ba63b03b4a74270ff65765ce57`。
修复源码 `02005c98ee6ac52fd46e20c878a717f314fd90cc`，独立分支 `codex/windows-mixed-paused-stop`。
规格见 [REC-MIXED-PAUSED-STOP-01](../superpowers/specs/2026-10-02-mixed-paused-stop.md)，承接
[PX-REC-AUDIO-MIX-01](../superpowers/specs/2026-09-23-recording-audio-mix.md) 第7/8项。

## 问题与修复

暂停已丢弃未提交输入，但原Stop仍将两路控制水位推进到停止时刻并持续pop_ready，生成暂停期间静音。
原worker拒绝暂停期间入队，返回Pipeline(AlreadyPaused)；原完整A/V owner返回同根音频错误并标记interrupted。
已接受前缀仍保留，但正常Pause→Stop无法正常完成文件。

Stop仍先尝试停止两源，分别验证原生控制下界、有限尾块格式/序号/区间/预算和真实早停。
已暂停时不再生成stopped-ready PCM；真实尾块若仍在暂存队列中则明确返回InputWhilePaused，不能静默丢弃。
返回原派生停止控制下界，由原worker/common A/V时间线扣除暂停，保留活动媒体和已接受PCM。
原生PTS/样本/增益、Exact、源身份、公共A/V暂停、W69取整与W70活跃恢复/排尾策略保持。
生产修改仅在audio_mixer.rs；av_session.rs仅接入新测试，原worker无需修改。

## 原实现对照与实际文件

三份规范夹具字节冻结，十二项原API在未改生产实现上为6 passed / 6 failed，Cargo101、包装器/终端1。
原源有首包时产生96960帧暂停静音，无首包时97920帧；两路先后停止均可重现。
原worker保留960/0帧前缀但返回AlreadyPaused；原完整owner同根AudioCapture(Pipeline(AlreadyPaused))、interrupted。
真正停止失败、暂停中实际尾块、控制倒退和格式错误等六项保护通过。实际根错误、前缀、
两音源/一视频源Drop、原journal/已存在媒体均保存并核对哈希。

修复后同十二项通过，完整录屏领域491/0 = 原479 + 新12。
原119份录屏文件的旧完整测试正文/原期限保持；新10项默认/QA两图，2项完整owner只在QA。
源正常Pause→Stop输出0帧，原worker正常封尾20ms、逐样本保留960帧前缀；无首包时0帧。
原完整owner为complete20ms/2视频帧/960编码输入及有效PCM，无启动裁剪；音源Drop2、视频Drop1。
真正麦克风Stop失败仍为同源AudioCapture(Source(...))、interrupted，两个Stop均尝试。
异常尾块由混音源明确拒绝；直接进入原单源worker的暂停中PCM仍严格返回Pipeline(AlreadyPaused)。

独立ffprobe和ffmpeg只读取合成WebM，exit0：2 VP9/2 Opus包，pre-skip312/discard648，
960有效/解码PCM帧、0.020s；解码信号RMS 0.3529378649，符合原输入信号。
WebM SHA-256 `bfae4cb2bc51344b8228e519e503874b9a3cb3dd8a567db62874f8ab1d7ef3b3`。未播放或启动原生设备，文件证据不计设备验收。

首版新AV夹具访问私有字段导致编译E0616，零项测试执行；改为夹具工厂参数后才建立规范基线。
首版夹具/原日志和退出码单列保留，不计产品缺陷；生产字段可见性和原测试未放宽。

## 完整 Windows 验证与边界

干净 `02005c98ee6ac52fd46e20c878a717f314fd90cc` 完整 `./scripts/ci-windows.ps1 -RecordingQa`，native/包装器/终端exit0。
33 passed / 0 failed / 1 Linux smoke skipped；默认Rust1308、QA1457，各5ignored；前端81文件/1403passed。
新12项已含总数，实际22次执行（10两图/2仅QA）；领域/两图重叠不累加，ignored/skipped/构建/文件不计通过数。
本轮一个生产修复，累计47项本地修复；门禁后只改七份Markdown，无生产/测试变化。

当前源码release/同SHA跨平台及codec CI、其它宿主、真实WGC/WASAPI、设备切换/漂移/长时同步、
桌面/安装器/updater/无CRT启动、Win10/多屏/混合DPI仍未验。W71 B0默认/QA release文件保留为前置源码证据，
不含本次修复，已安装457包也不含本次修复；39项桌面记录保持。
W53/W59/W63/W67历史失败根因仍未明，旧日志/失败材料保持；默认录屏入口仍关闭。
未启动应用/设备/桌面，未安装/签名/新增工具/证书/推送/PR/合入/发布。
规格最后复合验收和整体任务仍未完成。

证据：`src-tauri/target/mixed-paused-stop-contract/`、`mixed-paused-stop-native-qa-02005c9/`，
包含原始日志/退出码、冻结输入/同字节夹具、原API诊断、前缀/原journal/媒体、文件解码与合同/门禁审计。
