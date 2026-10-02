# WIN-NATIVE-01 / W51 — 首视频帧就绪后的 Windows 音频启动

需求 `REC-FIRST-FRAME-AUDIO-01`，基线 `d9264cd`，分支
`codex/recording-first-frame-audio-gate`；规格见
[`2026-10-02-recording-first-frame-audio.md`](../superpowers/specs/2026-10-02-recording-first-frame-audio.md)。
仅代码、已有 Windows CLI 与保存文件证据；桌面继续停止。

## 问题与实现

W50 修复 factory 初始化期间的提前音频轮询，但初始化成功后 WGC 仍允许五秒首帧等待，
encoder 在首视频帧前保留一个音频 head；两个容量一 bridge 以及一秒 PCM pipeline
使持续音频迅速背压。受控 AV source 在首帧前轮询十四次，读取 actual audio worker
确认 `Pipeline(Backpressure)`；没有启动设备，不能推为实际 WGC/WASAPI 测量。

视频 worker 在有效首帧通过 pipeline 校验并入队后才释放音频；首轮 None 与 owner 握手，
允许保持准备中，同时立即有效首帧保持原立即暂停合同。准备中暂停/恢复返回错误，避免
暂停视频后在音频释放等待中死锁；视频错误/无效帧/Drop/Stop 关闭信号并回收两条线程。
Windows WASAPI constructor 留在原线程准备，音频释放后才激活 stream。激活返回后续
PCM 时间下界，经平台与混音封装转发；两个下界均有效时，从较早下界初始化混音 cursor。
双源第二条激活失败停止第一条，保留原错误或同时报告回滚失败。其它平台默认 hook 沿用
原 constructor 激活，未把这一部分声明为其它原生平台已完成迁移。

## 验证状态

目标合同已通过，尚未声明完整门禁或冻结 source SHA 通过。

原三项新 AV 回归两次均为 0/3；初始诊断补充直接读取 audio worker 原错误，确认背压。
第一版修复领域 311 passed / 3 failed，其中原立即暂停合同失败，另两项在正常输出
阶段失败。首轮握手修复原立即暂停；第二轮、第三轮均 317/2。第三轮直接返回
`Encoder(Writer(Opus(InterleaveQueueFull)))`，临时诊断仅运行该实际 AV 用例得到 0/1，
audio 队列三十二包时间戳为 1.3065s–1.9265s，video 队列为空。诊断完成后原 interleaver
字节恢复，没有扩大三十二包、一秒 PCM 或视频预算；这些失败不计为通过。

最终首帧夹具保留 100 FPS，交付二十个有效视频帧及二十块 100 ms PCM，明确等两轨
全部交付后才 Stop，避免用启动测试覆盖另一个提前停止的尾部排空问题。领域
**319 passed / 0 failed**，原三百零一项实际全部通过，新增十八项在内；四个原测试
模块只添 include，原完整测试正文逐 LF 归一化字节一致。最终三个 AV 回归在原生产
源码重放为 **0 passed / 3 failed**，音频背压十四次再次出现，三个夹具与绿色文件
逐原始字节一致。原三项旧 AV/原立即暂停和 W50 初始化握手十一项保持。

新增十八项为 AV 首帧三项、准备中控制/Stop 两项、视频释放/None/无效帧/错误四项、
音频释放激活/取消/失败/单轨四项、混音激活/部分失败/回滚/下界五项。均运行实际
worker/会话或纯合同；Windows 原生编译解析真实 WASAPI/平台转发路径，不启用设备。
单轨保留立即运行，原其它宿主 constructor 通过默认 None hook 保持；不宣称原生
缓存迁移已在其它平台验证。Windows 激活使用初始化后的 Start，取消继续复用停止
状态的 Reset，合同参见 [Microsoft Start](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-start)
和 [Reset](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-reset)。

根 `src-tauri/target/recording-first-frame-audio-contract/` 保存初始/诊断红、第一次绿的
失败、冻结红、源文件恢复摘要及原 W50 状态/报告；源文件仅在受控红重放期间换为
原 Git blob，命令 finally 完整恢复六个修复文件。`final-contract/` 保存最终原实现红；
`CONTRACT-AUDIT.json` 绑定最终同字节夹具、原领域/四模块和未修改 scope。目标合同
使用 working source，不能冒充干净 SHA 门禁；冻结 SHA 门禁另证。

## 未完成项

首帧后长期没有新视频帧而音频仍连续到来的消费阻塞，以及 Stop 先于后续视频帧交付时
尾部 Opus 包等待视频编码而达到三十二包上限，须独立回归和调度/收尾修复。上面的
正常交付屏障没有解决或删除这些失败证据；不能据 319 绿色声称任意首帧后停顿可成功。
阻塞中的 factory/API、实际首帧/首包/混合音源、长时同步、当前 SHA 三宿主及 codec CI、
其它宿主门禁、安装器/无 CRT 系统启动、Windows 10/多屏/升级/Wayland 仍未验。
测试 panic 使用 unwind；release panic=abort 不承诺回收。构建不计测试通过。
历史旧桌面记录继续保留 2 pass / 1 fail / 36 not_run，整体目标未完成。
