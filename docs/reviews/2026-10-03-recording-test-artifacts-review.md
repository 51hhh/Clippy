# WIN-RECORDING-TEST-ARTIFACTS-01 — 录屏失败测试诊断审查

需求：[规范](../superpowers/specs/2026-10-03-recording-test-artifacts.md)；父任务WIN-NATIVE-01 / W80。
基线文档`abae79c688c03e7c55246880c4a81b7c6490e42b`，先定义合同`bb9c32712f3a4e851c18db8ab34ec543bdd65f9f`，
被测源码`e50192f3a51070b12b00d5b9e01087f1ffc5fb27`，分支`codex/windows-recording-test-artifacts`。

## 问题与变更

原session/AV合成测试使用TempDir，断言panic会在worker回收后自动删除目录，无法留下失败当时的
manifest、partial和已提交媒体作诊断。新增测试专用目录包装：正常退出按原规则清理，当前线程unwind
时先解除本目录自动删除，再尽力将JSON绝对路径写入stderr。输出失败不会二次panic；不复制、移动、
写入清单/媒体，不重试、放宽原期限或改变失败退出。原owner的Drop/abort/join仍按原顺序完成。
全模块只在cfg(test)引用，产品录屏与权限行为未改；累计产品修复保持49，此项单列测试设施修复。

仅在测试线程unwind时保留；线程外错误、panic=abort、OOM、强杀与系统TMP回收没有保留保证。
只覆盖已接入的session/AV合成测试，不能扩展为所有测试、产品崩溃恢复或CI自动上传保证。

## 实际验证

锁定原tempfile3.27.0/serde_json1.0.149缓存版本。独立Cargo对照使用同一诊断函数原字节：先写三份
嵌套固定输入，再受控unwind；原API因自动删除后文件NotFound而0 passed / 1 failed，native Cargo101，
包装器及终端1。修复绑定后1/0，全部exit0；JSON路径和原临时目录一致，三份原文件及保留副本的
字节数/哈希一致。这些固定字节不是有效编码文件，也不是W53/W59/W63/W67的原错复现。

应用新增7项：普通清理、unwind原字节、Unicode/空格JSON路径、独立目录所有权、输出错误，以及
真实合成session/AV owner unwind回收后的interrupted清单和已提交前缀。session验证原manifest文件
登记的媒体文件哈希/大小与RIFF前缀；QA AV复用原严格VP9/Opus读取器。受控回归的外层TempDir会在断言后清理，
独立绿色诊断原目录及三份副本保留在证据中；不将受控临时回归目录伪称持久失败媒体。

原217份录屏/前端/Cargo输入按原字节核对。三份旧父文件只有测试模块、tempfile别名/新include插入，
其它214份完全不变；19项父测试正文、完整include文件、辅助函数和旧30/10/5秒期限保留。
session/AV生产前缀完全相同，recording/mod.rs仅新增cfg(test)模块声明。

干净源码在Windows11/MSVC、Windows PowerShell5.1运行`./scripts/ci-windows.ps1 -RecordingQa`：
33 passed / 0 failed / 1 Linux smoke skipped，native子进程/包装器/终端exit0，终端25338已结束。
默认Rust1330、QA1482，各5 ignored；前端81文件/1405。新6项两图/1项仅QA，共13次执行，已含这些
总数；QA录屏516项包含于1482。两图重叠不累加，独立目录诊断1/0单列。独立剪贴板31/vendor18、
Python质量33/视觉3保留原阶段。1068份冻结输入和Git tree、原stdout/stderr哈希核对，旧d05通过
全名仍在当前相同阶段通过。jsdom Canvas提示保留为既有渲染限制，不能当真实Canvas/桌面验证。

## 审计异常与记录保留

准备时误填SHA被guard在启动native前拒绝，exit1，不计编译/测试失败。初次保留性脚本错误地假定
三份旧文件均CRLF，失败后曾提前提交；该时刻未通过审计，不计验证。随后按每份实际LF/CRLF和
原raw SHA严格重建允许插入模板，217份输入及原测试正文核对通过，修复提交未重写。

门禁已通过后，初次结果审计将PS5.1带注释字符串对象误作普通字符串，发生形状断言失败。
只投影原对象value字段并与原日志实际汇总匹配；原RESULT完整保留且raw SHA固定。这是审计形状
问题，门禁失败数仍0，没有为此重跑测试或改写原结果。所有异常及修正分别留在机器审计中。

## 未验证边界

当前e50192f的release编译、同SHA七项CI、真实设备/桌面、其它宿主未执行。W78默认/QA release
保持实际d05源码与原文件证据，本轮不声明新SHA二进制相同。原49项产品修复历史、原8R/9AC/47任务
和最后两项未勾选AC保持；当前生产架构不变，无额外架构更新。

旧45769c9安装包39项桌面记录仍2 passed / 1 failed / 36 not_run，不含后续修复。Pin复测、截图、
真实录屏/音频/30分钟漂移、Win10/多屏混合DPI/负坐标、权限、安装升级卸载/updater、无开发CRT
部署和Linux/macOS仍未验。W53/W59/W63/W67失败日志及根因未明状态完整保留；同SHA隔离通过和
新目录保留机制不能解释丢失的旧媒体，不归因Defender/索引/调度。

桌面操作继续暂停。后续仅做有限Windows代码审查，针对新确认缺陷定义合同并补原生回归；所有
受控目录/codec文件验证均不替代设备验收。没有应用/设备控制、安装、推送、PR、合入或发布。

证据：`C:\win\Clippy\src-tauri\target\recording-test-artifacts-contract`，包含原API及绿色命令/日志、
原输入哈希、原文件/副本、SOURCE-PRESERVATION/PRODUCTION-SCOPE/PROBE/CONTRACT审计与闭环。
完整门禁：`C:\win\Clippy\src-tauri\target\recording-test-artifacts-native-qa-e50192f`。
全局任务仍未完成；规范前四项局部AC已验证，最后一项跨平台/真实交付AC保留未勾选。
