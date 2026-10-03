# WIN-LONGSHOT-INPUT-CANCEL-01 — 取消后旧线程输入审查

父任务WIN-NATIVE-01 / W81、W04；关联PX-LS-NATIVE-AUTO-01。
[规范](../superpowers/specs/2026-10-03-longshot-input-cancel.md)。分支`codex/windows-longshot-input-cancel`。
基线文档`64e5abfd77e97ac164a9801a7618c5e4190f4eca`；规范先提交`6d49204def0a1b2fe06282f0cda2f56fa45bbe72`；
被测修复源码`00f40cc5cf8b3cf3c332dc7cce6be47cd07aefcc`。

## 缺陷与最终行为

原controller.cancel仅调用cancel_wayland并失效manager lease，该方法在Windows为空；旧锁外线程
仍可继续with_scroll的移动/激活/滚轮/恢复，最终结果被拒绝不能撤回这些调用。修复按会话共享输入
许可：匹配取消先不可逆撤销，再使manager lease失效，最后等待已进入的输入临界区结算后交还owner。
输入许可与同步mutation共用短临界区；旧/错误token不能撤销后继，克隆不能复活原许可。

原物理坐标、窗口/PID锁、UIPI/权限检查、四方向、3像素容差、45/360/500毫秒常量及图像质量门保持。
临界区只围住移动、Windows/macOS激活、滚轮和RAII恢复；初始化/settle/目标/抓帧边界复核许可，
不等待像素处理或强杀线程。正常/无人接管失败仍按原guard恢复；取消后的迟到恢复不再移动，
在入口已读到撤销状态时跳过查询；检查后发生取消时仍由mutation临界区阻止移动。
共享Native路径影响X11/macOS，Wayland原分支和Portal cancel保持，不宣称其它原生图已验。

“取消完成”指已进入输入临界区先结算；不能撤回OS已排队事件、保证API不阻塞或观测所有外部程序
竞争。取消整个会话与前端仅停止自动循环不同；本修复不改变继续使用同会话的前端暂停按钮。

## 原生受控验证

为验证原无撤销行为，将原同步mutation调用边界提取为无检查/无撤销转发协议，原窗口检查、输入
顺序和原测试保持。四项测试使用真实controller、共享克隆/后继、真实线程/通道及同一RAII guard，
原生MSVC对照0 passed / 4 failed、Cargo101/包装器终端1（会话5788）；修复后同三份测试文件原字节
4 passed / 0 failed、全部exit0（35750）。两进程均终结，原日志和dirty输入/diff保存。

对照是明确提取边界的协议红绿，不是未修改原应用/原生鼠标操作复现。没有调用本机指针、焦点或
滚轮API。新四项覆盖取消后克隆拒绝、旧token与后继隔离、已进入调用结算顺序、迟到恢复禁用。
旧23项父测试正文按词法哈希保留；其它1066份源/配置/测试完整字节不变，包含原include与期限。

干净修复源码Windows11/MSVC、Windows PowerShell5.1完整`./scripts/ci-windows.ps1 -RecordingQa`：
33 passed / 0 failed / 1 Linux smoke skipped；native子进程/包装器/终端exit0，42434已终结。
默认Rust1334、QA1486各5ignored；前端81文件/1405。新4项两图共8次执行已含总数；QA录屏516项
包含于1486，两图重叠不累加。独立剪贴板31/vendor18、Python质量33/视觉3保留原阶段。
旧W80通过全名仍在相同阶段通过，1072份输入与Git tree及日志原哈希绑定；jsdom Canvas提示仍为
原渲染限制，不当作真实桌面像素证据。此前49产品修复保留，此项累计50；W80测试诊断仍单列。

## 审计与有限依赖检查

前置脚本曾误读词法解析器的list/tuple格式，绿色快照比较又误将JSON数组与tuple比较；均在native
启动前失败，不计测试结果，原正文核对后已纠正。冻结清单初次把当前源码写进baseSpecSha字段，
初始文件另存后更正spec关联；实际sourceSha/Git tree及1072输入未变，不重跑门禁或改原日志。
PS5.1带注释字符串仅投影value并匹配原日志，完整原RESULT保留，未打印或替换为规范JSON。

锁定enigo0.6.1的Windows源码有限检查：构造时held键集合为空，scroll发送wheel且不登记held键；
Drop只释放登记键。本调用链只scroll，因此没有额外析构键输入。原文件SHA保存，不修改库、默认
设置或依赖。这不是OS队列/真实输入路由证明。当前生产架构同步记录输入许可与owner结算边界。

## 未验与闭环

当前00f40cc release编译/PE/CRT、新SHA七项CI、真实取消/接管/焦点/滚轮/窗口销毁与OS竞争、
Win10/多屏混合DPI/负坐标、权限、音频/长时漂移、安装升级卸载/updater、无开发CRT环境、Linux/
macOS原生图及人工QA仍未完成。原d05 release保持实际SHA和编译/文件范围，不含本修复。
旧45769c9安装包39项桌面记录仍2/1/36，不含后续50项修复；原Pin失败及未复测不改。
W53/W59/W63/W67历史失败根因保持未明，新许可和W80诊断不能解释已丢失的原媒体。

原8R/9AC/47Tasks及最后两条全局AC保持；此规范前四项局部AC完成，末项跨平台/真实交付仍空。
桌面继续停止，没有应用/设备控制、安装、推送、PR、合入或发布。下一步在独立target核对当前默认/
QA release编译与来源，仅作文件验证，不把构建计为测试或启动应用。

证据：`C:\win\Clippy\src-tauri\target\longshot-input-cancel-contract`；完整门禁：
`C:\win\Clippy\src-tauri\target\longshot-input-cancel-native-qa-00f40cc`。全局任务仍未完成。
