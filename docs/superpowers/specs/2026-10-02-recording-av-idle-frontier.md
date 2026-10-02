# REC-WINDOWS-IDLE-AV-01 — 空闲视频下界驱动双轨排空

## Goal

Windows WGC 首帧后没有新画面时，音频继续在原一秒 PCM 预算内消费，CFR 重复视频与周期
恢复分段继续提交，后来的真实帧仍保留原时间戳与可替换 slot。基线 `e59430f` / 源码
`66ceffd`；独立分支 `codex/recording-av-idle-frontier`。只做代码和已有 Windows CLI。

## Requirements

1. 用真实采集/编码 worker、VP9/Opus/writer 与合成源复现首帧后 PCM 背压。原 merge 行为
   先红后绿，保留旧测试正文；新增源接口在旧实现重放时只提供未被调用的默认声明。
2. WGC 桥接线程在接收超时之后按同一时钟发布未来帧时间下界；串行生产顺序证明后续
   stamped frame 不早于下界。有尚未交付的旧帧时下界不得越过它。原帧时间不重写。
3. source 默认不提供下界；没有合同的 source 保留原等待。共享 pipeline 只合并一个
   下界标量，先排已接受真实帧，再转交下界/终态；早于声明下界的真实帧必须拒绝。
   下界不建立首视频 epoch、不推进原生 last presentation、不计 captured/input 帧。
4. 编码 owner 只生成未来真实帧 CFR slot 之前已经确定的重复帧。PCM 在 sample 域拆分，
   不能越过仍可被真实帧替换的 slot；空闲重复帧和周期分段共用 CFR 边界，保留静音、
   pre-skip、padding、关键帧、严格 packet 顺序、哈希、统计与 journal 核对。
5. 1 FPS 空闲下界轮询保持及时；真实帧采集上限不提高。暂停不采集/发布进展，恢复清除
   原生缓存并沿用原时间线，Stop/Error/Drop 回收所有自有线程。三帧、三十二包与一秒
   PCM 预算不增加；不根据任意音频时间推断尚未收到的真实视频时间。

## Acceptance Criteria

- [ ] 固定首帧后空闲的实际 AV worker 回归在旧 merge 失败、修复后通过。
- [x] 长空闲/1 FPS、后续真实帧、源下界/缓存顺序、无下界等待、暂停/恢复/Stop/Error/Drop 有合同。
- [x] 真实恢复分段和最终文件通过严格 reader；周期提交、零起点关键帧与帧/PCM/时长统计一致。
- [ ] 干净 source SHA 完整 Windows 默认/录屏 QA 门禁通过；重叠、ignore、跳过与失败重跑单列。
- [ ] 同 SHA 三宿主/codec CI、其它宿主原生门禁与实际 Windows 设备/安装器/多屏验收通过。

## Out of Scope

桌面继续停止；不启动应用/设备，不安装工具/改证书，不推送、建 PR、合入或发布。
Linux/macOS 原生源尚不声明空闲下界，不能因共享代码通过称为其它宿主静态录屏已验。
W53 未修改 AVI 周期提交测试的一次 30 秒超时原因仍未定位；原记录保留。

## Evidence plan

`src-tauri/target/recording-av-idle-frontier-contract/` 保存原 W53 状态/报告、原 merge 红、
最终同夹具旧 merge 重放、绿色领域、源输入与旧测试正文核对、冻结 SHA 完整门禁。
源下界 API 的测试只在新实现可执行，单列于旧 merge 运行期红绿；合成 native codec
测试不代表实际 WGC/WASAPI 设备测量，文件/构建/矩阵不计测试通过。
