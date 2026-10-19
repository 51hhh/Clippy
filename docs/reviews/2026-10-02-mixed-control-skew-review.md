# REC-MIXED-CONTROL-SKEW-01 / W70 — 双源恢复首包与停止尾部审查

基线 `e72f3a3ec22f083f447fe47c1cb35fc8677ef988`；规格先行 `0a6f814a02a6a469c7203dc64013e02bd6eab3fd`，原 API 诊断/策略 `6db7d4e9e7297d2e70769460bd0700c5cd5a4656`。
修复源码 `b0b51f1196124383b9fc77cfc4ad22c21e8c3d84`，独立分支 `codex/windows-mixed-control-skew`。
规格见 [REC-MIXED-CONTROL-SKEW-01](../superpowers/specs/2026-10-02-mixed-control-skew.md)，承接
[PX-REC-AUDIO-MIX-01](../superpowers/specs/2026-09-23-recording-audio-mix.md) 第7/8项。

## 问题与修复

原恢复输入游标取两路较晚时刻，较早一路的合法首包被当成迟到输入拒绝，worker和完整owner终止。
原Stop排出范围取两源水位的较小值，正常会话虽提交完整40ms文件，较晚20ms PCM却未进编码器，
后半段被编码静音填满。两路谁先恢复、谁先停止均由原API回归覆盖。

恢复输入和输出段首改用两路较早原生下界，保留各路真实时间间隔；较晚源之前的空洞按原网格记静音。
沿用W69控制覆盖完整PCM、严格恢复与帧取整保护。Stop先分别验证各自已接受区间，再将已关闭输入
的静音水位推进到两路Stop/合法PCM的共同末尾。另一源较晚Stop不能掩盖本路真实早停或控制倒退。
原生PTS/PCM/序号、固定增益、Exact、staging预算、源身份/回滚、公共A/V暂停算法与默认门控保持。
生产改动仅在audio_mixer.rs；av_session.rs仅接入新测试模块。

## 原实现对照与文件内容

三个规范夹具字节冻结，九项原API在未改生产实现上为1 passed / 8 failed，Cargo101、包装器/终端1；
修复后同九项通过。七项在默认/QA两图，两项完整AV只在QA。原116份录屏文件和两个父模块的
旧完整测试正文/原期限核对保持；领域479/0 = 原470 + 新9，不累加重叠通过数。
原worker恢复Source(LateInput)，保留960帧前缀；停止正常返回但只提交960/1920帧。
修复后恢复2880、停止1920帧，逐样本幅度/时刻与两源Drop通过。
原完整恢复owner为同根音频错误、interrupted；原尾部owner为complete40ms，输入960、有效文件1920帧。
修复后原完整owner尾部complete40ms/4视频帧/1920输入及有效PCM，恢复complete60ms/6视频帧/2880输入及有效PCM，
无启动裁剪。原日志、根错误、原前缀、journal/媒体与新文件均保留并按哈希核对。

独立ffprobe与ffmpeg文件解码exit0，尾部4 VP9/3 Opus包、1920有效PCM、0.040s；
恢复6 VP9/4 Opus包、2880有效PCM、0.060s。尾部RMS原0.0003887856，
修复后0.0704104206，表明晚一路的预期信号进入实际文件；恢复早一路区间约0.28，
共同区间约0.35。逐样本断言检查编码前信号，解码检查实际有损文件内容；未播放或启动原生设备。
尾部WebM SHA-256 `65a8eda121d431585fb6e65890b1de5d020ce975e1602dcc12ac1b568d7b28b0`。
恢复WebM SHA-256 `3d0b73951844a008307fae89263f6c605537a0a75856e6254c89c759073f26a3`。

初版新视频夹具首帧等于恢复确认，不符合原视频严格时序；其日志/文件单列保留。
修正首帧晚10ms、保守活动控制时刻并等待两音源实际启动后，重新建立规范原生产基线；不计产品修复。
证据脚本读取UTF-8与受限沙箱写证据失败均属于工具执行问题；改用显式UTF-8和已授权证据写入后成功，
不计原生测试失败或新增修复。Windows PowerShell JSON大小写键读取问题改用现有Python读取，状态未改。

## 完整本机门禁与保留边界

干净 `b0b51f1196124383b9fc77cfc4ad22c21e8c3d84` 完整 `./scripts/ci-windows.ps1 -RecordingQa`，native/包装器/终端exit0。
33 passed / 0 failed / 1 Linux smoke skipped；默认Rust1298、QA1445，各5ignored；前端81文件/1403passed。
新9项已含总数，实际16次执行（7两图/2仅QA）；领域与两图重叠，ignored/skipped/构建/文件不计通过数。
本轮一个生产修复，累计46项本地修复；门禁后只有七份Markdown，无生产/测试修改。

W70时本源码release/同SHA跨平台及codec CI、其它宿主、真实WGC/WASAPI、设备切换/漂移/长时同步、桌面、
安装器/updater/无CRT启动、Win10/多屏/混合DPI仍未验。W68 AAC release为前置源码历史证据，
已安装457包不含本修复；39项桌面记录保持。W53/W59/W63/W67历史失败根因仍未明，日志与失败材料保留。
未启动应用/设备/桌面，未安装/签名/新增工具/证书/推送/PR/合入/发布；默认录屏入口仍关闭。
规格最后复合验收与整体任务仍未完成。

证据：`src-tauri/target/mixed-control-skew-contract/`、`mixed-skew-native-qa-b0b51f1/`，
含原始日志/退出码、冻结输入/同字节夹具、原API诊断、PCM前缀/AV文件、文件解码与合同/门禁审计。

W71后续：同源码默认/QA release编译和PE/CRT文件验证完成，实际启动、设备、CI和桌面等仍未验。
见 [release验证](2026-10-02-windows-mixed-control-release-review.md)；最后复合验收继续未完成。
