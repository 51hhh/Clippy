# REC-AV-BRIDGE-JOIN-01 — A/V 桥接线程 panic 后完整回收

## Goal

修复共享 A/V 编码 owner 在视频桥接线程 panic 后跳过音频线程 join 的退出漏洞。
对应 WIN-NATIVE-01 / W06 的独立 W26，基于 99f83f8；在 Windows 原生默认/QA 图验证，
桌面操作保持停止。没有观察到真实录屏线程 panic，故障注入证明的是退出合同。

## Requirements

1. 视频和音频桥接线程都必须 join；第一项错误不得短路第二项。保持视频 → 音频顺序。
2. 任一桥接 panic 均返回既有 BridgePanicked；两者正常则保留 run_inner 的原结果。
   现有失败时 abort 两条 pipeline、先 drop 两个接收端的顺序保持，不能因 join 引入循环或新线程。
3. 用生产共用入口和真实 JoinHandle 验证正常、单侧和双侧 panic，音频线程被门控时
   owner 不得提前返回。受控线程必须释放门控并观察清理，失败测试也不遗留永久等待。
4. 保持录屏非默认 feature、媒体顺序、writer/journal 和平台 API；这是共享录屏修复，
   Windows 本机证据不替代 Linux/macOS 原生图或真实设备验收。
5. 需求、CHANGELOG、计划/报告同步；完整本机门禁绑定干净源码 SHA，保留远程 CI/桌面未验。
   现有四平台原型 CI 的 recording::av 前缀必须覆盖新模块，不增加重复测试入口。

## Acceptance Criteria

- [x] 原短路协议在真实受控线程的红基线暴露 owner 返回时音频还未释放。
- [x] 四项生产入口回归验证正常、视频/音频单侧和双侧 panic；门控释放后全部清理。
- [ ] Windows QA Cargo 图及完整默认/录屏 QA 门禁通过；现有原型 CI 前缀覆盖新合同。
- [ ] 规格、CHANGELOG、计划和报告同步，桌面、新 SHA CI 和其它宿主图保留未验。

## Out of Scope

更改捕获/编码线程的 panic 来源、吞掉错误、引入 join 超时或取消协议、强制终止线程、
调整媒体时间线、启用默认有声录屏、操作桌面/真实音频设备、Linux/WSL、安装新包、合入 dev 或发布。

## Review Evidence

run_inner 结束后已 abort 失败 pipeline 并丢弃接收端，但
video_bridge.join().is_err() || audio_bridge.join().is_err() 会在视频 panic 时短路，
audio JoinHandle 随栈帧退出被丢弃。Rust [JoinHandle 合同](https://doc.rust-lang.org/std/thread/struct.JoinHandle.html)
明确丢弃 handle 会 detach，join 则等待线程和线程局部析构完成；丢弃不能证明完整回收。
共享 av_encoder_worker 由 recording-opus-webm 编译，Windows 双轨 QA feature 包含此图；
默认 Rust 图不覆盖它。预算续审确认 Windows 初始化前已按整屏 64 MiB 校验，不为该路径新增缺陷。

## Verification

MSVC 无依赖 harness include 完整 av_bridge_join.rs；旧控制协议保留原 join().is_err() ||
join().is_err() 表达式，四项红基线 2 passed / 2 failed（exit 101），两项视频 panic 场景的
owner 在音频门控未释放时返回。修复后同样四项通过（exit 0），真实 JoinHandle 和资源 Drop
通知完成，无类型 stub、COM、WinRT、音频设备或桌面操作。500 ms 观察窗口内不准提前返回；
夹具等待均有 5 秒上限，断言失败前先释放音频并观察清理。这个等待上限只用于测试。
原始失败和绿日志、源码及二进制保留于 src-tauri/target/windows-av-bridge-join-red。
真实 Cargo QA 图与完整 Windows 门禁待验；现有四平台原型 CI 的 recording::av 前缀覆盖
av_bridge_join 与 av_encoder_worker，新 SHA CI 未执行。共享其它平台图/真实 panic 未验，
不得把受控线程故障注入称为真实设备 panic 或整段录屏验收。
