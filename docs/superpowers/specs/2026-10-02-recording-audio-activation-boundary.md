# REC-AUDIO-ACTIVATION-BOUNDARY-01 — 音频激活边界与 Windows 控制时钟

## Goal

基于 `b791112c8866bebf57fae4c81241be260242e129`，核对并修复 Windows 包末尾下界导致
快速恢复返回相同控制时间，以及合法首块 PCM 恰好位于恢复时刻却被拒绝的问题。
承接 `PX-REC-AUDIO-WORKER-01`、`PX-REC-WINDOWS-AUDIO-01` 与
`REC-AV-CONTROL-TIMELINE-01`；默认录屏入口保持关闭。

## Requirements

1. Windows 控制时间不早于已复制 PCM 的末尾，且严格晚于前一次控制返回值。
   在原会话时间域中用有溢出检查的 1 ns 控制下界排序，不能重写原生 packet PTS、另建原点
   或通过等待设备/虚构 PCM 来掩盖时间戳错误；时钟耗尽返回明确错误并走原清理路径。
2. 恢复控制时间是后续 PCM 的包含下界。恰好位于该下界的首块可以入队一次；随后真实 PCM
   时间戳与序号仍严格递增，早于下界、重复块、区间重叠和无效原 source 恢复仍被拒绝。
3. 恢复标记只在成功入队后消费；背压失败不改变时间线，释放容量后同一块可以重试。
   暂停/恢复/封尾状态转换保持原始错误、线程亲和、一次 source hook 与析构合同。
4. 使用原 worker/pipeline API 做同字节对照，并用原完整双轨 session、真实 VP9/Opus、
   journal 与实际文件验证包含下界。旧 stateless Windows helper 的失败组合与新有状态控制
   合同分别记录，不能把新 API 绿色测试伪称为原 API 红绿证明。
5. 原完整测试模块、期限、平台 packet 映射、暂停媒体公共区间、队列预算、FPS 与 feature 边界
   保持；冻结干净源码的 Windows 默认/QA 完整门禁另验。

## Acceptance Criteria

- [x] 保存原 API 的恢复包含下界拒绝、线程/队列和实际会话错误，以及 Windows stateless 组合结果。
- [x] 恢复边界首次 PCM、重复/迟到拒绝、背压原子重试与完整双轨实际文件通过。
- [ ] Windows 包末尾夹持、相同/倒退 clock 读数、多轮控制、溢出与错误后状态保持通过；原生调用接线编译/lint 通过。
- [ ] 原完整模块/期限保持，冻结干净源码 Windows 默认/QA 完整门禁通过。
- [ ] 同 SHA CI、其它宿主、release、真实 WGC/WASAPI、设备/桌面/长时同步另验。

## Out of Scope

不启动设备、应用或桌面，不安装、推送、合入或发布；不改变源 PCM PTS、QPC 校准、
格式/帧率/容量或既有公共暂停扣时。合成 source 不计真实设备证据。
W53/W59/W63 历史失败原因继续未明，不由本次独立合同归因。
