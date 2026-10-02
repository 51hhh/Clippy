# REC-VIDEO-CONTROL-PREFLIGHT-01 — 视频控制状态预检

## Goal

首帧尚未到达、已经暂停或仍在录制时，拒绝不适用的视频控制请求，并保持平台源与
pipeline 状态一致。Windows WGC 的暂停会实际停止流，恢复会丢弃旧缓存；失败请求
不能先改变原生状态。基线为 `827c7028999d2e984800c8bfe98745dde8c92d87`。

## Requirements

1. 首帧前 Pause 和未暂停时 Resume 使用原有 timeline 错误返回，不调用平台控制。
2. 重复 Pause 使用原有 AlreadyPaused 错误返回，不重复调用平台源。
3. 拒绝这些请求后 worker 继续可用，首帧、正常暂停/恢复/停止和文件提交保持原合同。
4. 原生 source 控制失败仍中止 worker，并保留原始 source 错误与线程内析构。
5. 不修改帧率、采集预算、队列、时间戳规则、旧测试/期限、音频流程或默认功能门控。

## Acceptance Criteria

- [ ] 旧 API 的首帧前与重复/错误状态控制合同在原实现失败，并保存真实 worker 调用证据。
- [ ] 相同夹具在修复后通过，包含正常控制和 source 错误保护。
- [ ] 无音频 MJPEG 与 QA VP9 会话在拒绝早期暂停后可接收首帧并提交有效产物。
- [ ] 干净源码 SHA 的 Windows 默认/录屏 QA 原生门禁通过；跳过项保留。
- [ ] 当前 SHA 三宿主 CI、真实设备/桌面、Win10/多屏与发行验收另行记录。

## Out of Scope

不操作桌面或启动 WGC/WASAPI 设备。测试使用合成帧和遵守暂停/恢复合同的源，运行
原 worker、pipeline、会话、真实文件与 codec；不宣称实际 Windows 原生控制成功。
W53 历史 AVI 超时根因仍未知，本项独立启动时序问题不作为其根因。
