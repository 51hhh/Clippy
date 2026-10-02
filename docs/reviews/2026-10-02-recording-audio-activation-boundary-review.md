# WIN-NATIVE-01 / W65 — 音频激活包含边界与 Windows 控制时钟

需求 `REC-AUDIO-ACTIVATION-BOUNDARY-01`；基线 `b791112c8866bebf57fae4c81241be260242e129`；分支 `codex/recording-audio-activation-boundary`。
修改生产前已提交 Goal/Requirements/AC/OOS 规格，先冻结原 API 运行期合同。

## 发现与修复

原音频 timeline 把 source 恢复时刻当成上一块 PCM 时间，恰好位于恢复时刻的首块被当作
重复时间戳。原七项最终夹具 2 passed / 5 failed；恢复边界、重复块保护前置、背压重试、
原 worker 正常停止及原完整 AV session 都在合法边界报 SourceTimestampNotIncreasing。
迟到块拒绝和旧 stateless Windows 夹持组合诊断两项通过。

恢复标记允许首个合法 PCM 等于原 source 恢复边界，只在成功入队后消费；真实 PCM 的
序号/时间严格递增、迟到/重叠拒绝、坏 source 恢复拒绝保持，背压失败仍可重试同一块。
Windows stateless helper 对暂停 clock20ms/恢复30ms和包末尾40ms均返回40ms，原 worker
因此拒绝恢复。新有状态控制时钟取原包末尾下界与前一控制时刻+1ns的较大值，checked
溢出不改状态；正常更晚 clock 保持。1ns只排序控制元数据，不改原生 packet PTS/QPC
校准/会话原点。Start原生调用集中到原 source，启动/恢复各只记录一个返回下界。

## 分层证据

最终七项原 API 夹具在红/绿之间逐字保持；首轮红记录另保留。另六项新控制时钟 API
绿色保护，覆盖夹持/倒退/真实更晚时间、多轮控制、溢出不改状态、原 worker 成功恢复与
溢出根错/单次析构/可读 PCM 前缀。旧 stateless 诊断继续拒绝不合法 source；它和新
状态类的合成 source 不是同一个真实 WASAPI endpoint，不能伪称原生设备红绿证明。

五个旧完整测试模块及期限逐字保持。最终录屏领域442/0=原429+新13；十二项两图，
一项完整AV仅QA，领域/新项已含后续 Rust 总数。新增溢出夹具曾重复定义，编译101，
没有运行用例；修正后日志与失败阶段保留，不计通过。生产修复逻辑不由该编译错误归因。

原完整 AV session 保存 interrupted 清单且无已提交媒体，根错为边界 PCM 拒绝；
修复后原 owner/真实VP9+Opus生成complete WebM，280ms、28视频帧、13440有效PCM帧。
实际文件/长度/SHA、清单/分段与source单次析构保存。独立ffprobe exit0读取28 VP9包、
15 Opus包、流initial_padding312和尾discard_padding648，得到13440有效帧；不播放媒体。

完整 Windows 门禁待冻结源码后执行。

## 保留边界

真实 WASAPI 原生接线已在 QA 图编译，lint 随完整门禁另验；合同使用合成源。当前SHA CI、其它宿主、
release、真实WGC/WASAPI、设备/桌面、安装器/无CRT启动、Win10/多屏/长时同步未验。
历史003fe2d release与45769c9已安装包不含本修复，旧产物和39项桌面记录原字节保持。
W53/W59/W63历史失败根因仍未明。桌面停止，无应用/设备启动、安装、推送、合入或发布。
证据：`src-tauri/target/audio-activation-boundary-contract/`。
