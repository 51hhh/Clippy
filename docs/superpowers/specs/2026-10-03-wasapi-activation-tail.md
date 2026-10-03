# WIN-WASAPI-ACTIVATION-TAIL-01 — 保留音频激活边界后的包尾

## Goal

补齐 WIN-NATIVE-01 / W06、PX-REC-WINDOWS-AUDIO-01、REC-FIRST-FRAME-AUDIO-01 与
REC-AUDIO-ACTIVATION-BOUNDARY-01。当前 Windows copy_packet 只要包起点早于启动/恢复下界
就丢弃整包；跨越下界的合法 PCM 后半段及后续拆块也一起丢失。按原生样本边界保留有效包尾。

## Requirements

1. 先保存原整包起点筛选协议的受控原生对照。复用真实 packet_to_chunks 与 AudioPipeline，
   明确这是原生产谓词的提取，不是未修改 COM/WASAPI 实例或真实设备录音复现。
2. 包内帧按固定 48kHz 网格选择首个不早于启动/恢复下界的样本；删除之前的完整块，只裁切
   唯一跨界块。保留之后的块、原序号、格式、样本字节，以及原 packet next_sequence/end_ns。
3. 原 QPC 校准与包输入 PTS 不改，不使用抵达时钟、不另建原点或补写虚构 PCM；裁切后的块
   时间戳对应原包的首个保留样本。下界为包含边界，非整样本下界向上取首个合法样本。
4. 全部在下界之前的包为空，恰好从下界开始/更晚的包原样保留；静音正确保留对应零样本。
   大时间差安全处理，不产生空块或 slice panic；原有效性/时间重叠/错误传播和 ReleaseBuffer 保持。
5. 只更改 Windows packet 保留接线与纯合同 helper，不改变 Start/Stop/Reset、COM/event所有权、
   默认/显式 endpoint、正常停止尾包、暂停控制、队列预算、格式、20ms拆块或非默认 QA feature。
6. 原完整测试模块、include、断言、期限和固定输入保留。干净源码 Windows 默认/QA 完整门禁
   另验；红绿相同回归字节与原日志/退出保存，构建或夹具不能计真实设备验收。
7. 独立分支与同 RID 的规范、审查、CHANGELOG；旧50项修复及历史失败/未验项保持。

## Acceptance Criteria

- [x] 原整包谓词在跨界受控 PCM 中丢失有效包尾，原输入、差异与失败退出保存。
- [x] 同回归证明部分包/多块包/非整样本下界的保留字节、PTS与序号正确。
- [x] 恢复下界后的真实 AudioPipeline 接受裁切块并保留尾部，静音/完整丢弃/原样保留成立。
- [x] 原正文/期限/输入保留，干净 Windows 默认/QA 完整门禁与记录绑定同源码。
- [ ] 当前源码同 SHA CI、release、其它宿主、真实首包/暂停恢复/设备和完整交付验收完成。

## Out of Scope

桌面或设备控制、录音/录屏、安装、推送/PR/合入/发布；改变 QPC、source hook、公共暂停区间、
采样格式、队列/等待预算或开放默认录屏。不能解释 W53/W59/W63/W67 已丢失媒体的原失败根因。

## Source Evidence

当前源码 `00f40cc5cf8b3cf3c332dc7cce6be47cd07aefcc`，文档 `a142011191a6f7ee18659b048ae3a0ac2ab5eaa5`。
Windows copy_packet 用 captured_at_ns >= not_before_ns 控制整组 packet.chunks 入队；例如20ms包
从100ms开始、恢复下界110ms，原谓词丢失110–120ms的480帧。50ms包还会丢失之后完整拆块。
这是有限源码缺口；受控协议证明与真实 WASAPI 仍须分层记录。

Microsoft 的 [GetBuffer 合同](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)
说明 QPC 时间戳对应包内第一帧，包包含 pNumFramesToRead 帧。原生 packet 仍须完整复制并完整
ReleaseBuffer；本修复只裁切已归一化并取得所有权的 PCM，不改变原生读取/释放帧数。

## Verification（W83）

规范先提交`c4bb950`，修复源码`88891927654717a3d7524c6d94648daca4ad102a`。原整包起点谓词经明确提取，
复用实际packet_to_chunks：原生MSVC对照2 passed / 5 failed、Cargo101/包装器及终端1；
同一回归文件原字节修复后7/0、全部exit0。恢复夹具使用实际AudioPipeline，保留原有效PCM
和10ms公共展示尾部；其它案例覆盖多拆块、非整样本下界、静音、全旧包和包含边界。
这是dirty提取协议的受控红绿，没有COM实例或实际WASAPI设备录音。

干净源码Windows PowerShell5.1完整默认/录屏QA门禁33 passed / 0 failed / 1 Linux smoke skipped，
默认Rust1341、QA1493各5ignored；前端81文件/1405，QA录屏523含于1493。七项两图共14次执行
已含总数，不累加重叠图。17旧父正文、其它1070完整输入、include和期限保留；1073冻结输入
绑定Git tree、原日志和实际exit。旧W81通过全名仍在相同阶段通过，产品修复累计51。

当前SHA GitHub只读查询422/no commit、已知45769c9控制读取成功，不推送/触发CI；同SHA CI
仍not_run。当前release未编译；旧00f默认/QA文件保留实际SHA，不包含本项。其它宿主、真实
首包/暂停恢复/设备/声音/长时同步/交付未验；最后复合AC保持空。原失败根因保持未明，
新边界修复不解释旧缺失媒体。见 [审查记录](../../reviews/2026-10-03-wasapi-activation-tail-review.md)。

## Release Verification（W84）

同一8889192源码默认/显式QA release编译及PE/CRT文件核对完成，752编译输入/1073门禁输入绑定；只验证文件，不计测试或真实启动。原门禁保持；末项复合AC、同SHA CI/其它宿主/真实桌面与完整交付仍未完成。
见 [当前release文件验证](../../reviews/2026-10-04-windows-wasapi-activation-tail-release-review.md)。
