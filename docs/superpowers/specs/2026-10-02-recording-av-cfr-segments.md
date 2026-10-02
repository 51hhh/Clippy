# REC-AV-CFR-SEGMENT-01 — 双轨分段使用 CFR 边界

## Goal

真实采集时间不恰好落在帧率 slot 时，双轨周期分段仍可提交、独立读取和恢复；全局
连续输出和恢复分段的音视频时间线一致。基线 `3385507`，源码 `80e084c`，独立分支
`codex/recording-av-cfr-segments`。桌面保持停止，只做代码与已有 Windows CLI。

## Requirements

1. 用实际 VP9/Opus、writer 和编码 worker 复现非 slot 对齐输入的失败；固定同一夹具
   先红后绿，不启动 WGC/WASAPI，也不把合成结果称为实际设备测量。
2. 视频原生 PTS、共享时钟、epoch 与原 CFR 映射保持；周期分段和 PCM 拆分必须共享
   实际编码 slot 边界。原生帧只在同一 slot 时不创建空分段，等编码 slot 有进展才分段。
3. 各恢复分段首视频 packet 为零时间关键帧；全局视频帧数、PCM 总数/每段计数、
   连续时长和 journal complete/interrupted 保持严格一致，预算与 packet 顺序不扩大。
4. 非整数纳秒帧周期可能让局部分段时长落在相邻一纳秒端点；严格 reader 只允许这个
   精确量化边界，并核对全局 CFR 起点和相邻 slot 终点；不能放宽主输出缺帧、非 CFR 起点、
   错误 PCM/关键帧/packet 顺序或实际时间戳校验。
5. 保留原测试正文；新增合同覆盖 10/15/30 FPS 非对齐输入、1 FPS 同 slot 分段延后、
   实际编码 worker 的 PCM 跨界，以及 reader 对合法一纳秒与伪造边界的区分。
6. 实际 WebM 在 0.5 ms 量化后仍按视频先于同时间音频排序；交错器的 packet 比较、
   未来 frontier 与交付 mux 使用同一量化时间。原生 PTS/单轨入队严格性和三十二包预算保持。
   保存的 15/30 FPS 文件均复现 66.5 ms 音频后接同时间视频，严格 reader 继续拒绝该顺序。

## Acceptance Criteria

- [x] 固定非对齐 writer/编码 worker 回归在旧实现失败、修复后通过，原测试保持。
- [x] 真实生成的恢复分段通过严格 packet reader；总帧数/PCM/时长和首关键帧为零通过。
- [x] 精确一纳秒量化例外有正/负合同；全局起点/终点核对，错误时长/帧数继续被拒绝。
- [ ] 干净 source SHA 完整 Windows 默认/录屏 QA 门禁通过，重叠/ignore/跳过单列。
- [ ] 当前 SHA 三宿主/codec CI、设备录屏与恢复/同步、安装器/多屏验收通过。

## Out of Scope

不控制桌面、不启动应用/设备、不安装工具、不改证书、不推送/建 PR/合入/发布。
下一视频 head 缺失时的消费等待仍须独立审查；当前不声称静态桌面任意长时录屏已验证。
其它宿主门禁、真实设备、Win10/多屏/安装器仍未验；release panic=abort 不承诺回收。

## Evidence plan

`src-tauri/target/recording-av-cfr-segment-contract/` 保存原 W52 状态/报告、原实现红、
最终同字节红绿、源码恢复摘要与原测试正文对比。真实生成文件通过现有严格 WebM reader，
冻结提交的完整 Windows 门禁另证；文件/Git 对比不计测试通过。

## 当前证据

最终四个夹具文件在旧六份 Git 生产源码重放 3 passed / 9 failed，native exit 101，finally
恢复修复源码；同文件最终全录屏领域 341 passed / 0 failed，原 329 和新十二项均包含在内。
六个原完整测试模块正文保持。两份失败 WebM 的实际 timecode scale 为 0.5 ms，均复现
66.5 ms 音频后接同时间视频，文件与解析证据保留。新 worker 夹具只等待原容量队列被消费，
断言无 backpressure 丢帧，不增大三帧容量。完整 Windows source SHA 门禁待运行。
