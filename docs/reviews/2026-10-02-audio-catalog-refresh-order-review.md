# REC-AUDIO-CATALOG-ORDER-01 / W77 — 音频设备查询完成顺序审查

源码 `d05cd3e478934722273a33fb88c841648aeb1ef5`，分支 `codex/windows-audio-catalog-refresh-order`；基线文档 `10dbd71` / 源码 `2bb12f9`。
规格先行 `8293d9249318f092c25dd8e264e8970d0959454a`，见 [REC-AUDIO-CATALOG-ORDER-01](../superpowers/specs/2026-10-02-audio-catalog-refresh-order.md)。

## 问题和修复

首次能力查询仍在 blocking 枚举时，覆盖层允许以默认选择开始。若启动失败，前端会查询新目录后重新开放重试。
前端 generation 已保护旧响应，但原后端 refresh 在枚举完成时换发目录：新查询先返回、旧查询后返回时，
旧完成会作废前端仍在使用的新目录。原 API 受控顺序诊断实际返回 Err(StaleCatalog)，唯一断言失败，
Cargo101 / 包装器和终端1（session59066/native24536）。这是真实目录合同复现，未执行 Tauri IPC 或原生设备。

现在在 await/枚举之前 begin_refresh 预留 caller 的查询身份并替换旧目录；complete_refresh 只发布当前且未发布的身份。
待发布和已发布目录共用32目录上限；预留10分钟内须发布，发布后快照重新计算10分钟有效期。
迟到成功、枚举失败换发的空结果、重复发布、消费后回写、过期和容量淘汰均不能插入或复活目录。
待发布目录不能被 token 消费；无 token 系统默认选择可继续使用，不消费待发布目录。
调用者、设备类别/模式、64设备/类、160 scalar 标签、原生身份私有和一次消费保持。
平台枚举仍在 spawn_blocking，Windows endpoint 枚举/连接、混音与音频时间线没有改变。
仅测试的同步 refresh 组合两个阶段以保留旧8项完整测试。新接口回归与旧 API 诊断分开记录，未冒称同字节 API 对照。

## 验证

目录合同16/0（原8+新8），终端90431/native49888/exit0；前端定向2文件63/0，含旧能力和覆盖层完整测试与新2项迟到响应保护。
前端两个新用例验证首次查询迟到成功或失败时仍可提交重试目录的设备；它们保护既有前端行为，不计为前端产品修复。
原127个录屏/前端文件冻结，除两个生产文件及追加前端保护外均保持；原目录完整模块与前端完整模块保留，测试期限未改。
原诊断编译输入的原始 SHA256、日志和结果已核对；修复中的文件换行匹配准备脚本首次失败后纠正，不是原生测试失败。

干净源码 `d05cd3e478934722273a33fb88c841648aeb1ef5` 执行 ./scripts/ci-windows.ps1 -RecordingQa，33 passed / 0 failed / 1 Linux smoke skipped，终端session60115实际exit0。
默认1324、QA1475，各5ignored；前端81文件/1405passed。新10唯一用例为8 Rust两图/2前端，门禁中实际18次，均已含总数；两Rust图重叠不累加。
QA图中的509项 recording 用例已含1475，未额外重跑领域或累加。Python33+3、独立剪贴板31/vendor18沿门禁分层。
完整Git tree和全部输入原始哈希核对，Cargo.lock/vendor不做LF归一化；门禁后只变更本审查相关七个Markdown文件。
没有桌面、应用或设备操作，没有安装、签名、系统工具变更或推送/PR/合入/发布。

## 未完成边界

当前GitHub源码commit查询返回422 No commit found；同调用已知45769c9正控成功。当前同SHA CI不计通过。
当前源码release尚未编译，W76同源码2bb12f9的默认/QA release只证明前置源码；已安装45769c9包不含本轮修复。
其它宿主、真实默认/非默认设备、拔插、同名设备、桌面、安装升级卸载/updater、无开发CRT环境、Win10、多屏/混合DPI/负坐标与长时同步继续未验。
W53/W59/W63/W67旧失败根因未明，不以绿门禁覆盖。累计49项本机修复/48历史，WIN-NATIVE-01全局验收未完成，原设备规范的CI/真机复合AC仍未勾选。
本轮原计划Requirements/AC/Task复核仅用于确定审查方向；没有把W74的47项全局inventory冒称当前49项完整重审。

机器证据：`C:\win\Clippy\src-tauri\target\audio-catalog-refresh-order-contract` 的 BASELINE、baseline/green-RESULT、CONTRACT-AUDIT、NATIVE-GATE-INPUT-AUDIT、CURRENT-SHA-REMOTE-EVIDENCE，
`C:\win\Clippy\src-tauri\target\audio-catalog-refresh-order-native-qa-d05cd3e` 的 RESULT/GATE-AUDIT；原失败诊断和旧release材料均保留。

W78后续：同源码默认/QA release编译及PE/CRT文件验证完成；实际启动/设备、CI/其它宿主和桌面等仍未验。
见 [release验证](2026-10-02-windows-audio-catalog-order-release-review.md)；最后复合验收继续未完成。
