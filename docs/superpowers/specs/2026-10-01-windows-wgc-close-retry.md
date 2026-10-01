# WIN-WGC-CLOSE-01 — Windows WGC 部分关闭失败后的清理与重试

## Goal

修复 WgcRuntime 在 session.Close 失败后遗漏 frame_pool.Close，且提前 closed 标记使 Drop
放弃清理的问题。对应 WIN-NATIVE-01 / W06 的独立 W25，基于 3308677；桌面操作保持停止。
不声称已观察到真实 WGC 资源泄漏或关闭 API 失败。

## Requirements

1. session 与 frame pool 分别记录成功关闭；每轮对尚未成功的两个资源都尝试 Close，
   session 错误不得阻止 pool 尝试。成功资源不重复关闭，失败资源允许后续显式调用/Drop 重试。
2. 每轮保持 session → pool 顺序；session 失败优先返回原错误，两者均成功才返回 Ok。
   persistent failure 不得伪装成功；Drop 单次 best-effort 不引入循环或新增线程。
3. 生产 WinRT Close 与离线故障注入使用同一状态入口，验证单边/双边失败、部分成功、
   重试、幂等与 Drop 清理；测试不得创建 frame pool、session、显示器或调用桌面。
4. xcap 0.9.6、来源/许可证、光标参数及其它平台行为保持；登记修改及新模块的 LF 原始字节
   SHA-256，保留供应链校验。加入真实 Windows vendor lib 测试，不用主应用测试代替依赖库合同。
5. Windows 本机门禁与 Native CI 显式执行关闭合同；规格、补丁说明、CHANGELOG 和总计划
   引用同一需求 ID，干净源码完整默认/录屏 QA 门禁与新 SHA CI/桌面证据分开。

## Acceptance Criteria

- [x] 原提前 closed / 短路行为离线红基线暴露 pool 未尝试和错误被后续关闭吞掉。
- [x] 修复后六项合同验证部分失败、重试、错误优先级、成功幂等与 Drop 清理。
- [ ] vendor 原始字节正校验与篡改负例通过；Windows lib check/clippy/test 与完整门禁通过。
- [ ] 规格、补丁来源、CHANGELOG、计划和报告同步，桌面/新 SHA CI 保留未验。

## Out of Scope

启用录屏默认 feature、改造 WGC 回调/通道/线程、改动其它平台、真实设备/显示器测试、
保证失败 Close 的系统资源已经释放、部分 runtime 构造前的原生失败、安装新包、Linux/WSL、
合入 dev 或发布。

## Review Evidence

旧 close 在 self.closed = true 后执行 self.session.Close()?；session 失败就跳过 pool。
pool 失败或 session 失败后，Drop 再调用 close 都返回 Ok，不再尝试失败资源。
微软分别定义 [session.Close](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.close)
和 [frame_pool.Close](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.direct3d11captureframepool.close)
释放各自资源；此处仅证明 Clippy 的关闭尝试和错误传播，不证明 OS API 必然成功。

## Verification

MSVC 辅助 harness include 完整 wgc_runtime.rs，没有类型 stub 或外部依赖；原全局 closed
通过两个始终同时置位的标志表示，提取的原短路控制协议六项为 1 passed / 5 failed（exit 101）。
修复后六项通过（exit 0）：失败资源允许重试，成功资源幂等，两侧持续失败保持 Err，session
失败不阻止 pool。Drop fixture 使用同一 close 状态入口；不执行实际 WgcRuntime/WinRT Close。
证据在主检出的 src-tauri/target/windows-wgc-close-red，原始失败日志保留。

生产供应链验证器原样复制到最小仓库夹具，正例 exit 0；分别给 runtime、mod、recorder 追加
一个 LF 后均 exit 1，实际字节 drift 被拒绝，每例恢复后再进行下一例。生产文件未篡改，不归一化
哈希输入；证据 src-tauri/target/windows-wgc-close-supply-chain/RESULT.json。
Windows 本机与 Native CI 的 vendor lib tests/lint 入口在独立 ci 提交接线，源修复与 CI 分开提交。
干净源码完整门禁与真实 Cargo vendor 图待验；已安装源码仍为 45769c9，桌面未操作。
