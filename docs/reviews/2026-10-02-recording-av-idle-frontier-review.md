# WIN-NATIVE-01 / W54 — Windows 空闲视频下界与双轨消费

需求 `REC-WINDOWS-IDLE-AV-01`；基线 `e59430f17cf0dc044ca64d9565a0ba484abc5437`，
源码 `66ceffd`；独立分支 `codex/recording-av-idle-frontier`。规格
[`2026-10-02-recording-av-idle-frontier.md`](../superpowers/specs/2026-10-02-recording-av-idle-frontier.md)
在生产修改前创建。本轮仅代码与已有 Windows 原生 CLI，桌面保持停止。

## 问题与实现

首视频帧已经有效，随后源只返回 None，旧 AV owner 一直等待下一视频 head，音频继续
入队而耗尽一秒 PCM 预算。实际 AV session/采集 worker/编码 worker 与 VP9/Opus/writer
的两个合成源用例，原实现均返回实际 `Pipeline(Backpressure)`，并非设备故障测量。

现在帧源可明确声明未来帧时间下界；默认返回 None 的其它源仍沿用原等待。Windows
WGC 桥线程顺序完成 receive、同一会话时钟采样和 replace，接收超时后采样的下界不早于
后续帧；缓存旧 native frame 限制可交付下界，恢复时使用原 minimum 过滤。原始采集
时间戳未重写。pipeline 合并一个标量，排真实帧优先，终态优先于剩余元数据，拒绝
早于已声明下界的运行期帧/控制时间；暂停期间旧帧与下界仍忽略，不推进原生时钟。

编码 owner 只填补下界所在 CFR slot 之前的已封闭 slot，在 sample 域拆分 PCM。
下界不建立 epoch、不计 captured/input、不改 native last presentation，下一 slot
仍可由真实帧替换。每次空闲步骤至多推进一 slot；周期提交共用 CFR 与 PCM 切点，
保留编码占位图像，复用原分段完成、pre-skip/padding 和关键帧路径。1 FPS 等待期间
每 50 ms 只查询下界元数据，真实帧采集仍保留原帧率间隔；暂停停止轮询。
三帧、三十二包、一秒 PCM 预算及严格 packet reader/mux 保护不变。

## 阶段证据

初始运行期红 0/2、native exit 101；首轮领域绿 343/0。扩大到二十三个新合同后，
第二轮 361/3：一处实际 writer 在 1 FPS 同 slot 输入已带入 PCM 时，将游标补回
slot 起点导致 InvalidTimeline。空闲步骤改为只向前 padding，旋转仍要求准确共享
sample 切点。另两处是新夹具的错误预期：journal 原合同先提交清单再提升 partial，
严格读取要等已声明文件提升；worker Drop 的原取消/Stop 竞态允许任一关闭终态，
核心断言仍要求 source 析构、线程 join 与 pipeline 关闭。原测试和期限未修改。

首次最终旧 merge 重放两个均失败，其中一个先观察到完成通道断开而遮住 worker 原错。
新夹具在 sender 随源析构断开时 join worker，避免 is_finished 发布竞态；该阶段原日志
与恢复记录保留。最终再次重放 0/2，两项都取得实际 Pipeline(Backpressure)，源码恢复。

第三轮领域 364/0：原 341 与新二十三项在同一图中，不重复累加。新十三项在默认和 QA，
十项只在 QA；其中两个运行期用例用实际采集/编码 worker 观察原背压，其余二十一项
覆盖新 API 和行为边界。七个原完整测试模块、原 WGC 桥线程测试保留核对正在进行。
最终同夹具旧实现重放、恢复后的最终领域与冻结 SHA 完整门禁正在验证，未计为完成。
证据在 `src-tauri/target/recording-av-idle-frontier-contract/`；最终 audit 将单列原 merge
可编译的两个运行期回归和新下界 API 合同，不把 API 用例伪称为旧实现执行过。

## 未完成项

当前 SHA 三宿主/codec CI、其它宿主原生门禁、实际 WGC/WASAPI 空闲/暂停/长时同步、
安装器与无 CRT 启动、Windows 10/混合 DPI 多屏/负坐标、发布均未验。Linux/macOS
尚不声明空闲源下界，不能因共享图通过称为这些宿主的静态双轨录屏已验。
本 SHA release 未构建，安装的旧 `45769c9` 和历史 QA `1c66112` 不含本修复。
W53 原 AVI 周期提交用例的一次 30 秒超时原因仍未定位，重跑通过不代表已解决。
panic 只覆盖 unwind，release panic=abort 不承诺 Drop 回收。旧桌面记录保持
2 pass / 1 fail / 36 not_run，整体目标未完成。
