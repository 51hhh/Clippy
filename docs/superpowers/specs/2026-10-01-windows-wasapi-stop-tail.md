# WIN-WASAPI-STOP-TAIL-01 — Windows WASAPI 正常停止保留尾部 PCM

## Goal

修复 Windows 录屏音频 Stop 与 Pause 共用清空路径造成的尾音丢失；正常停止应提交已复制 PCM
及停止后仍在 endpoint 中的有限尾包。对应 WIN-NATIVE-01 / W06 的独立 W24，基于 da18681。
录屏仍是非默认 QA feature，桌面操作保持停止，不声称已经观察到真实音频设备的尾音缺失。

## Requirements

1. Pause / Drop 保持 Stop、Reset、清空旧 PCM；正常 Stop 在成功停止流后读取剩余 packet，
   Reset 后保留本地 PCM，通过既有 take_stopped_chunks 在 pipeline.finish 前提交。
2. Stop 排空以 Initialize 后 GetBufferSize 的实际帧容量为上限，不能按请求的 20 ms 猜容量；
   每包读取前检查剩余预算，超限或 API/时间戳/释放错误继续失败关闭，不能伪造完成。
3. packet 继续使用已有 QPC 映射、静音处理、20 ms 拆块、序号、重叠与恢复过滤合同；
   Stop 时间戳不能早于排空后的最后 PCM frame 末尾，尾部队列一次取走。
4. 生产 Stop / Pause 控制入口和离线故障注入共享协议；测试必须验证真实样本、有限排空、
   暂停清空、零尾包、控制/读取/重置失败，而不创建 COM、音频 endpoint 或系统录屏。
5. 独立需求、CHANGELOG、总计划与审查报告使用同一 ID；干净源码完整 Windows 默认与录屏
   QA 门禁分别记录，远程同 SHA CI、真实系统声/麦克风/混音和 Windows 10 保留未验。

## Acceptance Criteria

- [x] 保持旧 Stop/Reset/清空行为的离线红基线暴露已复制及 endpoint 尾音丢失。
- [x] 修复后保留拆块/静音/序号/时间边界；Pause 与失败关闭、实际容量有限排空通过。
- [x] 干净源码完整 Windows 默认/录屏 QA 本机门禁通过，真实原生 API 图编译/lint。
- [x] 规格、CHANGELOG、计划和报告同步；桌面与远程 CI 边界保留。

## Out of Scope

启用录屏默认 feature、设备热切换、重新设计音频 mixer/时间线、真实设备采集、桌面操作、
安装新包、Linux/WSL 本机验收、合入 dev 或发布。失败后的已采集 PCM 不伪装成正常完成。

## Review Evidence

WindowsWasapiAudioSource::stop_capture 调用 stop_and_reset，后者 Reset 并 pending.clear；
source 沿用默认空 take_stopped_chunks。既有 audio worker 和 mixer 已在 Stop 成功后提交尾部，
Windows source 没有交付。原录屏音频规格禁止静默丢包，混音规格要求两路有限尾部提交。

微软 [Stop 合同](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-stop)
说明停止数据流；[Reset 合同](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-reset)
说明清除待处理数据；[GetBufferSize 合同](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudioclient-getbuffersize)
提供实际 endpoint 最大帧容量。[GetNextPacketSize 合同](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getnextpacketsize)
规定下次 GetBuffer 的帧数相同，查询、读取和释放在同一线程执行。

## Verification

MSVC 辅助 harness 包含完整 windows_audio_contract.rs，链接锁定 thiserror；外围 PCM 类型
使用最小 stub，endpoint 全部 fake，不调用 COM、WASAPI 或录屏。红版本提取旧 Stop/Reset/清空
协议（没有尾包查询），13 项中 9 passed / 4 failed，exit 101；七项既有合同不变，六项新增合同
中 2 passed / 4 failed。修复后 13 passed，exit 0；包括 960 帧已复制拆块、17 帧非零尾样本、
480 帧静音、序号 8/9/10 和末尾 50,354,167 ns 的字面断言，暂停清空和零尾包继续通过。
容量 0/17/2 与持续返回非空 packet 的负例保证排空有限；Stop/查询/读取/Reset 失败均返回错误。
循环负例红状态在首个失败断言停止，不能宣称每个循环输入分别红复现；绿状态执行全部输入。

证据在主检出的 src-tauri/target/windows-wasapi-stop-tail-red。该 harness 只验证共享协议，不替代
完整 Cargo API 图；Windows source 已接线 Preserve/Discard、实际容量查询与一次性尾块提交。
原 read_packet 的复制、QPC、过滤和 ReleaseBuffer 路径保持。暂停或正常停止后重复 Reset 使用
API 的已重置成功合同；重置失败仍返回错误，Drop 继续尝试清理，不能视为成功完成。
干净源码 `a463c3ba9be876dbfe1a45893dcca28e013cadb6` 完整 Windows 默认/录屏 QA 门禁
exit 0，27 passed / 0 failed / 1 skipped（Linux smoke）。默认 Rust 1058 passed / 5 ignored，
QA Rust 1111 passed / 5 ignored（重叠不累加）；13 项音频合同在真实两个 Cargo 图均通过，
已包含在 Rust 总数。既有 worker 尾块提交和 mixer 双源有限尾部测试在两图通过。
前端 75 文件 / 1292 passed，Python 33 + 3 passed；四组 Windows 剪贴板合同共 24 passed。
验证后检出干净，stdout/stderr 哈希已核对；录屏 QA 的真实 WASAPI API 图 check/clippy 通过。

完整证据在主检出的 src-tauri/target/windows-wasapi-stop-tail-native-qa-a463c3b/RESULT.json；
红绿辅助证据与原失败日志保留。现有 Windows 原型 CI 的 windows_audio 前缀包含此合同，
不需要新入口，但新 SHA 远程 CI 未运行。后继提交仅同步四份 Markdown，不能冒称该文档 SHA
执行过门禁。已安装包仍为 45769c9，不含本修复；桌面、真实设备/混音/听音、设备拔出、
Windows 10、多屏和其它宿主的纯合同图仍未验，Linux/WSL 没有启动。
