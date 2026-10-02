# REC-FIRST-FRAME-AUDIO-01 — 首视频帧就绪后启动 Windows 音频

## Goal

避免视频 factory 已就绪但尚无有效首帧时，双轨会话因一秒 PCM 队列背压而提前失败。
本项继承 `REC-AV-STARTUP-GATE-01` 的双 factory 握手，单独覆盖首帧等待阶段。

## Requirements

1. 先用实际 `AvRecordingSession`、采集 worker、pipeline 和编码线程复现首帧等待期间的
   音频背压；测试不启动 WGC/WASAPI，也不把合成结果称为真实设备故障。
2. 音频轮询只能在有效首视频帧已被 pipeline 接受后释放；无首帧、视频错误、无效帧
   与 Drop/Stop 均须取消等待并回收线程，不放大 PCM/视频/桥接预算，不改时间线。
3. 等待期间暂停/恢复返回明确的准备中错误，不进入同步控制握手形成死锁；首帧到达后
   复用原控制、编码、停止与 journal 合同。首轮视频轮询与 owner 握手，立即就绪的帧保留
   `start` 返回后立即暂停的原合同；首轮 None 不阻塞直到未来首帧。
4. Windows WASAPI constructor 只准备原生对象，启动释放后由同一音频线程激活 stream。
   平台封装和混音源转发激活；任一激活失败中止 pipeline，原错误保留，双源部分启动回收。
   可延迟源返回后续 PCM 的有效时间下界；双源都给出下界时从较早下界启动混音，避免
   从会话零点突发生成准备阶段静音，仍保留两源启动差与原生 PTS。
5. 默认单轨 worker 继续在 factory 完成后立即运行；新增释放规则仅用于双轨。原回归保留，
   代码注释中文；原生 PTS/共享时钟与静音补洞合同不替换成到达时间。

## Acceptance Criteria

- [x] 同一受控 AV 测试在旧实现出现首帧前采集/背压，修复后无提前采集并正常提交双轨输出。
- [x] 首帧前错误、无效帧、Drop/Stop、准备中控制与首帧后既有控制有对应线程回收证据。
- [x] 音频激活与平台/混音转发、部分失败有离线合同；Windows 原生编译检查实际 API 接入。
- [ ] 干净 source SHA 的完整 Windows 门禁通过；跳过项、ignore 与重叠测试分开记录。
- [ ] 当前 SHA 三宿主/codec CI、真实设备首帧/首包与两种音源、安装器/多屏验收通过。

## Out of Scope

本轮不启动应用或设备、不恢复桌面控制、不安装工具或改变证书、不推送/建 PR/合入/发布。
本机是 Windows 11 单屏；其它宿主的原生 source 激活迁移、真实设备的响应速度/音画同步
和首帧超时不会因受控测试变成已验证。构建/文件核对不计为测试通过。

## Evidence plan

基线 `d9264cd85acd88123620236c8292a280de96c6f2`，源码 `1c66112`；此前 factory 初始化修复
不覆盖本项。证据保存于 `src-tauri/target/recording-first-frame-audio-contract/`；先红后绿
固定同一 AV 夹具，补充激活/取消合同单列。最终门禁在冻结提交上运行，桌面历史记录保留。
