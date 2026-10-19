# REC-AV-GAP-DRAIN-01 — 双轨编码空洞与停止尾段有界排空

## Goal

修复下一视频帧/EOS 已知、Stop 留下较长尾段或分段边界有空洞时，双轨 writer
单独突发生成一轨 packet、耗尽三十二包重排预算而失败的问题。基线 `e4a4a3c`，
源码 `2dccc43`；继承首帧修复，本项单独审查 writer，桌面继续停止。

## Requirements

1. 用实际 VP9/Opus、`SegmentedAvRecordingWriter` 与编码 worker 复现长尾静音、视频
   空洞和恢复分段边界失败；不使用真实 WGC/WASAPI，受控结果不算设备验收。
2. 保持三十二包、PCM/视频/bridge 预算。按已确认的 presentation 下界增量交付两轨，
   保持原生时间戳、视频 CFR 最近帧填充、48 kHz 静音补洞、稳定 packet 顺序与音轨 padding。
3. 视频推进只编码已经确定不能再被未来真实输入替代的 CFR slot，保留下一 slot 的图像；
   同一 slot 的后到真实帧仍可替换，不能把音频到达时间当成视频采集时间。
4. 完成和分段仍校验帧/包/PCM/时长，journal complete 与失败 interrupted 合同保持。
   保留旧测试正文；新增 writer/编码 worker 合同先红后绿，推进边界另有纯编码合同。
5. 不以 writer 修复声称 encoder 在“视频 head 尚未到来”时的采集消费等待已解决；该路径
   若仍存在，作为独立未完成项保留。

## Acceptance Criteria

- [x] 同字节长尾、长视频空洞、分段空洞与实际编码 worker 回归在旧源码失败、修复后通过。
- [x] CFR 同 slot 替换、推进时间边界、完整 manifest 与旧领域回归通过，预算不变。
- [x] 干净 source SHA 完整 Windows 默认/录屏 QA 门禁通过，重叠/ignore/跳过单列。
- [ ] 当前 SHA 三宿主/codec CI、设备录屏/长时同步/Stop 与安装器/多屏验收通过。

## Out of Scope

不控制桌面、不启动应用/设备、不安装系统工具、不改证书、不推送/建 PR/合入/发布。
其它宿主门禁和真实设备性能、Win10/多屏/安装器仍未验；release panic=abort 不承诺回收。

## Evidence plan

`src-tauri/target/recording-av-gap-drain-contract/` 保留原 W51 状态、原实现红、同字节绿、
源码恢复和原测试正文对比；冻结 SHA 完整 Windows 门禁另证。此前启动夹具等待两轨
交付后 Stop，不能代替本项；其失败诊断记录继续保留。

原四项 0/4；最终补充 1 FPS 两项，同字节六项旧源码 0/6，真实错误均为
`InterleaveQueueFull`，finally 恢复三个生产文件。四项 CFR 下界/同 slot/停止与分段
边界合同单列；共十项新增，原三百一十九项保持，最终领域 329 passed。
视频下一 head 尚未到来时的消费等待，以及不对齐 CFR slot 的分段时间映射尚未单独回归。

干净源码 `80e084cdba265d95e259bee8f0c13db32052f928` 完整 Windows 默认/录屏 QA 门禁
child/terminal exit 0：33 passed / 0 failed / 1 Linux smoke skipped；默认 Rust 1214/QA 1295
各 5 ignored，两图重叠，前端 81 文件/1403 passed。新增十项仅 QA，领域 329 包含在 QA
内；门禁结束 checkout 干净。`recording-av-gap-drain-native-qa-80e084c/RESULT.json`、
`recording-av-gap-drain-contract/COMMITTED-CONTRACT-AUDIT.json` 与最终 closure 分层绑定。
后继只改五份 Markdown；本 SHA release/安装器未构建，设备/其它宿主/当前 CI 仍未验。
