# WIN-RECORDING-TEST-ARTIFACTS-01 — 录屏失败测试的合成文件证据

## Goal

补齐 WIN-NATIVE-01 的原生测试诊断缺口：session/AV encoder 合成测试断言失败时保留当时目录，
使后续超时、manifest 和媒体错误有实际文件可以检查；不能用补诊断解释已丢失的历史失败。

## Requirements

1. 先用锁定 tempfile 版本和原 TempDir API 的受控 unwind 对照，记录嵌套 manifest/partial/分段
   被清理的实际结果。它是目录所有权诊断，不是 W53/W59/W63/W67 原因复现。
2. 保留机制只编译到测试图；session 和 AV encoder 测试的 tempfile 名称解析接入该机制，
   原测试函数、断言、输入、include 文件和等待期限保持，生产 worker/编码/日志/权限不改。
3. 普通退出仍按原 TempDir 清理；测试线程 unwind 时只解除自己的目录自动删除，不复制、移动、
   重写媒体或修改 manifest。记录绝对目录的 JSON 路径，由原生命令日志及冻结源码绑定来源。
4. 诊断输出尽力写入，写失败不能二次 panic、掩盖原错或改退出码；不执行重试、放宽期限、跳过
   原断言或通过媒体容错把失败算作通过。强杀、abort/OOM 和平台临时目录清理不在保证内。
5. 测试验证普通清理、unwind 后原字节/嵌套目录、输出错误和独立目录；session/AV 原 owner 的
   Drop/abort/join 在目录保留之前完成，保留真实合成编码的 interrupted 清单与已提交前缀。
6. 对照包故意失败的 native 子进程必须保留非零退出、stdout/stderr、路径和原文件哈希；修复后
   同一诊断正文通过。另跑干净源码 Windows 默认/QA 完整门禁，比较不计新测试通过数。
7. 文档使用同 RID，记录来源 SHA、完整门禁和当前 release/CI/设备边界。产品修复数保持49，
   此项是测试诊断修复；原历史失败、原包39项桌面记录及原未完成验收不改写。

## Acceptance Criteria

- [x] 原 API 受控 unwind 对照实际失败，原目录删除与文件输入/日志绑定保存。
- [x] 测试专用目录在 unwind 保留原字节及 JSON 路径，正常测试仍清理，输出错误不改原失败。
- [x] 原 session/AV worker 的真实合成已提交前缀和 interrupted 状态在 unwind 后可读。
- [x] 原完整测试函数/期限/生产代码保留，干净源码 Windows 默认/QA 完整门禁通过。
- [ ] 当前源码同 SHA 跨平台 CI、真实设备/桌面和完整交付验收完成。

## Out of Scope

- 重建或解释已丢失的历史失败媒体，归因 Defender、索引、调度或设备故障。
- 捕获真实屏幕、剪贴板、系统音频、麦克风，恢复桌面操作，安装/启动应用。
- 产品崩溃恢复、release panic=abort 的 Drop 保证、全仓库所有测试保留、CI 自动上传诊断文件。
- 推送/发布/合入、放宽原等待预算、自动重试或取消失败退出。

## 验证记录（W80）

先定义合同 `bb9c327`，测试专用修复源码 `e50192f3a51070b12b00d5b9e01087f1ffc5fb27`。
原 TempDir API 与修复后目录绑定使用同一诊断函数原字节：原生对照0 passed / 1 failed，
Cargo exit101 / 包装器及终端exit1；原目录确已删除。修复后1 passed / 0 failed，exit0，
原目录及三份固定输入文件字节/哈希、JSON报告路径和保存副本一致。该独立诊断不产生有效编码媒体，
不解释历史 worker/清单失败，也不包含在应用测试总数。

新增应用7项（6项两图、1项仅QA），包括真实合成 session/AV owner unwind 后的 interrupted
清单、提交前缀和原校验/VP9/Opus解码。原217份输入按字节核对，三份旧父文件仅插入测试模块和
tempfile绑定，19项父测试正文、完整include及旧30/10/5秒期限保持。生产前缀不改、模块仅cfg(test)。
受控回归外层目录在断言结束后清理；独立绿色诊断的原目录及保存副本另行保留。

干净源码完整Windows PowerShell5.1门禁33 passed / 0 failed / 1 Linux smoke skipped；默认Rust1330、
QA1482，各5 ignored，两图重叠；前端81文件/1405。QA录屏516项及新7项（实际13次两图执行）已含总数。
1068份源码/测试/配置冻结输入及原日志哈希绑定，旧d05通过全名仍在相同阶段通过。
当前e50192f release/同SHA CI、设备/桌面及其它宿主未执行；原d05 release只保留其实际来源。
产品修复仍49，此项单列测试诊断；原最后两项全局AC、39项旧桌面记录及历史失败根因未明状态保留。
详见 [审查记录](../../reviews/2026-10-03-recording-test-artifacts-review.md)。
