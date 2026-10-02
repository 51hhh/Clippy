# REC-MIXED-STOP-READY-BOUND-01 / W75 — 混音停止输出的固定预算

生产源码 `2bb12f9eeb04f98217189c197ecafef83b326604`，分支 `codex/windows-mixed-stop-ready-bound`；基线文档 `1e17b34` / 源码 `02005c9`。
规格先行 `8d98383`，原API诊断/策略 `e8110c3`，见 [REC-MIXED-STOP-READY-BOUND-01](../superpowers/specs/2026-10-02-mixed-stop-ready-bound.md)。

## 问题与修复

活跃Stop原先按整个剩余时间段pop_ready并积累Vec，输出内存随时间增长。两秒有限间隔的原源首批96960帧（无首包97920），超过一秒48000帧预算。
现在Stop只验证两路原生控制/有限尾部并冻结最终水位；按输出原点和最终帧派生控制末尾，覆盖取整后的全部PCM。
读取每批最多50个20ms块/48000帧，空批表示结束；worker完整排空后才pipeline.finish，后续批错误立即失败。
每个输入的全部有限批仍先通过原staging预算/格式/序号/真实早停检查；平台enum逐批转发无需修改。
Windows pending.drain、macOS mem::take和Linux默认空尾部仍在下一读取返回空批；其它宿主未原生编译。
固定增益、PCM/PTS/序号、原生控制下界、Exact、公共A/V暂停与一秒pipeline背压保持。
暂停中的实际尾部、Stop源身份和真实错误仍拒绝；默认录屏入口保持关闭。

## 原实现与修复后的证据

同字节原API九项2 passed / 7 failed，Cargo101、包装器/终端1，实际session64688/native31252。
源三例首批超预算；拟议分批合同夹具经原worker只读一次，2880帧只有1920帧被提交，后续批错误未被读到。
原完整owner两例complete60ms/1920编码输入，错误例也错误完成；这是新分批接口必须同步worker的合成兼容诊断，不冒称旧原生单批尾部漏包。
原Backpressure和真正原生Stop错误两项保护通过。原journal/媒体和Drop全部保存，合成源不计设备验收。

修复后完整录屏领域501 passed / 0 failed = 原491 + 新10；九项规范夹具字节保持，另一天间隔安全用例只在修复后运行。
源首批48000帧；逐样本/时刻/序号排空后包含前缀共97920帧，两路先后停止及无首包均保持全部合法样本/静音。
worker全部批成功为60ms/2880帧，后续批错误为Source且保留1920帧前缀；原背压仍Backpressure并保留48000帧。
完整owner成功60ms/6视频帧/2880编码输入及有效PCM、无起始裁剪；后续批失败仍AudioCapture(Source(...))/interrupted。
后续批错误例仅保存manifest，未产生已提交分段；此例证明错误状态/Drop，不冒称已验失败媒体恢复。既有完整失败/恢复模块保持并在领域通过。
音源Drop2、视频Drop1、两路Stop均尝试；原122份录屏文件旧完整测试模块/原期限保持。
真实ffprobe/ffmpeg仅检查合成文件：6 VP9/4 Opus、pre-skip312/discard648、2880有效PCM、60ms。
旧文件末20ms为低能量静音，修复后末段保留0.1增益信号；解码文件/日志/原文件哈希保存，不播放/不开设备。

## 冻结源码完整Windows门禁

`2bb12f9eeb04f98217189c197ecafef83b326604` 干净源码默认/QA完整门禁33 passed / 0 failed / 1 Linux smoke skipped，终端session13446实际exit0。
默认1316、QA1467，各5ignored；前端81文件/1403passed，Python33+3、独立剪贴板31/vendor18分层。
新10唯一用例：8两图/2仅QA，实际18次已含各图总数；两图重叠不累加。原API/领域、文件检查、release和CI分层，矩阵/安装器不计通过数。
没有桌面/应用/设备操作，也没有安装、签名、系统工具变更或推送。

当前源码release尚未编译，旧02005c9 release仅证明前置源码；已安装45769c9包不含新修复。
同SHA三native/四原型CI、其它宿主、实际设备/桌面、安装升级卸载/updater、开发CRT缺席环境、Win10/多屏/混合DPI/负坐标/长时同步仍未验。
W53/W59/W63/W67旧失败根因继续未明，不用本轮绿结果覆盖。累计48项本机修复，47项历史保存，全局任务仍未完成。

机器证据：`C:\win\Clippy\src-tauri\target\mixed-stop-ready-bound-contract` 的BASELINE-API-AUDIT/CONTRACT-AUDIT/MEDIA-INSPECTION，
`C:\win\Clippy\src-tauri\target\mixed-stop-ready-bound-native-qa-2bb12f9` 的RESULT/GATE-AUDIT。

W76后续：同源码默认/QA release编译及PE/CRT文件验证完成；实际启动/设备、CI/其它宿主和桌面等仍未验。
见 [release验证](2026-10-02-windows-mixed-stop-ready-bound-release-review.md)；最后复合验收继续未完成。
