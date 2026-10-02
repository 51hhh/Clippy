# WIN-NATIVE-01 / W52 — 双轨空洞与停止尾段的有界排空

需求 `REC-AV-GAP-DRAIN-01`，基线 `e4a4a3c37a890aa6f9d4686b780263996e36cf94`，
源码 `2dccc43`；独立分支 `codex/recording-av-gap-drain`。规格
[`2026-10-02-recording-av-gap-drain.md`](../superpowers/specs/2026-10-02-recording-av-gap-drain.md)
在生产修改之前创建。只做代码/已有 Windows CLI；应用、设备和桌面继续停止。

## 问题与修改

原 writer 在 Stop 与 rotate 先一次性补齐全部音频，再补齐视频；长视频空洞也会先
一次性编码所有重复视频帧。编码 worker 收到视频 EOS 后仍直接交付整段音频。三条路径
都可能在另一轨尚未推进前填满三十二包 interleaver。实际原 codec/writer/worker
四项受控回归全部失败；补充两个有效 1 FPS 夹具后，最终同字节旧实现 0 passed / 6 failed，
六个原始错误都包含 `InterleaveQueueFull`。没有启动 WGC/WASAPI，不能称为设备故障测量。

编码 worker 在拆分 PCM 时转交已知下一视频 head 时间戳，视频 EOS 后声明不再有真实帧；
原“缺少下一 head 时等待”分支没有修改。writer 按原 100 ms PCM 块交替推进视频与音频，
大视频空洞也先增量补齐音频。VP9 只编码下一真实帧下界之前已经确定的 CFR slot，
保留下一 slot 图像和原生 last presentation；同 slot 后到帧仍可替换。低 FPS 采用足够
推进音频的 slot 上界，并受已知未来视频下界约束。最后的占位图像不增加停止帧数，
已排空至分段边界时交给边界真实帧，避免多编码一帧。原 packet 顺序、队列上限、
PTS/epoch、音频 padding、帧/包/PCM/时长与 journal 核对保持。

## 验证状态

目标领域 329 passed / 0 failed，原 319 在内；新增十项在内，不累加。
第一次四项修复领域 323/0，补充 CFR 和 1 FPS 后 329/0。六项旧源码重放在实际
三个 Git blob 上完成，finally 逐字节恢复当前文件；四个夹具文件哈希绑定红绿，
其中四项新 API 合同只在绿色执行。三个原完整测试模块只加 include，旧正文保留。
完整 Windows 门禁尚待冻结 SHA；目标领域使用 working source，不能代替干净门禁。

证据 `src-tauri/target/recording-av-gap-drain-contract/`：原 W51 状态/报告，原四项红，
阶段绿、六项 frozen-red 与 `FROZEN-RED-RESTORE.json`；最终绿和 `CONTRACT-AUDIT.json`
单列源码输入、原测试正文、预算/mux/时钟/平台未修改范围。没有 release/安装器构建，
历史 `1c66112` QA EXE 不代表当前产物。构建与文件对比不计测试通过。

## 未完成项

下一视频 head 尚未到来时的编码消费等待仍需独立回归和修复，不能因 writer 的增量排空
称为任意静态桌面长期录屏已验。不对齐 CFR slot 的分段时间映射仍需独立受控回归。
当前 SHA 三宿主/codec CI、其它宿主门禁、真实设备/长时同步/Stop、安装器/无 CRT 启动、
Windows 10/多屏/升级/Wayland 仍未验。panic 只覆盖 unwind；release panic=abort 不承诺回收。
历史旧桌面 2 pass / 1 fail / 36 not_run 保持，整体目标未完成。
