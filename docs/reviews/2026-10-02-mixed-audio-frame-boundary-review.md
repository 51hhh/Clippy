# REC-MIXED-FRAME-BOUNDARY-01 / W69 — 混音输出帧与源控制边界审查

基线 `0cbf42c8c9ad4d3b0bb5721906d2d4573294ffa7`；规格先行 `b0dc61ce18d3cd89032274774f5752dc84eb78d0`，原诊断/策略文档 `c866832`。
修复源码 `4bc197767dfb04c7d067ee55c7744ea668dcde76`，分支 `codex/windows-mixed-audio-frame-boundary`。
规格见 [REC-MIXED-FRAME-BOUNDARY-01](../superpowers/specs/2026-10-02-mixed-audio-frame-boundary.md)。

## 问题与修复

两个原生音源返回纳秒控制时刻，混音输入/output cursor 使用 nearest 48 kHz 帧。
原首块被取整到源承诺的下界之前；派生 PCM 末尾又可能超过返回的 Stop/Pause。
同组原 API 十项回归 2 passed / 8 failed、Cargo exit101、包装器/终端exit1，生产未改。
实际保存启动早5000ns、恢复 SourceTimestampNotIncreasing、封尾短833ns 的 FinishBeforeBufferedAudioEnd。
原 worker 的641帧前缀可读且两源Drop；原完整 A/V owner同根错误、interrupted文件已保留。
这是原代码路径证据，不证明设备实际采用该 packet/控制节奏。

保持输入原生PTS/样本/序号、绝对网格、增益、预算与静音水位。
输出段首锚定派生启动/恢复边界，以相对帧数计时；控制时刻覆盖已输出PCM与已返回下界。
恢复严格晚于派生暂停至少1ns；归一化前分别拒绝两路原生控制时刻倒退。
输入恢复游标仍从原生恢复时刻取帧，避免派生时刻跨 nearest 中点而拒绝首包。
输出PTS/末尾/下一序号的溢出在释放staging和移动游标前拒绝。
Exact pipeline、原两源失败身份/回滚、公共A/V扣时算法、平台/WASAPI精度合同与默认门控保持。

## 回归和文件

三个原十项夹具字节冻结，修复后同十项通过；额外一项恢复取整中点保护未声称原实现红基线。
新11项中10在默认/QA两图，1仅QA。最终录屏领域470/0 = 原459 + 新11。
冻结112份原录屏文件；两个父模块的旧完整测试与其余原文件保持，旧期限未改。
原worker正常Stop接收641帧（一个起始静音帧+640混音输入帧），全部样本幅度检查与两源Drop通过。
原A/V owner使用真实VP9/Opus和journal，complete20ms、2视频帧、641输入帧、960有效PCM，无启动裁剪。
保存最终WebM SHA-256 `dca5d92760eabc2b3e95f30ce3ce6185d79d0d4578888409381cc066ae40c91b`。
独立ffprobe exit0，2 VP9/2 Opus包，pre-skip312、discard648、有效960PCM和0.020s；未播放。
原基线、首版领域469/0与最终470/0日志/文件均保留，不累加通过数。

## 完整本机门禁与未完成项

干净 `4bc197767dfb04c7d067ee55c7744ea668dcde76` 执行完整 `./scripts/ci-windows.ps1 -RecordingQa`，native/包装器/终端exit0。
33 passed / 0 failed / 1 Linux smoke skipped；默认Rust1291、QA1436，各5ignored；前端81文件/1403passed。
11项新合同已含总数，实际21次两图执行（10两图/1仅QA），领域与两图重叠不累加。
本轮一项修复，累计45项本地修复；构建/文件不计测试通过。

本源码release/同SHA跨平台及codec CI、其它宿主、真实WGC/WASAPI、设备切换/漂移/长时同步、桌面、
安装器/updater/无CRT启动、Win10/多屏/混合DPI仍未验；W68 release属于前置AAC源码，已安装457包未更新。
W53/W59/W63/W67历史失败根因继续未明，原日志/失败材料和39项桌面记录保持。
没有应用/设备/桌面操作、安装、签名、新工具/证书、推送、PR、合入或发布；默认录屏入口仍关闭。
原规格最后复合验收与整体任务仍未完成。

证据在 `src-tauri/target/mixed-frame-boundary-contract/` 和 `mixed-frame-native-qa-4bc1977/`，包括原始日志、
native退出码、冻结输入、同字节夹具、根错误/PCM前缀、AV文件、独立合同/门禁审计。
