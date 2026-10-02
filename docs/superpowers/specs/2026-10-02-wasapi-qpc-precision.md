# REC-WASAPI-QPC-PRECISION-01 — WASAPI 时间戳量化与连续 PCM

## Goal

基于 `a02c35fdb4f5f29aad8997e031dfe073801ed2a8`（生产源码 `56d750c`），修复
WASAPI QPC 的 100 ns 输出精度与 48 kHz PCM 时长计算不一致时，合法连续块被误判为
重叠并中止录音的问题。承接 `PX-REC-WINDOWS-AUDIO-01` 和 `REC-AUDIO-ACTIVATION-BOUNDARY-01`。
默认录屏入口保持关闭。

[Microsoft GetBuffer 合同](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)
规定 QPC 输出为 100 ns 单位，没有规定整数舍入方式或固定 packet 帧数。
原 API 合成诊断已验证：640 帧时长 13,333,333 ns，下一块起点 13,333,300 ns；
pipeline/worker 返回 PresentationOverlap，完整 VP9/Opus 会话保留 interrupted。
这证明代码路径，不能证明当前设备实际采用该 packet 节奏或舍入方式。

## Requirements

1. 音频源以内部类型声明精度；默认 Exact，Windows WASAPI 明确为 100 ns。
   平台包装必须传递该信息；混音器输出已按音频帧归一化，仍用 Exact，不继承输入精度。
   worker 在激活源之前配置 pipeline；精度不能在首块或控制操作之后改成另一种。
2. 对声明 100 ns 精度的源，packet 和媒体呈现区间可容忍最多一个 100 ns 刻度的末尾差。
   只将媒体起点对齐到上一块媒体末尾，不改原始 QPC 映射、captured_at_ns、sequence、PCM、
   会话原点或公共暂停区间；不丢帧、插帧、重采样或建立新时钟。真正空洞保持。
3. 允许上述对齐导致的暂停/封尾媒体末尾差，不重写源控制时刻或改变恢复下界。
   超过 100 ns、原始序号重复、时间倒退、早于起点/恢复下界、格式错误仍按原合同报错。
   背压失败原子性、溢出检查、原错误/已接受前缀与线程析构保持。
4. 原 Exact API 诊断夹具固定，仍拒绝量化重叠；新增精度声明后的绿色验收单独记录，
   不将新 API 测试声称为原 API 红绿证明。原完整测试模块和期限保持。
5. 用原 worker、完整双轨 owner、真实 VP9/Opus 和 journal 文件验证合成量化输入；
   Windows 默认/QA 原生完整门禁基于干净源码另验。

## Acceptance Criteria

- [x] 原 API 的量化重叠、worker 根错误/PCM 前缀、完整会话 interrupted 与混音帧网格诊断保存。
- [x] 精度声明、平台路由、混音 Exact 与 packet 边界合同通过。
- [x] PCM/原始时间戳保持；100 ns 边界、真实重叠/空洞、无效源、背压、控制与溢出保护通过。
- [x] 精度源经原 worker 与完整双轨 owner 输出 complete，实际文件独立核对有效 PCM 和时长。
- [x] 原完整模块/期限保持；干净源码 Windows 默认/QA 完整门禁通过。
- [ ] 同 SHA CI、其它宿主、release、真实设备/桌面、设备时钟漂移与长时同步另验。

## Out of Scope

桌面保持暂停；不启动应用/设备，不安装、推送、合入或发布。硬件时钟漂移、设备切换、
重采样、数据 discontinuity、混音器启动帧取整边界属于后续独立范围。
W53/W59/W63 历史失败根因继续未明。本次证据不替代真实 WGC/WASAPI、安装器或跨平台验收。

冻结源码 `aac5e0a728d46adc7bd7603188f41b9380138650`：完整Windows门禁33/0/1，默认1281/QA1425各5ignored，前端81/1403。
领域459/0与新17项已含总数；实际文件核对40ms/1920有效PCM，八个旧模块/期限保持。
审查见 [W67记录](../../reviews/2026-10-02-wasapi-qpc-precision-review.md)；最后复合验收保留未完成。

W68后续：同源码默认/QA release编译及PE/CRT文件验证完成；实际启动、设备/CI、其它宿主与桌面仍未验。
见 [release验证](../../reviews/2026-10-02-windows-qpc-precision-release-review.md)；最后复合验收保持未完成。
