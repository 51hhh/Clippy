# WIN-WGC-INIT-ROLLBACK-01 — WGC frame pool 初始化失败回滚

## Goal

为 frame pool 创建后、WgcRuntime 建立前的两处初始化失败补齐显式 Close 尝试。
对应 WIN-NATIVE-01 / W06 的独立 W27，基于 3c19883；桌面操作保持停止。
只证明回滚尝试和所有权转移，不声称已观察到真实 COM/WinRT 泄漏。

## Requirements

1. frame pool 创建后立即由 RAII guard 拥有；FrameArrived 注册或 CreateCaptureSession 失败时，
   返回原初始化错误前尝试 pool.Close，随后释放该 pool 所有权。
2. rollback Close 失败仍保留原初始化错误，仅按既有风格记录关闭错误；不保证 OS 最终释放。
   成功时解除临时 guard 并把 pool/session 交给 WgcRuntime，正常 Close/Drop 仍由既有入口处理。
3. 原生初始化与离线故障注入使用同一函数；覆盖两处失败、关闭失败及成功转移，
   验证 Close 在资源 Drop 前，失败时不继续后续初始化，成功时不提前 Close。
4. 复用锁定 scopeguard 1.2.0，不加依赖或改 lock；保持光标、回调行为、同步通道、StartCapture、
   公开 API 及其它平台。新增/修改的 vendor LF 原始字节继续校验，篡改负例不能被弱化。
5. Windows 本机/CI 显式执行独立 vendor 回滚测试，完整默认/QA 门禁绑定干净源码 SHA；
   规格、补丁说明、CHANGELOG、计划/报告同 ID，真实 API 失败与新 SHA CI 保留未验。

## Acceptance Criteria

- [x] 原初始化错误短路在离线红基线遗漏 Close，资源先 Drop。
- [x] 四项生产入口合同覆盖注册失败、session 创建失败、回滚关闭失败及成功转移。
- [ ] 原始字节正/负校验、vendor check/clippy/test 和完整 Windows 默认/QA 门禁通过。
- [ ] 本机/CI 测试入口与文档接线，真实 WGC、其它平台和新 SHA CI 保留未验。

## Out of Scope

WinRT frame pool 创建自身失败前的系统内部资源、session API 部分创建的系统内部行为、
捕获回调/线程重构、真实资源泄漏证明、保证 Close 成功、安装新包、桌面或音频设备操作、
Linux/WSL、启用默认录屏、合入 dev 或发布。

## Review Evidence

此前 create_runtime 在 FrameArrived(...)? / CreateCaptureSession(...)? 失败时退出，
WgcRuntime 尚未构造，已有 WGC-CLOSE 合同无法处理这个分支。一次性 WGC 截图已经使用
scopeguard::guard，录屏初始化则没有。复用 [scopeguard 1.2.0](https://docs.rs/scopeguard/latest/scopeguard/)
的 guard 与 into_inner：错误返回时执行回滚，成功时转移所有权。依赖版本/源码均按 Cargo.lock 核对。

## Verification

MSVC 辅助 harness include 完整生产纯模块。原两处 ? 退出协议四项为 1 passed / 3 failed（exit 101），
失败事件仅到 pool-drop；绿色四项通过（exit 0），使用真实 scopeguard 1.2.0 rlib，未造类型 stub。
受控泛型 Pool 和回滚回调验证 Close 在 Drop 前、原错误保持及成功仅转移。依赖锁定来源、rlib
字节及红绿源码/日志/二进制保留于 src-tauri/target/windows-wgc-init-rollback-red。
生产字节验证器原样复制到隔离最小仓库，正例 exit 0；init/mod/recorder 分别追加一个 LF
均 exit 1，逐例恢复，生产文件未篡改。十个 pinned 文件不归一化；Node 24.21.0，证据在
src-tauri/target/windows-wgc-init-rollback-supply-chain/RESULT.json。
独立 vendor Windows Cargo 图与完整本机门禁待验。
离线资源和关闭回调不创建 WinRT 对象，不能替代真实 API 错误或设备验收。
