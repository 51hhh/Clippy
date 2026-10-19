# WIN-WASAPI-ACTIVATION-TAIL-01 — 音频激活边界包尾审查

父任务WIN-NATIVE-01 / W83、W06；关联PX-REC-WINDOWS-AUDIO-01、REC-FIRST-FRAME-AUDIO-01、
REC-AUDIO-ACTIVATION-BOUNDARY-01。[规范](../superpowers/specs/2026-10-03-wasapi-activation-tail.md)。
分支`codex/windows-wasapi-activation-tail`；基线文档`a142011191a6f7ee18659b048ae3a0ac2ab5eaa5`；
规范先提交`c4bb950659654ec541c0d99ce2c404072ec767c5`；被测修复源码
`88891927654717a3d7524c6d94648daca4ad102a`。

## 缺陷与行为

原copy_packet仅在整个包的captured_at_ns不早于not_before_ns时入队。20ms包从100ms开始，
恢复下界110ms时，110–120ms的480帧也被丢弃；50ms原包还会丢失合法的后续完整拆块。
Start或恢复后的激活时钟可以落在第一原生包内部；这里只证明源码和受控合同缺口。
Microsoft [GetBuffer合同](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)
给出的QPC是包内第一帧。平台保持完整GetBuffer/复制/ReleaseBuffer的原帧数，只裁切已经
归一化并取得所有权的PCM，不能据此宣称实际端点/驱动/声音已复现。

纯helper删除下界之前的完整块；唯一跨界块按ceil(delta_ns * 48000 / 1e9)裁切旧样本。
u128乘法处理大时间差，先有界比较再转换，用get和checked_add传播形状/时间错误。
新块PTS对应原样本网格的首个保留帧；原序号、格式、之后块字节和包next_sequence/end_ns
保持。全旧包为空，恰好从下界开始或更晚的包原样保留，静音只保留原零样本。
没有使用抵达时钟、另造原点或虚构PCM；48kHz/20ms拆块、QPC校准/100ns精度、重叠检查、
队列/等待预算、COM/event所有权、Start/Stop/Reset、endpoint选择、停止尾包和公共暂停保持。

## 受控原生回归

先将原整包起点谓词在实际packet_to_chunks之后提取为受控helper。对照是dirty提取协议，
没有未修改COM实例或设备录音；五个跨界案例失败，完整旧包及原样保留两例通过。
原生MSVC对照2 passed / 5 failed，Cargo101/包装器和终端1（25399）；修复后同一回归文件
原字节7 passed / 0 failed，全部exit0（53141）。两进程均终结，原日志/diff/输入保存。
七例覆盖部分包、多拆块、非整样本下界、实际AudioPipeline恢复后的10ms有效尾部、静音、
全旧包含u64最大下界、包含边界原样保留。没有调用WASAPI/声音端点或桌面API。

两份生产文件以逆模板验证原接线/include整体保持；17旧父测试正文词法哈希不变，其它
1070份原源/配置/构建/测试完整原字节保持，旧断言和期限没有延长。

## 完整Windows门禁

干净源码Windows11/MSVC、Windows PowerShell5.1实际`./scripts/ci-windows.ps1 -RecordingQa`：
33 passed / 0 failed / 1 Linux smoke skipped；native子进程/包装器/终端exit0，54992已终结。
默认Rust1341、QA1493各5ignored，前端81文件/1405。新增七项在两个图中分别通过，共14次
执行已含总数；QA录屏523包含于1493，重叠图不累加。原W81所有通过全名仍在相同阶段通过；
独立剪贴板31/vendor18、Python质量33/视觉3保留原阶段，1073冻结输入与Git tree/原日志哈希绑定。
PS5.1扩展字符串只投影value并匹配原日志，原RESULT保持；jsdom Canvas提示为原渲染限制，
不当作真实桌面像素证据。累计51产品修复/50历史，W80测试诊断另列。

## 未验与闭环

当前8889192 GitHub只读查询422/no commit，已知45769c9控制成功，仅确认读取通道；没有
推送或触发workflow，同SHA七项CI仍not_run。当前源码默认/QA release未编译，旧00f两份
release和D05历史文件保持实际SHA，不含本项；编译/PE/CRT文件也不计真实启动或测试通过。
真实首包/暂停恢复、WASAPI设备/默认路由/权限/声音、长时同步、Win10、多屏混合DPI/
负坐标、无开发CRT启动、安装升级卸载/updater、Linux/macOS原生图和人工QA仍未完成。
旧45769c9包39项桌面记录保持2passed/1failed/36not_run，Pin失败与未复测保持。
W53/W59/W63/W67原媒体丢失和失败根因仍未明，新裁切及W80诊断不能解释旧失败。

初始一组搜索曾落到旧Root检出，并误传Windows literal glob；重新按当前A142工作树读取
后才修改/验证。诊断记录保留，不计为native失败或测试。原8R/9AC/47Tasks保持；本规范
前四项局部AC完成，末项复合AC和最后两条全局AC仍空。桌面停止，无安装/推送/PR/合入/发布。
下一步可在独立target核对当前两图release编译及文件来源，不启动程序或操作设备。

证据：`C:\win\Clippy\src-tauri\target\wasapi-activation-tail-contract`；完整门禁：
`C:\win\Clippy\src-tauri\target\wasapi-activation-tail-native-qa-8889192`。全局任务未完成。

## Release Verification（W84）

同一8889192源码默认/显式QA release编译及PE/CRT文件核对完成，752编译输入/1073门禁输入绑定；只验证文件，不计测试或真实启动。原门禁保持；末项复合AC、同SHA CI/其它宿主/真实桌面与完整交付仍未完成。
见 [当前release文件验证](2026-10-04-windows-wasapi-activation-tail-release-review.md)。
