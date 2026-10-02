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

第三轮领域 364/0；最终恢复后领域同样 364/0，原 341 和新二十三项不重复累加。
最终同字节两个运行期夹具与严格读取 helper 在九个原 Git blob 的 merge 上重放为
0 passed / 2 failed，native exit 101，均实际 Backpressure。为编译相同 trait override
只加未被调用、返回 None 的默认 API 声明；其余二十一项新合同只在绿色图执行，
不伪称为旧实现回归。finally 逐字节恢复，红绿输入与 stdout/stderr 哈希保留。
七个原完整测试模块与原 WGC 桥线程测试正文保持；严格 packet reader、mux、其它
平台源、共享时钟、音频预算/worker、三帧与三十二包预算未修改。

干净源码 `6518661490519739a472ef9659fbfb966f4340ae` 完整 Windows 门禁 child/terminal exit 0，
33 passed / 0 failed / 1 Linux smoke skipped，结束 checkout 干净。默认 Rust 1227 /
QA 1330 各 5 ignored，两图重叠；新十三项在两图，十项仅 QA，领域 364 已在 QA 内。
前端 81 文件/1403 passed；file/Git 核对不计测试通过。`COMMITTED-CONTRACT-AUDIT.json`
将十七份 Rust 输入绑定干净 SHA，后继仅五份 Markdown，生产/测试保持。
证据在 `src-tauri/target/recording-av-idle-frontier-contract/`：原 W53 状态/报告，
初始红、阶段失败/绿、最终 frozen-red 与逐字节恢复、final-green、合同/源码绑定。
完整门禁 `recording-av-idle-frontier-native-qa-6518661/RESULT.json` 保存原日志哈希；
根状态/报告与 `REVIEW-CLOSURE.json` 保留旧桌面/安装/产物身份。

## 未完成项

当前 SHA 三宿主/codec CI、其它宿主原生门禁、实际 WGC/WASAPI 空闲/暂停/长时同步、
安装器与无 CRT 启动、Windows 10/混合 DPI 多屏/负坐标、发布均未验。Linux/macOS
尚不声明空闲源下界，不能因共享图通过称为这些宿主的静态双轨录屏已验。
本 SHA release 未构建，安装的旧 `45769c9` 和历史 QA `1c66112` 不含本修复。
W53 原 AVI 周期提交用例的一次 30 秒超时原因仍未定位，重跑通过不代表已解决。
低 FPS 下尚有未交付 WGC 帧时，缓存下界、采集节流和 PCM 预算的组合仍需独立回归；
本轮缓存顺序与空闲 session 合同分别覆盖，不冒称该组合的长期运行已验。
panic 只覆盖 unwind，release panic=abort 不承诺 Drop 回收。旧桌面记录保持
2 pass / 1 fail / 36 not_run，整体目标未完成。
