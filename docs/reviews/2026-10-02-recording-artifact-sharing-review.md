# WIN-NATIVE-01 / W57 — 已提交录屏产物提升共享冲突

需求 `REC-ARTIFACT-SHARING-01`；基线 `b2f6e955c76e663285decbc69d6a1adc4ee16feb`；
独立分支 `codex/recording-artifact-sharing`。生产修改前建立
[`规格`](../superpowers/specs/2026-10-02-recording-artifact-sharing.md)，沿用
`PX-REC-AV-MANIFEST-01` 的清单先提交、产物后提升和可验证前缀合同。

## 原生复现

真实 Windows File 句柄允许读写而不共享删除，且原生 DELETE 访问探针确认冲突。
分段/最终输出提交的夹具在清单原子提交后继续短暂持有 75 ms；正常启动恢复直接
持有已经被清单声明的 partial；恢复 remux 观察实际创建的 partial 并持有至 finalizing。
原操作返回或 worker 提前退出时清理句柄，避免用额外等待遮住原错误。
文件 I/O、journal、codec 和 worker 没有被替换；帧与媒体使用合成输入。

生产未修改的首轮 native exit 101，2 passed / 6 failed，约 0.31 秒。五个提升点
全部取得实际 `os error 32`：分段提交、最终输出提交、分段启动恢复、最终输出
启动恢复和 remux 输出提交。第六项在原 10 FPS/200 ms MJPEG session 取得
Encoder/Mux/Journal 的同一分段提升根因。永久冲突保留已提交前缀及损坏 partial
拒绝提升两项已经通过；不是把所有用例都写成原实现失败。

## 修复

五个已提交产物提升点复用 W56 的 Windows 有界共享冲突策略。清单使用同一策略，
仅重命名私有 wrapper；32/33 或经原生探针确认共享拒绝的错误 5 最多尝试
21 次/500 ms，每次等待最多 25 ms。单次原生 I/O 的阻塞不受该窗口承诺。
原 retry/probe 算法没有改变；权限错误、已提升源和永久冲突仍返回原错误。

fsync、清单提交、ACL、哈希/长度和分段/音轨统计、提升与目录同步顺序保持。
旧权限工具、导出保存、采集时钟/FPS/预算及其它平台的原调用保持。恢复先验证
长度和哈希，损坏 partial 不进入重试；永久冲突后合法已提交 partial 保留，可在
句柄释放后恢复。没有忽略异常、扩大队列、增加旧测试期限或删除原断言。

## 验证证据

八项新增合同全部使用旧 API；五项在默认/QA 两图，三项 VP9 仅在 QA。
实际 AVI 文件与报告/清单核对；WebM 产物除完整文件长度/哈希外，再通过原严格
remux reader 遍历 packet，核对全帧数和总时长。原阶段结果及输入/日志哈希保留。

默认严格 lint 曾拒绝新夹具的 `future_partial` 未使用方法；该方法仅由 VP9 恢复
remux 用例消费。增加相同 VP9 feature 条件，保持所有旧测试与生产代码，不允许
警告豁免。旧阶段红绿/原重放及 lint 失败保留，最终相同字节对照使用修正后的夹具。

同字节八项旧API夹具原实现2/6（六条实际错误32）、修复领域393/0=原385+新8。
干净 `830b12b43051efa7eebad36a3d1a4434975785b2` 完整 Windows 门禁33 passed / 0 failed / 1 Linux smoke skipped；
默认Rust1240/QA1359各5ignored，两图重叠不累加；前端81文件/1403passed。
新5在默认/QA两图、新3仅QA，已含在Rust总数；两个旧完整测试模块与原权限/恢复保护保持。
三份运行期夹具字节相同，全部使用原API，无接口stub；原retry/probe算法与导出保持，十四份Rust输入绑定干净SHA。
旧实现重放及finally恢复、最终领域与完整门禁的实际日志/输入哈希保留。

证据目录 `src-tauri/target/recording-artifact-sharing-contract/` 保存 W56 原状态、
初始红、阶段绿、原 Git 源码重放/finally 恢复、最终领域和冻结 source SHA 门禁。
文件/Git比较不计测试通过；领域与新增项已经包含在 Rust 总数内。

## 未完成项

桌面继续停止。实际 WGC/WASAPI/长时录屏、安装器/无 CRT 启动、当前 SHA 三宿主/
codec CI、其它宿主本地门禁、Windows 10/混合 DPI/负坐标/多屏与发布未验。
W57 当时本 SHA release 未构建；W58 已补充默认/QA 编译与文件核对，见
[后续验证](2026-10-02-current-windows-release-review.md)。旧安装 `45769c9` 与旧 QA EXE `1c66112` 不含本修复。
W53 历史 AVI 一次 30 秒超时没有 worker 原错，本轮独立共享冲突不能作为其根因。
旧桌面仍为 2 pass / 1 fail / 36 not_run，整体目标未完成。
