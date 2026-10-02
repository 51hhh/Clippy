# REC-OPUS-EBML-VERIFY-01 — Opus 元数据测试按元素边界读取

## Goal

让现有双轨 WebM 合同真正核对容器字段，避免把 TrackUID、Void 或编码载荷中的
相同字节当作元数据。基线为 `678a817abe073fc1b27f33eeb38657bf1f04c9f1`。
本项只修复测试读取器，不改变产品封装、恢复读取器、设备或默认功能门控。

## Requirements

1. 保留旧双轨测试的字段值、音视频包、ffprobe 检查和所有断言，不放宽期限或重试。
2. CodecDelay、SeekPreRoll、TimecodeScale、DiscardPadding 只从受支持的父路径、
   EBML 元素 ID/长度/边界读取；非 Master 元素的载荷不能作为子元素搜索。
3. 超过父边界、截断/无效 VINT、未知大小的标量必须拒绝；未知大小只接受文件顶层
   Segment 这一原 writer/严格恢复读取器支持的形状。有符号值保留符号扩展。
4. 对真实 VP9/Opus mux 输出只等长替换非零视频 TrackUID，保存干扰文件、字段偏移、
   哈希和新旧读取结果；原严格恢复读取器必须完整读取，音视频包与原文件相同。
5. 缺失字段不能由载荷里的伪字段补足。正常整数、正负 padding 与支持的 Segment
   长度继续通过。所有生产代码、原测试函数/断言和 codec 配置保持字节一致。

## Acceptance Criteria

- [x] 原测试读取器在相同的新夹具中稳定失败，保存真实媒体与日志。
- [x] 按父路径/长度读取后相同夹具通过，包含缺失、截断、未知大小和符号保护。
- [x] 真 mux 干扰样本经原严格读取器完整遍历，包/元数据匹配；本机 ffprobe 另验。
- [x] 干净源码 SHA 的 Windows 默认/录屏 QA 完整门禁通过，测试重叠和跳过分层记录。
- [ ] 当前 SHA 三宿主 CI、真实设备/桌面、Win10/多屏与发行验收另行记录。

## Out of Scope

不操作桌面、设备或安装器，不修改生产解析器/封装器，不增加默认音频能力。
W59 历史 SeekPreRoll=None 的失败文件未保留：独立可复现的测试缺陷不证明该次
失败的具体根因。W53 历史 AVI 超时同样继续保留；不得以新测试成功宣称已归因。

## References

- [RFC 8794](https://www.rfc-editor.org/rfc/rfc8794.html)：元素长度、Master/Binary 与未知大小。
- [Matroska 元素表](https://www.matroska.org/technical/elements.html)：四个字段及其父路径。
- `src-tauri/src/recording/mux/webm_remux.rs`：现有严格读取器与文件长度/SHA-256 验证。
