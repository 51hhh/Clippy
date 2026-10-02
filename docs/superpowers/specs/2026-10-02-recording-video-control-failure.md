# REC-VIDEO-CONTROL-FAILURE-01 — 源控制后时间线失败关闭

## Goal

视频源已经执行暂停/恢复后，pipeline 拒绝控制请求时终止采集线程，保留该原始错误，
通过现有 owner/编码联动回收资源并把失败清单保持为 interrupted。
基线为 `4bf8ceeb909787eee0d5ef451fde8a084447dca3`；原视频控制状态预检继续保留。

## Requirements

1. 首帧前暂停、重复暂停、未暂停恢复继续在 source 控制前返回原状态错误；worker 可继续。
2. 已执行 source 控制后，时间戳无效或 pipeline 已关闭/中止等错误必须同时交给请求者
   与 worker join，不能继续轮询、留在 paused 等待，或在后续 Stop 覆盖该根因。
3. 原 abort guard、线程内 source 析构和单轨/双轨失败联动完成资源回收；失败不得 complete。
4. 正常暂停/恢复/停止、source API 错误、队列/FPS、时钟规则、原测试与期限保持。
5. 新合同运行原 worker、pipeline、单轨/QA双轨 session；源依照 WGC running 的控制语义
   注入坏时间戳。文件/codec 使用真实 I/O，不能将合成源误记为实际 WGC/WASAPI 验收。

## Acceptance Criteria

- [x] 原实现中故障控制后线程不退出或 Stop 覆盖根因，有保存的旧 API 对照证据。
- [x] 同字节夹具修复后根错误相同、线程终止/析构，开放 pipeline 中止、原终态及前缀保留。
- [x] 单轨与双轨故障会话在原 owner 中停止，清单 interrupted、无 complete/未提交产物。
- [x] 原控制/状态预检合同保持，干净源码 SHA 的 Windows 默认/QA 完整门禁通过。
- [ ] 当前 SHA CI、其它宿主、真实设备/桌面、Win10/多屏及 release 另验。

## Out of Scope

不启动桌面/设备、安装或发布，不修改平台 control、pipeline/timeline 的拒绝规则、
双轨顺序控制协议或时钟/FPS/容量。此处证明故障路径，不宣称普通 WGC 时间戳会倒退。
W53 AVI 超时和 W59 原 Opus 失败原因继续未证明，不将这次独立复现作为其根因。
