# WIN-NATIVE-01 / W50 — 双轨 factory 就绪后的采集释放

需求 `REC-AV-STARTUP-GATE-01`；基线 `cc5f7af`，独立分支 `codex/recording-av-startup-gate`。
被测源码 `1c661123c875e0a19d9df9e706fbe79d90b3ff54`；规格见
[`2026-10-02-recording-av-startup-gate.md`](../superpowers/specs/2026-10-02-recording-av-startup-gate.md)。
本轮仅代码审查、已有 Windows 原生 CLI 与文件检查；桌面保持停止。

## 问题与修复

原 `AvRecordingSession` 初始化并运行音频 worker 后才初始化视频 source。原 encoder
等待首视频帧，PCM pipeline 只有一秒容量。受控视频 factory 尚在等待时，原音频 source
已轮询五次；即使该 factory 随后失败或 panic，音频也已进入采集循环。
本轮复现的是提前轮询，容量耗尽是从源码预算/顺序推导的风险，未在设备上复现溢出。

保留在各自线程创建原生 source 的就绪握手，新增 owner 的独立释放通道；两个 factory
均成功后才启动两条采集循环。等待期间关闭通道或 worker Drop/Stop 会取消，source 留在
创建线程回收。Stop 区分未释放等待与正常采集，避免 join 等待无人释放的通道。
单轨入口仍立即运行；原有 PCM 上限、共享时钟、暂停/停止时间线与平台 constructor 未改。

## 分层验证

- 原生产逻辑同组六项：**3 passed / 3 failed**。原三个 AV 合同通过，新三个成功/错误/
  unwind panic 回归均观察到视频 factory 等待期间音频已轮询五次；新三项保存源码与
  修复后字节相同。修改生产释放后同六项全部通过。
- 补充两个实际 worker 各四项：owner 释放、通道断开、sender 仍存活时 Drop 与 Stop。
  验证等待期间零轮询/零入队、取消后 pipeline 中止、非 Send source 同线程销毁，以及
  释放后正常采集/停止。八项全部通过；录屏领域 **301 passed**，新增十一项在内。
- 原 `worker`/`audio_worker`/`av_session` 三个测试模块仅增加 include，原二十七项测试
  正文与 fixture 保持，相关原用例全部通过。对比数不增加测试通过数。
- 干净 `1c66112` 完整 `./scripts/ci-windows.ps1 -RecordingQa`，child/terminal exit 0：
  **33 passed / 0 failed / 1 Linux smoke skipped**。Rust 默认 **1201**、QA **1267**，各
  5 ignored，两图不累加；worker 八项在两图中，AV 三项只在 QA，领域 301 不额外累加。
  前端 **81 文件/1403 passed**，原八十一份前端测试 Git blob 未改。
- 平台 source、音视频预算/时钟/epoch、encoder、manager/lifecycle、vendor、锁文件、
  基础配置、scripts/workflow 和前端产品保持。编译前后源码干净，后续仅五份 Markdown。
- 隔离显式 `x86_64-pc-windows-msvc` unsigned/unbundled QA release 约 3m47s，
  **native/wrapper/terminal exit 0**。真实主程序 rustc 命令保留 QA 五项 feature 与
  opt-level=s / panic=abort / fat LTO / 单 codegen unit / strip=symbols。
- 实际 QA EXE **31,690,240 bytes**，SHA-256
  `a3317e0d2f4eb97024574faeb926abc00be929c0b08494266393e1b9e157f68b`。
  同目录十份真实 Microsoft 签名 VC143 CRT 均为 14.44.35211.0，toolset 14.44.35207；
  原/暂存/部署哈希、清单摘要和 direct/delay 递归闭包通过。十七份基础许可证哈希一致，
  默认 release EXE 保持 `bde08375a1950f4925e2771f544476947e958a832b2800934e007e85b3837410`。

构建、DLL、资源或 Git 对比数量均不增加测试通过数。目标回归使用提交前 working source，
提交绑定与完整新 SHA 门禁另行记录；不能把 working-source 回归冒充冻结 SHA 的完整门禁。
测试 panic 走 unwind，实际 release 的 panic=abort 不承诺捕获或 Drop 回收。
CLI 测试使用受控 source；未运行产品 QA EXE、安装器或真实 WGC/WASAPI 设备录制。

## 本机证据

根目录 `C:\win\Clippy\src-tauri\target\` 下：

- `recording-av-startup-gate-contract/`：原红、同六项绿、领域 301 报告与真实退出；保存
  新三项源码、原测试正文对比、`STARTUP-CONTRACT-AUDIT.json`、提交绑定
  `COMMITTED-STARTUP-CONTRACT-AUDIT.json`，以及原 W49 状态/报告。
- `recording-av-startup-gate-native-qa-1c66112/RESULT.json`：完整干净 SHA 门禁与日志摘要。
- `recording-av-startup-gate-release-build-1c66112/`：实际 QA EXE、编译日志、CRT 来源/配置、
  `PAYLOAD-VERIFICATION.json`、`MAIN-RUSTC-COMMAND.json`、`SOURCE-AND-PAYLOAD-AUDIT.json`。

## 未验证与下一步

本轮只闭合 factory 尚未完成的提前轮询；WGC 在 factory 返回后仍允许五秒首帧等待，
而 PCM 预算仍为一秒。首视频帧暖机与音频入队需要独立受控回归，不能据本轮断言任意
延迟启动都可成功。原生缓存/首包、阻塞中的 factory 中断、真实设备/权限/长时漂移未验。

当前 SHA 远程 CI、其它宿主完整门禁、同 SHA 三平台/四编码器 CI、NSIS/MSI 与无 CRT
系统启动、Windows 10、多屏/混合 DPI、升级及 Wayland 回归仍未验。未推送/合入/发布，
未修改系统工具或信任；根 checkout 保持，W49/W48/W47 证据保留各自历史 SHA。
保存旧 `45769c9` 桌面 QA 仍为 2 pass / 1 fail / 36 not_run，记录不刷新；整体任务未完成。
