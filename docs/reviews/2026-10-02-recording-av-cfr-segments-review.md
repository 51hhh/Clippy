# WIN-NATIVE-01 / W53 — 双轨 CFR 分段与 WebM 量化顺序

需求 `REC-AV-CFR-SEGMENT-01`，基线 `3385507a096ab0fa9613d268daabc0b01e024521`，
源码 `80e084c`；独立分支 `codex/recording-av-cfr-segments`。规格
[`2026-10-02-recording-av-cfr-segments.md`](../superpowers/specs/2026-10-02-recording-av-cfr-segments.md)
在生产修改之前创建。只做代码和已有 Windows CLI；桌面、应用与设备继续停止。

## 问题与修改

原 writer 按原生采集时间分段，VP9 却向下映射到 CFR slot。例如 10 FPS 的 220 ms
输入编码在 200 ms，新分段起点为 220 ms，局部时间相减失败。PCM 若也按原生 head
拆分，就会先跨入旧段。五项实际 VP9/Opus/writer/编码 worker 回归均 `InvalidTimeline`。
现在分段与编码线程 PCM 拆分共享原编码 slot 边界，并在音频 sample 域比较拆分条件。
同 slot 后到帧不创建空分段，保留原输入 PTS、共享时钟、epoch 与 CFR 替换行为。

15 FPS 相邻全局 CFR 纳秒时间取整后的差可为 66,666,667 ns，旧 reader 误算应有两帧。
例外只接受实际全局 CFR 起点到相邻 slot 的精确差；主输出缺帧、非 CFR 起点、
不合法时长/时间戳、实际帧/包计数、PCM、关键帧和哈希校验保持。

首次修复的严格读取还发现另一真实边界：交错器按原纳秒排序，WebM 却以 0.5 ms
写入。保存的 15 FPS 最终输出与 30 FPS 恢复首段，都有 66.5 ms 音频后接同时间视频，
被原严格顺序检查拒绝。现在 packet 比较、另一轨 frontier 与交付 mux 的时间都使用
同一 timecode 量化域，同时间视频在前；入队仍检查原单轨时间严格递增，三十二包预算
不变。严格 packet reader 和 `AvWebmPacketMux` 原实现保护逐字节保持。

## 验证状态

初始八项旧实现 2/6；首次修复领域 334/3，其中两项为实际 reader 顺序失败，一项为新
worker 夹具四帧直接推入原三帧队列导致丢帧。夹具改为根据实际 queued_frames 等待消费，
五秒上限、原容量和四帧断言保持，再断言零丢帧；诊断八项为 6/2，保存了失败文件。
统一 timecode 后领域 339/0；补充全局起点/两 slot 端点的严格 reader 合同后冻结夹具。

最终同字节十二项在旧六个 Git blob 上真实重放 3 passed / 9 failed，native exit 101；
五项 `InvalidTimeline`、两个合法纳秒端点断言失败、两个量化 frontier 过早排空。
三个非法输入检查保持通过。finally 逐字节恢复生产源码；同夹具最终领域 341/0，
原 329 和新十二项在内，不累加。四份夹具哈希一致，六个原完整测试模块正文保持；
所有这些新增项只在非默认 codec QA 图。每段/全局真实文件通过原严格 reader，
首视频 packet 为零时间关键帧、PCM/帧数/时长/journal 核对和伪造计数拒绝通过。

干净源码 `66ceffdfea061d7fde9085a7a2f6ba25d1c30d29` 完整 Windows 默认/录屏 QA 门禁
child/terminal exit 0，33 passed / 0 failed / 1 Linux smoke skipped，结束 checkout 干净。默认
Rust 1214/QA 1307 各 5 ignored，两图不累加；前端 81 文件/1403 passed。领域 341 和新十二项
包含在 QA 内，不重复累加。`COMMITTED-CONTRACT-AUDIT.json` 将十份 Rust 输入绑定干净 SHA，
冻结 SHA 完整门禁另运行；后继仅五份 Markdown，生产/测试源码保持。
`recording-av-cfr-segment-native-qa-66ceffd-retry1/RESULT.json` 绑定实际原 stdout/stderr 哈希；根当前
状态/报告与 `recording-av-cfr-segment-contract/REVIEW-CLOSURE.json` 保留原 W52/旧桌面身份。
证据在 `src-tauri/target/recording-av-cfr-segment-contract/`：原 W52 状态/报告，初始红、
阶段失败/绿、失败 WebM 与 `QUANTIZED-PACKET-DIAGNOSIS.json`、最终 frozen-red、恢复记录、
final-green 和 `CONTRACT-AUDIT.json`。真实 codec/writer/worker 合成测试不代表设备测量，
文件/Git 核对不计测试通过。

首次完整门禁 `recording-av-cfr-segment-native-qa-66ceffd/RESULT.json` 为 32 passed / 1 failed /
1 skipped，child/terminal exit 1。QA Rust 1306/1/5 ignored，默认与其它检查通过；失败为未修改
`periodic_segment_is_committed_before_stop_and_preserved_in_final_manifest` 等待 AVI 清单提交
30 秒超时。该测试单独 0.23 秒通过；保持原测试与期限、同一干净 SHA 完整重跑通过。
首次失败日志和 `GATE-RETRY-REASON.json` 保留，原因未定位，不能称为稳定性问题已解决。

## 未完成项

下一视频 head 尚未到来时的音频消费等待仍需独立审查；不能称为任意静态桌面长期录屏已验。
当前 SHA 三宿主/codec CI、其它宿主门禁、真实设备/长时同步/Stop/恢复、安装器/无 CRT 启动、
Windows 10/多屏/升级/Wayland 保持未验。本 SHA 未构建 release/安装器，历史 `1c66112`
QA EXE 不代表当前产物。panic 只覆盖 unwind；release panic=abort 不承诺回收。
旧桌面 2 pass / 1 fail / 36 not_run 保持，整体目标未完成。
