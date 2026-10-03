# WIN-NATIVE-01 — 最新分支 Windows 审查与整改

日期：2026-10-01；状态：in_progress。

## Goal

以最新功能分支为基线，在 Windows 复核现有核心功能及非默认录屏 QA 能力，修复可复现问题，
补齐本机验证入口，并建立可追踪的原生编译、安装包和桌面验收任务。

## 当前续审状态

W79：按原W77日志修正当前门禁汇总的旧02005c9源码/日志关联，规范绑定d05cd3e；49项原成功日志及同阶段用例、1428份Rust正文/111个前端调用保留性核对完成，比较不算测试。原8R/9AC/47任务和49子规范全部保留；源码/测试未改。见 [W79当前审查](../../reviews/2026-10-03-windows-current-review-inventory.md)。当前同SHA CI/真实QA/其它宿主仍未验，下一步审查未来失败的诊断文件保留。

W78 / WIN-NATIVE-01：同源码d05cd3e默认/QA release编译、743编译输入与1064门禁输入、实际优化参数/feature、PE/CRT文件核对通过。新源码/测试/修复0，49修复/48历史和原门禁计数保持；构建/文件不计测试。当前同SHA CI、原生设备/桌面/其它宿主/安装器仍未验。见 [W78 release](../../reviews/2026-10-02-windows-audio-catalog-order-release-review.md)。

W77 / `REC-AUDIO-CATALOG-ORDER-01`：修复迟到的旧设备枚举覆盖启动重试目录。源码 `d05cd3e478934722273a33fb88c841648aeb1ef5` 在枚举前预留身份，仅当前查询可发布一次。原API诊断0/1，新API合同16/0、前端定向63/0；原完整模块/期限保持。干净Windows完整门禁33/0/1，默认1324/QA1475各5ignored、前端81/1405，新10项已含总数。累计49修复/48历史；当前release/同SHA CI、设备/桌面/其它宿主等仍未验。见 [W77审查](../../reviews/2026-10-02-audio-catalog-refresh-order-review.md)。

W76 / WIN-NATIVE-01：同源码默认/QA release编译及742输入、真实优化参数、两图feature、PE/CRT文件验证完成；未启动应用/设备。生产/测试/期限、48项修复和原门禁计数保持，构建/文件不计测试。当前SHA CI/其它宿主/真机/安装器等仍未验。见 [W76 release](../../reviews/2026-10-02-windows-mixed-stop-ready-bound-release-review.md)。

W75 / `REC-MIXED-STOP-READY-BOUND-01`：源码 `2bb12f9eeb04f98217189c197ecafef83b326604` 有界分批排出混音停止尾部，worker排空后封尾、后续批错误继续拒绝。原API2/7、完整领域501/0；原122份录屏文件旧模块/期限保持。干净Windows完整门禁33/0/1，默认1316/QA1467各5ignored、前端81/1403；合成codec文件保持60ms/2880有效PCM。累计48项本机修复；当前源码release/同SHA CI及设备/桌面/其它宿主仍未验。见 [W75审查](../../reviews/2026-10-02-mixed-stop-ready-bound-review.md)。

W74：47项修复和原八条需求/九条AC更新inventory；源码02005c9已有本机完整门禁和默认/QA release文件验证。当前GitHub提交不可取得，同SHA CI/真机/其它宿主仍未验。活跃混音Stop ready积累为待原API复现的静态疑点。见 [W74 inventory](../../reviews/2026-10-02-windows-readiness-inventory.md)。本轮无源码/测试改动、新通过数0。

W73 / `WIN-NATIVE-01`：冻结 `02005c98ee6ac52fd46e20c878a717f314fd90cc`，默认/录屏QA在独立target完成release编译，
实际编译参数、两图feature、AMD64 GUI PE和QA10份CRT来源/部署/导入核对，native/包装器/终端exit0。
五个核心文件只读续审未确认新缺陷；生产/测试/期限、47项本地修复和原门禁计数保持，构建/文件不计测试。
当前SHA CI/其它宿主/设备/桌面/安装器/无CRT/长时同步仍未验；历史失败根因仍未明。
旧release/安装包/39项桌面记录保持；桌面停止，无推送/PR/合入/发布。见 [W73审查](../../reviews/2026-10-02-windows-mixed-paused-stop-release-review.md)。

W72 / `REC-MIXED-PAUSED-STOP-01`：原Pause→Stop制造暂停静音，worker/完整owner报AlreadyPaused，
原十二项同字节API基线6/6。修复暂停状态只封闭停止控制时间线，真实尾块/源失败/倒退/格式错误仍拒绝，
原worker保护、W69/W70活跃路径和119份旧文件完整测试/期限保持。领域491/0，同十二项通过。
原完整owner正常complete20ms/2视频帧/960输入及有效PCM，文件解码保留已接受信号，不计设备验收。
干净02005c9完整Windows门禁33/0/1 Linux smoke skipped；默认1308/QA1457各5ignored，前端81/1403。
新12项已含总数（10两图/2仅QA），累计47项本地修复。当前release/CI/其它宿主/设备/桌面/安装器/长时同步未验；
W71 B0 release与安装包/39项桌面记录保留为历史，历史失败根因仍未明；桌面停止，无推送/PR/合入/发布。
见 [W72审查](../../reviews/2026-10-02-mixed-paused-stop-review.md)。

W71 / `WIN-NATIVE-01`：冻结 `b0b51f1196124383b9fc77cfc4ad22c21e8c3d84`，默认/录屏QA在独立target完成release编译，
实际编译参数、两图feature、AMD64 GUI PE和QA10份CRT来源/部署/导入文件核对，native/包装器/终端退出0。
包含W69/W70混音修复；生产/测试/期限、46项本地修复、原门禁/测试计数保持，构建/文件检查不计通过。
当前SHA CI/其它宿主/设备/桌面/安装器/无CRT/长时同步仍未验；历史失败根因保持未明。
旧release/安装包/39项桌面记录保持；桌面停止，无推送/PR/合入/发布。见 [W71审查](../../reviews/2026-10-02-windows-mixed-control-release-review.md)。

W70 / `REC-MIXED-CONTROL-SKEW-01`：两路恢复/Stop时刻不同时，原九项同字节API基线1/8；
恢复拒绝较早PCM，正常尾部文件却丢较晚信号。修复从较早下界恢复，分别验证Stop后完整排出共同末尾，
原生PTS/样本/增益、Exact与真实早停保护保持。同九项通过，领域479/0，116份原文件/旧模块/期限保持。
原完整owner尾部40ms/1920输入PCM、恢复60ms/2880输入PCM，实际文件解码保留预期信号，不计设备验收。
干净b0b51f1完整Windows门禁33/0/1 Linux smoke skipped；默认1298/QA1445各5ignored，前端81/1403。
新9项已含总数（7两图/2仅QA），累计46项本地修复。当前release/CI/其它宿主/设备/桌面/安装器/长时同步未验；
历史失败根因保持未明，安装包与39项桌面记录字节保持；桌面停止，无推送/PR/合入/发布。
见 [W70审查](../../reviews/2026-10-02-mixed-control-skew-review.md)。

W69 / `REC-MIXED-FRAME-BOUNDARY-01`：原混音适配器十项同字节API基线2/8，恢复首块/尾部控制失败及interrupted文件保存。
保留原生PTS/PCM与输入网格，派生输出/控制边界一致，原生控制倒退先拒绝，Exact/公共暂停/预算保持。
同十项通过，加一项取整中点保护；领域470/0（原459+新11），原完整模块/期限保持。
原worker641帧全接收、两源Drop；原双轨complete20ms/2视频帧/960有效PCM，独立文件核对不计设备验收。
干净4bc1977完整Windows门禁33/0/1 Linux smoke skipped；默认1291/QA1436各5ignored，前端81/1403。
新11项已含总数，10两图/1仅QA；累计45项本地修复，历史失败根因保持未明。
当前release/CI/其它宿主/设备/桌面/安装器/长时同步未验；W68 release属前置源码，安装包/桌面字节保持。
桌面停止，无推送/PR/合入/发布；见 [W69审查](../../reviews/2026-10-02-mixed-audio-frame-boundary-review.md)。

W68 / `WIN-NATIVE-01`：冻结 `aac5e0a728d46adc7bd7603188f41b9380138650`，两个新独立target完成默认/录屏QA release编译，
实际编译参数、两图feature、AMD64 GUI PE与QA10份CRT来源/部署/导入文件核对，native/包装器/终端退出0。
包含W67 QPC精度修复；生产/测试/期限保持，44项本地修复和原门禁/测试计数不增。
旧release/安装/桌面字节保持；编译/文件检查不计测试通过。当前SHA CI/其它宿主/设备/桌面/安装器/长时同步未验。
W67初版os error 5与W53/W59/W63历史失败根因保留未明；合成source和codec/文件不计设备验收。
桌面停止，无推送/PR/合入/发布；见 [W68审查](../../reviews/2026-10-02-windows-qpc-precision-release-review.md)。

W67 / `REC-WASAPI-QPC-PRECISION-01`：原API合成诊断4/0确认100ns量化与PCM时长的33ns表观重叠会中止worker/会话。
Windows源声明精度，经平台包装与worker配置到队列，只对齐最多100ns的媒体差；原PTS/PCM/公共暂停保持。
Exact与混音输出仍严格；新API绿色保护独立记录，原诊断继续拒绝，不能声称原API接受路径红绿证明。
领域459/0=442+新17（15两图/2仅QA）；完整合成双轨complete 40ms/4帧/1920有效PCM，实际文件独立核对。
八个旧完整模块/期限保持。干净 `aac5e0a728d46adc7bd7603188f41b9380138650` 完整Windows门禁33 passed / 0 failed / 1 Linux smoke skipped；
默认Rust1281/QA1425各5ignored，前端81文件/1403passed；两图重叠，新项已含总数。累计44项本地修复。
W67时当前SHA release/CI/其它宿主/设备/桌面/安装器/时钟漂移/长时同步未验，历史失败根因保持未明。
初版门禁32/1/1，旧分段测试清单写入os error 5；同SHA隔离1/0与完整复查33/0/1不证明首次根因，原日志保持。
桌面停止，默认录屏仍关闭，无推送/PR/合入/发布。见 [W67审查](../../reviews/2026-10-02-wasapi-qpc-precision-review.md)。

W66 / `WIN-NATIVE-01`：冻结 `56d750c8b482108b15ebb5dd2787e418bdceecb6`，两个新独立target完成默认/录屏QA
release编译；实际主程序参数、两图feature、AMD64 GUI PE与QA10份CRT来源/部署/导入
文件闭包核对，native child/包装器/终端均退出0。包含W65音频激活边界与控制时钟修复；
生产/测试/期限未改，43项修复及原门禁/测试计数不增。旧release/安装/桌面字节保持。
编译/文件检查不计测试通过；当前SHA CI/其它宿主/设备/桌面/安装器/多屏/长时同步未验。
历史W53/W59/W63运行期失败根因保持未明。见 [W66审查](../../reviews/2026-10-02-windows-audio-activation-release-review.md)。

W65 / `REC-AUDIO-ACTIVATION-BOUNDARY-01`：恢复时刻是首PCM的包含下界，成功入队后恢复严格重复拒绝；
Windows控制时刻增加状态和checked 1ns排序，原生PCM PTS不改。原七夹具2/5，修复录屏
领域442/0=429+新13（12两图/1仅QA，含6新API绿色保护）；原完整会话从边界错误/
interrupted变为complete 280ms/28帧/13440PCM，独立ffprobe文件核对，源是合成。
五个旧完整测试模块/期限保持；新增夹具重复定义曾编译失败，已修正并保留日志。
干净 `56d750c8b482108b15ebb5dd2787e418bdceecb6` 完整Windows门禁33 passed / 0 failed / 1 Linux smoke skipped；
默认Rust1266/QA1408各5ignored，两图重叠不累加；前端81文件/1403passed。
新增13项实际运行次数为12两图/1仅QA，录屏领域/新项已含总数。W65时当前release/CI/其它宿主/设备/桌面及历史失败根因保留未验。
见 [W65审查](../../reviews/2026-10-02-recording-audio-activation-boundary-review.md)。

W64 / `WIN-NATIVE-01`：冻结 `003fe2dc642627b9c6a071bf629be89342973941`，默认/录屏QA的新独立target
release编译、实际主程序参数、两图feature、AMD64 GUI PE及QA10份CRT来源/部署/导入
文件闭包核对，native child/包装器/终端均退出0。包含W63公共暂停/零交集修复；生产/测试
保持，42项修复及W63门禁/测试计数不增。旧release/安装/桌面字节保持。
编译/文件检查不计测试通过；设备/桌面/CI/其它宿主/安装器/多屏/长时同步仍未验。
W53/W59和W63初版分段超时根因继续未明。见 [W64审查](../../reviews/2026-10-02-windows-av-timeline-release-review.md)。

W63 / `REC-AV-CONTROL-TIMELINE-01`：原双轨各自扣时导致不同控制耗时错位；统一
扣两源共同暂停区间，保留源恢复下界与真实音频空洞。相同八项原API夹具原2/6，
修复录屏领域429/0=原415+新14（八原API红绿/六后增保护，全部仅QA）。原完整会话
实际WebM从560 ms/56帧变为580 ms/58帧，独立ffprobe/PCM核对，源是合成。
六个原完整测试模块/期限保持；零交集组件提案原0/2、修正后2/0，保留原源下界。
初版门禁32/1/1，两旧分段用例5秒超时；同SHA隔离4/0，根因未明。最终干净 `003fe2dc642627b9c6a071bf629be89342973941` 完整Windows门禁33 passed / 0 failed / 1 Linux smoke skipped；
默认Rust1254/QA1395各5ignored，两图重叠不累加；前端81文件/1403passed。
新增14项在QA图各实际运行一次，领域/新项已含总数，六个原完整测试模块与期限保持。
源码改动后7579222的release属历史。
当前CI/其它宿主/release/设备/桌面/安装器/多屏/长时同步及两个历史失败原因保留未验。
见 [W63审查](../../reviews/2026-10-02-recording-av-control-timeline-review.md)。

W62 / `WIN-NATIVE-01`：冻结 `75792220cea374dd2f1e6526122be30dad328e6a`，默认与录屏 QA 在新独立 target
完成 release 编译；两次 native child/包装器/终端退出码均为 0。实际 release 参数、两图
feature、AMD64 GUI PE、QA 10 份 CRT 的来源/部署/导入文件闭包核对。包含 W59/W61 修复，
生产/测试未改；41 项修复和 W61 门禁/测试计数保持。本轮编译与文件核对不计测试通过。
旧 release/安装/桌面字节保持；当前 CI、其它宿主、设备/桌面/安装器/Win10/多屏、长时同步
仍未验，两个历史失败根因仍未知。见 [W62审查](../../reviews/2026-10-02-windows-control-release-review.md)。

W61 / `REC-VIDEO-CONTROL-FAILURE-01`：源已pause/resume后，pipeline拒绝必须让worker
保留原错并退出；不改变source前的非致命状态拒绝。原实现八条运行期合同0/8，两个原owner
Stop成功的实际complete清单/媒体保留；同字节修复领域415/0=原407+新8（7两图/1仅QA）。
单轨/双轨实际interrupted清单、源析构/前缀/原错误与原完整测试模块核对；干净 `75792220cea374dd2f1e6526122be30dad328e6a` 完整Windows门禁33 passed / 0 failed / 1 Linux smoke skipped；
默认Rust1254/QA1381各5ignored，两图重叠不累加；前端81文件/1403passed。
八项新合同实际次数为7两图/1仅QA，已含Rust总数；原完整worker与前置状态合同保持。
设备/桌面、当前CI、其它宿主、长时同步/安装器/多屏/release及两个历史失败根因保留未验。
见 [W61审查](../../reviews/2026-10-02-recording-video-control-failure-review.md)。

W60 / `REC-OPUS-EBML-VERIFY-01`：仅修复 Opus 测试读取器的裸字节搜索；按父路径/尺寸
读取四个元数据字段，生产代码与原测试函数/断言保持。同字节六项夹具原1/5、修复领域407/0
=原401+新6（仅QA）；五份同字节真实媒体保留，原严格读取器与独立EBML/ffprobe全包哈希核对。
W59那次失败无媒体，不能宣称已归因；W53超时也保留。干净 `a5e03f8d075cd09023a824e26d51ff80f9466014` 完整Windows门禁33 passed / 0 failed / 1 Linux smoke skipped；
默认Rust1247/QA1373各5ignored，两图重叠不累加；前端81文件/1403passed。
新六项各在QA图实际运行一次，已含在总数；原双轨合同、生产代码与旧期限保持。
生产修复仍40。
桌面/设备、当前CI、其它宿主、安装器/多屏/release继续未验。
见 [W60审查](../../reviews/2026-10-02-recording-opus-ebml-verification-review.md)。

W59 / `REC-VIDEO-CONTROL-PREFLIGHT-01`：首帧前/重复暂停和未暂停恢复在调用平台源前拒绝，
防止失败命令停止推送流或清除缓存；无音频MJPEG/VP9首帧与文件提交恢复。
同字节旧API对照2/6、修复领域401/0，新增8（七两图/一仅QA）；旧测试/期限保持。
干净 `4eb65d8218c22e9909ab7dd9d5d16c59c548a4a4` 完整Windows门禁33 passed / 0 failed / 1 Linux smoke skipped；
默认Rust1247/QA1367各5ignored，两图重叠不累加；前端81文件/1403passed。
新7两图/新1仅QA的实际运行次数核对，已含在Rust总数；原source错误与旧控制合同保持。
桌面停止，本SHA release/当前CI/其它宿主/设备/安装器/多屏未验；W53 AVI与旧Opus
一次元数据查找失败根因均保留。见 [W59审查](../../reviews/2026-10-02-recording-video-control-preflight-review.md)。

W58 / `WIN-NATIVE-01`：固定生产 `830b12b43051efa7eebad36a3d1a4434975785b2`，默认与录屏 QA 在独立目录
完成原生 release 编译，实际 child/terminal exit0，源码/测试保持。核对实际主程序
release 参数、两图 feature、AMD64 GUI PE 与 QA 10份 CRT 的部署/导入闭包。
旧 EXE/安装/桌面记录保持；未启动/安装/打包/发布，累计39修复及旧门禁计数不增。
当前SHA CI/其它宿主/设备/安装器/无CRT启动/多屏仍未验，桌面停止，W53历史根因未知。
见 [W58审查](../../reviews/2026-10-02-current-windows-release-review.md)。

- W57 / `REC-ARTIFACT-SHARING-01`：独立 `codex/recording-artifact-sharing`，基于
  `b2f6e95`。原五个已提交产物提升点及周期session取得实际错误32；原生首轮2/6。
  复用原W56有界重试，永久锁保留已提交partial，损坏文件先验证拒绝。原权限/
  提交恢复顺序/导出/FPS/时钟/队列/旧测试与期限保持。
  同字节八项旧API夹具原实现2/6（六条实际错误32）、修复领域393/0=原385+新8。
  干净 `830b12b43051efa7eebad36a3d1a4434975785b2` 完整 Windows 门禁33 passed / 0 failed / 1 Linux smoke skipped；
  默认Rust1240/QA1359各5ignored，两图重叠不累加；前端81文件/1403passed。
  新5在默认/QA两图、新3仅QA，已含在Rust总数；两个旧完整测试模块与原权限/恢复保护保持。
  三份运行期夹具字节相同，全部使用原API，无接口stub；原retry/probe算法与导出保持，十四份Rust输入绑定干净SHA。
  旧实现重放及finally恢复、最终领域与完整门禁的实际日志/输入哈希保留。
  当前SHA CI/其它宿主/设备/安装器/多屏仍未验，桌面停止，W53 AVI历史根因未知。

- W56 / `REC-MANIFEST-SHARING-01`：独立 `codex/recording-manifest-sharing`，基于
  `9a7a6e2`。原生文件句柄短暂阻止删除共享，原 session/encoder 取得实际错误 5。
  Windows 清单对 32/33 或经删除探针确认共享拒绝的 5 有界重试；最多 21 次/500 ms。
  私有文件/ACL、提交与恢复顺序、原测试/期限/预算保持，其它保存/平台不改。
  同字节运行期旧实现0/1（实际错误5）、修复领域385/0=原377+新8。
  干净 `d7c66bba93e3a439a72a23039267a85a190f5ce1` 完整 Windows 门禁33 passed / 0 failed / 1 Linux smoke skipped；
  默认Rust1235/QA1351各5ignored，两图重叠不累加；前端81文件/1403passed。
  新8在默认/QA两图内，已含在Rust总数；两个旧完整测试模块与原权限/恢复保护保持。
  运行期夹具字节相同，无接口stub；新七项API合同仅在绿色图，十一份Rust输入绑定干净SHA。
  旧实现重放及finally恢复、最终领域与完整门禁的实际日志/输入哈希保留。
  当前 SHA CI/其它宿主/设备/安装器/多屏未验，桌面停止；W53 AVI 超时原因未知。

- W55 / `REC-PENDING-FRAME-PCM-01`：独立 `codex/recording-pending-frame-budget`，基于
  `bdffab3`。原 bridge/1 FPS/同一时钟实时 PCM 已复现实际 Backpressure；保留缓存
  下界与真实帧替换，按全局/分段剩余包槽消费未封闭 slot 的 PCM，先对齐分段切点。
  同字节运行期夹具旧实现0/1（实际Backpressure）、修复领域377/0=原364+新13。
  干净 `120b3add51e0d569cdc5e4442f0a4577c12f7b88` 完整 Windows 门禁33 passed / 0 failed / 1 Linux smoke skipped；
  默认Rust1227/QA1343各5ignored，两图不累加；前端81文件/1403passed。
  新13仅QA，已含在Rust总数内；原五个完整测试模块、Windows生产/原测试与严格reader保持。
  红绿运行期/bridge/helper字节相同，无接口stub；十八份Rust输入绑定干净SHA。
  最终旧实现重放与finally恢复、最终领域及完整门禁的实际日志/输入哈希已保留。
  当前 SHA CI/其它宿主/设备/安装器/多屏未验，桌面停止；W53 AVI 超时原因仍保留。

- W54 / `REC-WINDOWS-IDLE-AV-01`：独立 `codex/recording-av-idle-frontier`，基于 `e59430f`。
  原两个首帧后空闲 AV worker 用例均 Backpressure；按 Windows 源保证的未来帧下界
  生成已封闭 CFR slot、sample 域消费 PCM 与周期提交。缓存旧帧限制下界，pipeline
  标量合并、真实帧/终态优先；暂停停止查询，1 FPS 等待只轮询元数据，计数与预算保持。
  首轮领域 343/0，扩大后 361/3；实际同 slot 游标倒退已修复，另两处新夹具预期修正。
  最终同字节旧 merge0/2、领域364/0，新23/原341在内，七个旧模块/原桥线程测试保持。
  干净 `6518661` 完整 Windows 门禁33/0/1，默认Rust1227/QA1330各5ignored，前端81/1403，
  两图/领域不累加。其它宿主无下界仍等待，设备/当前 CI/安装器/多屏未验，桌面停止；
  W53 AVI 一次超时原因仍保留。
  下一代码项独立复现低 FPS 未交付 WGC 帧与 PCM 预算/下界节流的组合；本轮单独的
  缓存顺序和空闲 session 合同不替代组合长时验证。

- W53 / `REC-AV-CFR-SEGMENT-01`：独立 `codex/recording-av-cfr-segments`，基于 `3385507`。
  writer 分段与 PCM 拆分使用原 CFR slot 边界，同 slot 延后；timecode 量化域内排序和
  frontier 保证同时间视频在前。相邻全局 CFR 精确端点有例外，缺帧/非 CFR 起点继续拒绝。
  最终同字节旧实现 3/9、修复新十二项/原 329，共领域 341/0，六个旧模块正文保持。
  干净 `66ceffd` 完整 Windows 门禁 33/0/1（Linux skip），默认 Rust 1214/QA 1307 各 5 ignored，
  两图/领域不累加，前端 81/1403，本 SHA release 未构建；下一 head 未来时的消费等待与
  当前 CI/其它宿主/设备/安装器/多屏仍未验，桌面停止。首次完整门禁 32/1/1，
  未修改 AVI 分段用例 30 秒超时；单独 0.23 秒通过后同 SHA 完整重跑通过，原因未定位保留。

- W52 / `REC-AV-GAP-DRAIN-01`：独立 `codex/recording-av-gap-drain`，基于 `e4a4a3c`。
  最终同字节六项旧 writer/编码 worker 回归均失败，真实 `InterleaveQueueFull`；包含 1 FPS。
  原 PCM 块增量交替排空两轨，传已知下一视频下界/EOS，CFR 保留下一 slot 与真实 PTS；
  同 slot、精确停止/边界合同通过，新增十项/原 319，共领域 329，旧三个模块正文保持。
  干净 `80e084c` 完整 Windows 门禁 33/0/1（Linux smoke 跳过），默认 Rust 1214/QA 1295
  各 5 ignored，两图/领域不累加，前端 81/1403，本 SHA release 未构建。
  下一 head 未来时的消费等待、不对齐 slot 的分段映射及
  设备/其它宿主/当前 CI/安装器/多屏保持未验，桌面停止。

- W51 / `REC-FIRST-FRAME-AUDIO-01`：独立 `codex/recording-first-frame-audio-gate`，基于 `d9264cd`。
  原生产源码固定三个首帧 AV 回归全部失败，音频 worker 实际返回 Backpressure；首帧前
  十四次轮询。实现有效首帧入队后的音频释放与首轮视频握手，Windows 延迟 stream 激活，
  混音从有效 PCM 下界开始，原预算/时钟/encoder 保留。最终同字节三项原实现 0/3，
  新十八项及原领域三百零一项，共 319 passed，四个旧测试模块正文保持。干净 `2dccc43`
  完整 Windows 门禁 33/0/1，默认 Rust 1214/QA 1285 各 5 ignored，前端 81/1403；
  两图与领域不累加，本 SHA release 未构建。首帧后无新视频帧与提前 Stop 的三十二包尾部阻塞为独立未完成项，
  设备/其它宿主/当前 CI/安装器/多屏和桌面保持未验。

- W50 / `REC-AV-STARTUP-GATE-01`：独立 `codex/recording-av-startup-gate`，基于 `cc5f7af`。
  原生产入口在视频 factory 等待时已轮询音频五次；新三项红、原三项 AV 绿，修复后同六项绿。
  两个 factory 都完成后才释放 pipeline 采集，新十一项/原相关二十七项及录屏领域 301 通过。
  干净 `1c66112` 完整 Windows 门禁 33/0/1，Rust 默认 1201/QA 1267 各 5 ignored，
  前端 81/1403；隔离 unsigned/unbundled QA release、实际 payload/许可证哈希通过。
  原测试正文与默认产物保持；不扩大队列、不改原生 constructor，首视频暖机仍需独立审查。
  详见 `docs/reviews/2026-10-02-recording-av-startup-gate-review.md`，文件/构建不算测试通过。
  真实 native 缓冲/设备/当前 CI/其它宿主与桌面仍未验，原 W49 历史证据保持。

- W49 / `WIN-QA-CRT-DISCOVERY-01`：独立 `codex/windows-qa-crt-discovery`，基于 `263a2e7`。
  QA 从仅 VC143 改为已发布 v14 desktop x64 家族发现，按数字版本选最新，歧义/不可信
  新版失败关闭，原文件级校验保留。原新增 24 项红 12/12，旧五十项通过；同组绿后增加
  六个目录标签用例，共新三十/旧五十项通过。受控目录/元数据不证明实际新/旧 SDK 编译。
  干净 `ae2fdb6` 完整 Windows 门禁 33/0/1，前端 81/1403；原八十份测试 Git blob 未改。
  现有真实 VC143 十份有效签名 DLL 与隔离 unsigned/unbundled QA release 编译/payload
  核对通过，native/wrapper/terminal exit 0；默认产物、十七份基础许可证和原 W48 证据保持。
  详见 `docs/reviews/2026-10-02-windows-qa-crt-discovery-review.md`；真实 VS 2026/当前 CI/
  安装与设备/其它平台仍未验，桌面停止，文件/构建数量不算测试通过。

- W48 / `WIN-QA-CRT-01`：发现真实录屏 QA PE 导入 MSVCP140 而旧 MSI/resources 没有
  部署 CRT；默认 PE 不依赖该运行库。独立 `codex/windows-qa-runtime` 增加受验证的 SDK
  app-local CRT、direct/delay 递归闭包及构建后 payload 检查，QA 显式 target 隔离默认
  目录；默认 feature/Windows 配置/正式 release 保持。完整版本红 47/3、同五十项绿。
  干净 5d900ca 完整门禁 33/0/1，默认 Rust 1193/QA 1256（各 5 ignored），前端 80/1373；
  unsigned/unbundled QA 编译 native/wrapper/terminal 0，十份 DLL、十七份许可证及默认
  产物保持核对，原七十九份测试文件字节相同。见 [W48 审查](../../reviews/2026-10-02-windows-qa-runtime-review.md)。安装/无 CRT 系统启动、
  新 SHA CI、Windows 10/多屏/其它平台仍未验，桌面停止。

- W47 / `WIN-NATIVE-01`：[Windows release 编译审查](../../reviews/2026-10-02-windows-release-build-review.md)。
  冻结 `f5ad5da` 默认与录屏 QA release 均 native/wrapper/terminal 0；x64 GUI 未签名 PE、
  产物哈希、锁定输入、feature 指纹与实际 fat LTO/单 codegen 参数核对。panic=abort
  与测试 unwind 清理范围明确；不增加测试数。产物未启动/安装，全局 Native/CI/其它
  宿主仍未完成；下一步仅导入表与 Windows 基线/API 门控代码核对。

- W46 / `WIN-NATIVE-01`：累计 [代码证据核对](../../reviews/2026-10-02-windows-code-review-evidence.md)。
  29 个原被测提交/日志 SHA 与原阶段用例保留；616 次 Rust 正文比较为 611 原样/五个精确
  物理夹具新增字段，31 个前端测试调用保持；均不计新通过数。当前 `f5ad5da` CI 查询无 run。
  全局验收未完成，下一步仅构建/只读核对当前源码 Windows release QA 可执行文件，
  `--no-bundle --no-sign`，不启动/安装/生成证书或修改系统工具；仍不恢复桌面。

- `WIN-PASTE-CLEANUP-01` / W45：独立 `codex/windows-paste-key-cleanup`，基于 `10fd2ea`。
  V Click 错误/展开显式清理 V，首次失败 RAII 再试一次，modifier 及其 Drop 重试保持。
  正常 Click 三步与主要错误保持，持续阻塞不报成功；原协议提取红 10/6，旧六项全绿，
  同十项原字节绿 16。原实现/十二关联文件/锁定 SDK 核对；受控模型不代替真实按键。
  干净 f5ad5da 完整门禁 30/0/1（Linux skip），默认 Rust 1193/QA Rust 1256，各 5 ignored
  不累加，新十项各图通过；前端 79/1323，剪贴板 31/WGC 十八另列。实际系统/其它宿主/
  新 SHA CI/安装与 Wayland 未验，桌面停止，安装包未更新。

- `WIN-CLIP-SNAPSHOT-01` / W44：独立 `codex/windows-clipboard-snapshot`，基于 `43b400c`。
  Windows HTML/text 共用一个 guard；原解码/回退、其它平台/抑制保持。原协议提取红 27/4，
  旧 23 通过；借用修正后原协议重放 27/4，同修正八项原字节绿 31，guard/旧 parser 共 16。
  vendor 严格 lint 通过；guard/sequence 模型不是实际系统并发证据，原合同/十六文件保持。
  干净 b541e87 完整门禁 30/0/1（Linux skip），默认 Rust 1183/QA Rust 1246，各 5 ignored
  不累加，新八项各图通过；剪贴板独立 31、WGC 十八、前端 79/1323。其它宿主/新 SHA CI
  与真实系统/安装/Wayland 回归未验，桌面停止，安装包未更新。

- `WIN-SHORTCUT-SHARED-01` / W43：独立 `codex/windows-shortcut-shared`，基于 `62d5103`。
  共用动作按 ID 继承唯一键位首次结果，不能以 Shared 掩盖全失败；部分成功与判重/优先级保持。
  提取原协议红基线 9/6（旧五项全绿），五次 aggregate、一项动作错误 Ok 复现，
  同十项原字节修复后通过，共 15 passed；原合同与十九份关联文件保持。
  干净 0f793c9 完整默认/QA 门禁 child/终端 exit 0，30/0/1（Linux skip）；默认 Rust
  1175、QA Rust 1238（各 5 ignored，重叠不累加），新十项/旧五项各图通过，前端 79/1323。
  受控回调不是 OS 注册/冲突证据；实际系统/UI、其它宿主/当前 SHA CI 留未验，安装包未更新。

- `REC-LIBRARY-READY-01` / W42：独立 `codex/recording-library-ready`，基于 `a2c31d2`。
  ready 绑定本次 effect 退休标记，cleanup 后迟到加载不再显示/聚焦请求；当前正常/失败
  重试页通知一次保持。原 App 字节 jsdom 红基线 35/5（旧 32 全绿），四项旧 ready 调用
  与 StrictMode dev/test 两次调用复现；同八项原字节修复后通过，共 40 定向测试。
  旧三文件/十三份关联文件与限定 guard 外原 App 保持。干净 1907809 完整 Windows 默认/QA
  门禁 child/终端 exit 0，30 passed / 0 failed / 1 Linux smoke skipped；默认 Rust
  1165、QA Rust 1228（各 5 ignored，重叠不累加，无新 Rust 用例），前端 79/1323 含新
  八项。源码/日志/发现清单和干净检出核对；mock ready 不等于实际窗口/focus 或发布
  effect 重放，桌面/其它宿主/当前 SHA CI 留未验，安装包未更新。Tauri Shared 结果记账
  为下一步受控回调候选，尚未复现，不注册真实系统热键。

- `REC-PLAYBACK-LIFECYCLE-01` / W41：独立 `codex/recording-playback-lifecycle`，基于 `092c413`。
  播放请求身份在卸载/服务清理/预览关闭时退休，迟到租约由旧服务释放；旧失败不改新请求。
  关闭只清理该播放 busy，其它导出保持。原 App 字节红基线 27/5（旧 24 全绿），观察到
  释放缺口、播放器重开和旧 busy 阻塞；第五项未执行到旧拒绝。新八项原字节修复后通过，
  定向 3 文件 / 32 passed，旧两测试文件和十一份关联文件保持。干净 ba26a83 完整 Windows
  默认/QA 门禁 child/终端 exit 0，30 passed / 0 failed / 1 Linux smoke skipped；默认
  Rust 1165、QA Rust 1228（各 5 ignored，重叠不累加），没有新增 Rust 用例，前端
  78 文件 / 1315 passed，新增八项在前端总数内；发现清单/源码/原始日志与干净检出核对。
  替身释放/服务更换不是实际 backend 撤销/窗口重建；加载 ready 回调退休为下一步候选，
  尚未复现；桌面/其它宿主/新 SHA CI 留未验，安装包未更新。

- `REC-MEDIA-REVOKE-01` / W40：独立 `codex/recording-media-revoke`，基于 `116f1d3`。
  准备进入后台前捕获会话身份；撤销同时失效已签发与待签发租约，首块/约 16 MiB 哈希检查
  及最终同锁签发检查同一凭据。其它会话、新准备保持，最后持有者 Drop 回收弱条目。
  原生产 manager 红基线 7 passed / 4 failed，四项实际重新签发 Ok 租约；同组六正文保持，
  类型/API adapter 迁移，追加两项生命周期/跨 manager 测试，含旧五项共 13 passed。
  原读取/校验/响应/租约断言与十份关联文件核对。干净 6944b68 完整 Windows 默认/QA
  门禁原生 child/终端 exit 0，30 passed / 0 failed / 1 Linux smoke skipped；默认 Rust
  1165、QA Rust 1228（各 5 ignored，两图重叠不累加），新八项与旧媒体五项各图通过；
  前端 77 文件 / 1307 passed，源码/日志与干净检出核对。真实 IPC/删除/合并/播放、
  哈希中间撤销、其它宿主和当前 SHA CI 未验；前端迟到 Promise/卸载释放先留生产组件
  复现任务，尚未追加缺陷结论。安装包未更新，桌面操作停止。

- `WIN-EXPORT-IDENTITY-01` / W39：独立 `codex/windows-export-identity`，基于 `935766b`。
  Windows 在临时复制前按卷号/128 位 FileId 拒绝同文件别名；原新建、不同目标和哈希协议保持。
  原生产导出直接红基线 2 passed / 4 failed（三者迟至替换 error 5、硬链接返回 Ok），同组六项
  修复后通过，新测试字节保持；既有导出合同与一项原生无效句柄错误通过。干净 69cf0b4 完整
  默认/QA 门禁原生子进程 exit 0，30 passed / 0 failed / 1 Linux smoke skipped；默认 Rust
  1157、QA Rust 1220（各 5 ignored，重叠不累加），新增六项与错误一项在各图总数内，前端
  77 文件 / 1307 passed，源码/日志与干净检出核对。正常文件的身份/权限错误、其它文件系统/
  网络盘、外部竞态和实际对话框未验；pending 播放租约跨撤销仍待生产夹具复现，
  不将夹具当作真实录屏/播放；当前 SHA CI/其它宿主仍未验，桌面操作停止。

- `REC-DELETE-OWNER-01` / W38：独立 `codex/recording-delete-owner`，基于 `ab13a3a`。
  VP9/QA 图内在同一 registry 原子取得合并/删除所有权，删除 guard 覆盖原 worker，冲突前不
  撤销播放/缓存或删除文件；同会话互斥，其它会话操作及全局单合并保持，默认删除路径不变。
  提取原无所有权删除协议 MSVC 红基线 3 passed / 3 failed，生产 VP9 已提交文件删除缺口
  复现；同组六项修复后通过，新测试字节保持，既有单槽测试红/绿通过。干净 0321355 完整
  默认/QA 门禁确认原生子进程 exit 0，30 passed / 0 failed / 1 Linux smoke skipped。默认 Rust
  1150、QA Rust 1213（各 5 ignored，重叠不累加），新增六项仅在 QA 总数内；前端 77 文件 /
  1307 passed。源码/日志与干净检出核对；真实窗口重开/强杀/录屏/播放、其它宿主和当前 SHA CI
  未验，桌面操作停止。Windows 导出路径别名仍待生产入口复现。

- `WIN-CONTROL-ROLLBACK-01` / W37：独立 `codex/windows-control-rollback`，基于 `7ca01a0`。
  准备失败回滚按销毁请求返回结果结算，与普通关闭共用路径；请求失败阻止替换，原启动错误和旧
  session/caller/正常关闭保持。提取原协议 MSVC 红基线 22 passed / 2 failed（旧十七项全绿，
  新七项红 5/2），同组二十四项修复后通过。干净 a52ecaa 完整默认/QA 门禁确认原生子进程
  exit 0，30 passed / 0 failed / 1 Linux smoke skipped。默认 Rust 1150、QA Rust 1207（各
  5 ignored，重叠不累加），新增七项每图在总数内；前端 77 文件 / 1307 passed，源码/日志与
  干净检出核对。请求发送成功不代表原生销毁完成；真实请求失败/窗口/捕获像素、设备/停止/
  恢复、其它宿主与当前 SHA CI 未验，桌面和录屏默认门控保持。恢复/merge-delete/导出路径
  身份续审尚未复现新缺陷。

- `WIN-MAIN-TARGET-01` / W36：独立 `codex/windows-main-window-target`，基于 `301c782`。
  主窗口已有保存目标时不执行后备原生查询；必要后备及保存查询的原错误保持，几何/配置/焦点
  和 debounce 不变。提取原 eager 协议 MSVC 红基线 11 passed / 2 failed（旧七项全绿，新六项
  为 4 passed / 2 failed），同组十三项修复后通过；干净 c9d5512 完整默认/QA 门禁确认原生
  子进程 exit 0，30 passed / 0 failed / 1 Linux smoke skipped。默认 Rust 1143、QA Rust 1200
  （各 5 ignored，重叠不累加），新增六项每图在总数内，前端 77 文件 / 1307 passed；源码/日志
  哈希与干净检出核对。纯数据/closure 不计真实窗口或原生错误证据；多屏/DPI、其它宿主、
  当前 SHA CI 和桌面仍未验，原生 DPI 消息/约束时序没有确认缺陷。

- `WIN-WGC-BRIDGE-ROLLBACK-01` / W35：独立 `codex/windows-wgc-bridge-rollback`，基于 `f8409b7`。
  应用帧桥在 start 错误和正常销毁时取消并 join，接收循环不依赖全部 sender 被释放；成功转移、
  原错误、帧/时钟与暂停恢复语义保持。提取旧协议 MSVC 红基线 3 passed / 4 failed，同组七项
  真实线程合同全绿；干净 7922457 完整默认/QA 门禁确认原生子进程 exit 0，30 passed / 0 failed /
  1 Linux smoke skipped。默认 Rust 1137、QA Rust 1194（各 5 ignored，重叠不累加），新增七项
  每图在总数内；前端 77 文件 / 1307 passed，源码/日志哈希与干净检出已核对。真实 WGC/Close 时限、系统释放、
  设备/硬件矩阵、当前 SHA CI 与其它宿主未验；vendor、默认 feature 和桌面停止边界保持。

- `WIN-PIN-LIVE-DPI-01` / W34：独立 `codex/windows-pin-live-dpi`，基于 `c6075e2`。
  Windows 渲染由当前窗口原生 DPI 驱动，首读使用 caller-bound 只读业务命令，既有 core 权限
  与来源/展示元数据保持；先订阅事件，旧查询或 payload 不覆盖实时比例，未知回退 auto。
  原 App 十二项红基线 2 passed / 10 failed，同组全绿；三项 API 适配器、六项 MSVC 纯读取
  合同、既有 Pin/权限回归与 TS 检查通过。干净 e339042 完整默认/QA 门禁确认原生子进程 exit 0，
  30 passed / 0 failed / 1 Linux smoke skipped；默认 Rust 1130、QA Rust 1187（各 5 ignored，
  重叠不累加），新增六项每图在总数内，前端 77 文件 / 1307 passed。真实 DPI 与 WebView2 像素、
  Windows 10/多屏、当前 SHA CI 和其它宿主未验；源码/日志哈希与干净检出已核对。
  WGC 续审确认现有 monitor-local crop 合同和 prepare/connect 的原生 descriptor 复核；
  未把冻结到 prepare 的原点变化定为缺陷或修改它。运行中热插拔/身份重用仍未验。

- `WIN-PIN-WORKAREA-01` / W33：独立 `codex/windows-pin-workarea`，基于 `7017562`。
  原生 owner/物理交集驱动保存、保留原生目标的恢复和客户区工具条边界；旧存储格式与正常旧记录
  相对位置/展示参数保留。提取旧生产协议红基线 1 passed / 15 failed，同组十六项 MSVC 回归通过；
  干净源码 4a58101 完整默认/QA 门禁确认原生子进程 exit 0，30 passed / 0 failed / 1 Linux smoke skipped；
  默认 Rust 1124、QA Rust 1181（各 5 ignored，重叠不累加），前端 1292 passed；十六项包含在各图。
  旧错误来源不能反推；真实 API/窗口/DPI/热插拔、WGC 与新 SHA CI 未验。

- `WIN-PIN-ORIGIN-01` / W32：独立 `codex/windows-pin-physical-origin`，基于 `e1c7191`。
  冻结物理来源贯穿普通/长截图、输出重试和像素登记；新建图片 Pin 按一次原生快照和 PNG 像素
  规划，创建/reveal 保留物理请求，Windows resize 不重复逻辑定位，来源失效按光标/主屏回退。
  旧生产输出及提取的旧布局/请求协议红基线 3 passed / 15 failed，同组十八项 MSVC 离线回归通过；
  干净源码 888127a 完整默认/QA 门禁确认原生子进程 exit 0，30 passed / 0 failed / 1 Linux smoke skipped；
  默认 Rust 1108、QA Rust 1165（各 5 ignored，重叠不累加），前端 1292 passed；十八项包含在各图。
  工作区迁移、工具条交集、WGC、真实多屏/DPI、新 SHA CI 与其它宿主未验。

用户最新要求停止桌面操控，先进行代码 review 与修复。桌面操作保持停止；本机仅执行原生
编译、自动合同和文件证据核对，Linux/WSL 未启动。

- `WIN-PIN-TOOLBAR-01`：独立 `codex/windows-pin-toolbar-height`，修复源码 `7aa6cf6` 的完整
  Windows 默认/录屏 QA 门禁 exit 0，23 passed / 0 failed / 1 skipped；修复后桌面复测未运行。
- `WIN-PRIVATE-WRITE-01`：独立 `codex/windows-private-write-order`，基于 Pin 修复后的 `aefc413`。
  修复源码 `f788b1f57d852b7df87e334b579c3224c7fd2543` 的完整 Windows 默认/录屏 QA 门禁 exit 0，
  23 passed / 0 failed / 1 skipped（Linux smoke）；默认 Rust 1042、QA Rust 1095、前端 1292 passed。
  两组 Rust 重叠且各有 5 ignored，不累加；新 SHA 原生 CI 尚未执行。
- `WIN-LONGSHOT-CURSOR-01`：独立 `codex/windows-longshot-cursor-restore`，基于 `1cd09c8`。
  提前失败的生产恢复 guard 红基线 1 passed / 3 failed，修复后八项定向合同通过；源码
  `d8dff808e320fd840376e2acec396887e6bbc3ce` 的完整 Windows 默认/录屏 QA 门禁 exit 0，
  23 passed / 0 failed / 1 skipped；默认 Rust 1046、QA Rust 1099、前端 1292 passed。
  两组 Rust 重叠且各有 5 ignored，不累加。新增回归使用注入指针接口；真实桌面接管及新 SHA CI 未验。
- `WIN-CF-HTML-01`：独立 `codex/windows-cf-html-bounds`，基于 `46e3fdc`。Windows HTML 读取关闭
  clipboard-win 5.4.1 未受缓冲区边界约束的复制调用，改为实际字节和安全片段解析；离线红基线
  2 passed / 7 failed，修复后九项通过。源码 `50b7778ec9e4bd52fa31aa657be607877c4990ef` 完整
  Windows 默认/录屏 QA 门禁 exit 0，24 passed / 0 failed / 1 skipped，包含新增依赖库定向组。
  默认 Rust 1046、QA Rust 1099、前端 1292 passed；Rust 重叠且各有 5 ignored，不累加。
  Windows Native CI 已添加入口，但远程新 SHA CI、真实富文本互操作未验，未执行原生畸形复制。
- `WIN-CLIP-IMAGE-BUDGET-01`：独立 `codex/windows-image-decode-budget`，基于 `cf59157`。
  将 watcher 的尺寸预算前移到 Windows PNG / DIB 像素解码前；四字节故障注入红基线
  3 passed / 2 failed，修复后七项预算合同通过。531d791 本机完整默认/QA 门禁 exit 0，
  25 passed / 0 failed / 1 skipped；CI 定向入口已接线但远程未运行。
  当时扩展图片组 9 passed / 1 failed，Chrome DIB 在原 cf59157 源码也失败；后来由下项 W22 独立修复。
- `WIN-DIBV5-PIXEL-01`：独立 `codex/windows-dibv5-pixel-offset`，基于 `6069cce`。
  W22 红基线 2 passed / 2 failed：原 Chrome 读取失败及带尾部数据的成功错图；改用借用原数据的
  BMP 文件视图提供显式像素偏移。原 Chrome/Firefox 逐像素断言保留，五项 DIB 和三项文件视图
  合同通过；连同预算和富文本 24 项 Windows 离线合同通过。源码 25fb5d7159af66a88d829eda199efa649698633d
  完整本机默认/QA 门禁 exit 0，27 passed / 0 failed / 1 skipped；默认 Rust 1046、QA Rust 1099，
  两图重叠且各有 5 ignored，不累加；前端 75 文件 / 1292 passed，CI 已接线未远程运行，桌面未验。
- `WIN-PASTE-RECHECK-01`：独立 `codex/windows-paste-input-recheck`，基于 `949a2d9`。
  Windows 输入后端初始化后、首次按键前再查窗口/PID/前台；初始化期间目标变化的离线红基线
  2 passed / 4 failed，修复后六项通过，未调用 Enigo 或窗口/输入 API。干净源码
  `14bf61630efab7b62905bbc5b976fbed8e62166c` 完整 Windows 默认/QA 门禁 exit 0，
  27 passed / 0 failed / 1 skipped；默认 Rust 1052、QA Rust 1105（各 5 ignored，重叠不累加），
  前端 75 文件 / 1292 passed。六项粘贴合同已包含在两个 Rust 图，日志哈希已核对；
  最后复核后的系统竞争、真实用户接管、新 SHA CI 与 macOS 原生图仍未验。
- `WIN-WASAPI-STOP-TAIL-01`：独立 `codex/windows-wasapi-stop-tail`，基于 `da18681`。
  正常 Stop 与 Pause 的清空策略分开；先停止、按实际 endpoint 容量排空，再 Reset 并保留 PCM。
  六项新增停止合同与七项既有音频合同：旧控制协议红基线 9 passed / 4 failed，修复后 13 passed。
  干净源码 `a463c3ba9be876dbfe1a45893dcca28e013cadb6` 完整 Windows 默认/QA 门禁 exit 0，
  27 passed / 0 failed / 1 skipped；默认 Rust 1058、QA Rust 1111（各 5 ignored，重叠不累加），
  13 项音频合同在两个真实 Cargo 图通过，已包含在 Rust 总数；前端 75 文件 / 1292 passed。
  WASAPI API 图编译/lint、检出干净与日志哈希已核对；录屏仍非默认，真实设备/混音和新 SHA CI 未验。
- `WIN-WGC-CLOSE-01`：独立 `codex/windows-wgc-close-retry`，基于 `3308677`。
  原全局 closed 和 session 错误短路的离线红基线 1 passed / 5 failed，修复后六项通过；
  vendor 原始字节正校验和 runtime/mod/recorder 三个篡改负例通过。关闭状态按资源记成功，
  两者均尝试，失败可重试。产品修复 869a13f、独立 CI 接线及被测 SHA 03b4cb8；完整
  Windows 默认/QA 门禁 exit 0，28 passed / 0 failed / 1 skipped（Linux smoke）。
  默认 Rust 1058、QA Rust 1111（各 5 ignored，重叠不累加），前端 75 文件 / 1292 passed。
  六项真实 vendor Cargo 合同通过，不包含在应用 Rust 总数；含测试的 vendor 严格 clippy、
  干净检出与日志哈希已核对。系统 Close 最终释放、真实 WGC 与新 SHA CI 未验。
- `REC-AV-BRIDGE-JOIN-01`：独立 `codex/windows-av-bridge-join`，基于 `99f83f8`。
  视频 join 出错后布尔短路遗漏音频 join；两条线程均 join 后再报告既有 panic 错误。
  完整纯模块和真实受控线程红基线 2 passed / 2 failed，修复后四项通过。干净源码
  091b5cb7663055a3b1a44e2958255bf3717bef79 完整 Windows 默认/QA 门禁 exit 0，28 passed /
  0 failed / 1 skipped；默认 Rust 1058、QA Rust 1115（各 5 ignored，重叠不累加），前端 1292 passed。
  四项回归仅在 QA 图、已计入总数；既有 worker 合并/失败中止测试通过，检出和日志哈希已核对。
  共享 Linux/macOS 图、新 SHA CI 和真实 panic/桌面仍未验，既有原型 CI 前缀覆盖新模块。
- `WIN-WGC-INIT-ROLLBACK-01`：独立 `codex/windows-wgc-init-rollback`，基于 `3c19883`。
  pool 创建后、完整 runtime 建立前的注册/session 错误须尝试 Close，再返回原错误。
  原控制协议红基线 1 passed / 3 failed，真实 scopeguard 的四项绿合同通过；vendor 原始字节
  正例/三个篡改负例通过。产品修复 db05650、独立 CI 接线及被测 SHA 61d6823，完整 Windows
  默认/QA 门禁 exit 0：29 passed / 0 failed / 1 skipped（Linux smoke）；默认 Rust 1058、QA Rust
  1115（各 5 ignored，重叠不累加），前端 75 文件 / 1292 passed。独立 vendor 四项初始化及六项
  关闭合同通过，不计入应用 Rust 总数；严格 clippy、检出与日志哈希已核对。真实最终释放、桌面与新 SHA CI 未验。
- `WIN-REGISTRY-BUFFER-01`：独立 `codex/windows-registry-buffer`，基于 `f84e6be`。
  RegGetValueW 字节数误作 u16 长度的初始化合同缺陷已修复；八项生产入口离线回归通过，
  安全旧单位协议辅助模型 4 passed / 4 failed，原未定义行为不执行。8c6fbfe 首次门禁因遗留
  导入的严格 lint 失败（29 passed / 1 failed）；修正后干净源码 bb38cc6 完整 Windows 默认/QA
  门禁 exit 0，30 passed / 0 failed / 1 skipped（Linux smoke）；默认 Rust 1058、QA Rust 1115，
  各 5 ignored、重叠不累加，前端 1292 passed。八项真实 vendor Cargo 回归、严格 clippy、
  三个字节篡改负例、检出干净和日志哈希已核对；新 SHA CI、实际注册表与桌面未验。
  其它 Windows 持久化/自启动路径未确认新增缺陷；混合 DPI 的实际硬件验收保留 W04。
- `WIN-WINDOW-SCALE-01`：独立 `codex/windows-window-candidate-scaling`，基于 `2bb755a`。
  单一窗口比例跨屏求交的实际旧函数抽取协议红基线 2 passed / 6 failed；Windows 物理矩形
  逐帧转换、裁剪并保留分数坐标，八项 MSVC 离线回归通过，含空像素帧边界。干净源码
  78bd83fc3547459c523a150faa8a999248c16267 完整默认/QA 门禁 exit 0，30 passed / 0 failed /
  1 skipped（Linux smoke）；默认 Rust 1066、QA Rust 1123（各 5 ignored，重叠不累加），前端
  1292 passed，八项回归含在两个 Rust 图。首次包装器退出码缺失不计通过，记录保留；
  修正捕获并验证退出 0/17 后，同 SHA 完整重跑通过。W04 真机、新 SHA CI 与其它宿主仍未验。
- `WIN-OVERLAY-FOCUS-01`：独立 `codex/windows-overlay-focus`，基于 `6a9a84e`。原 reveal 把
  物理光标与逻辑矩形比较，实际生产入口红基线 2 passed / 6 failed；按各冻结帧比例判定，
  保留会话/焦点兜底，既有双屏夹具补齐一致两帧并保留原断言。八项绿回归及干净源码
  f5779966f2adaca65d75dd0a0f6affece6e4fc6c 完整默认/QA 门禁确认子进程 exit 0，30 passed /
  0 failed / 1 skipped（Linux smoke）；默认 Rust 1074、QA Rust 1131（各 5 ignored，重叠不累加），
  前端 1292 passed；八项新回归在两个 Rust 图执行，日志哈希、检出干净和原断言审计通过。
  原生覆盖层/guide 建窗的逻辑位置歧义及原始物理原点舍入仍单列 W04，未用本修复关闭。
- `WIN-NATIVE-MONITOR-01`：独立 `codex/windows-physical-monitor-bounds`，基于 `4630b38`。
  原始物理边界在逻辑归一化前保留，覆盖层/guide 请求为物理类型；光标、窗口候选、长截图
  滚动点和重捕获身份直接使用原始边界，拒绝缺失/不匹配/空/溢出元数据。实际旧算法红基线
  1 passed / 15 failed，五份回归原始字节不变，十六项 MSVC 离线回归通过。首次 b3e8f7a 门禁
  28 passed / 2 failed / 1 skipped，夹具严格 lint 失败；原日志保留，修正后的干净源码
  2f0e225494dcf850bacac21c59566b781a3b6767 完整默认/QA 门禁确认子进程 exit 0，30 passed /
  0 failed / 1 skipped（Linux smoke）；默认 Rust 1090、QA Rust 1147（各 5 ignored，重叠不累加），
  前端 75 文件 / 1292 passed、Python 33 + 3。每图新十六项及既有焦点/候选各八项包含在总数；
  原始日志哈希、干净检出、严格 lint、供应链和构建通过，后继仅四份 Markdown。
  当前 SHA CI、实际窗口/DPI 事件、Pin/WGC 原点身份、Windows 10/多屏仍未验，不关闭 W04。
- 已安装包仍为旧源码 `45769c9`。实际 Windows 11 桌面记录为 2 pass / 1 fail（旧 Pin 工具栏裁切）/
  36 not_run；原始 39 项 not_run 模板保持原字节，模板不能替代实际记录。
- NSIS 落盘及启动已有子步骤证据；完整安装升级、MSI、卸载、录屏/音频、管理员目标、
  Windows 10、双屏/混合 DPI/负坐标仍未验收。暂停中的桌面项不能用代码合同或旧 CI 勾选。

对应修复规格见 `2026-10-01-windows-pin-toolbar-height.md`、`2026-10-01-windows-private-write-order.md`、
`2026-10-01-windows-longshot-cursor-restore.md`、`2026-10-01-windows-cf-html-bounds.md`、
`2026-10-01-windows-image-decode-budget.md`、`2026-10-01-windows-dibv5-pixel-offset.md`、
`2026-10-01-windows-paste-input-recheck.md`、`2026-10-01-windows-wasapi-stop-tail.md`、
`2026-10-01-windows-wgc-close-retry.md`、`2026-10-01-windows-av-bridge-join.md`、
`2026-10-01-windows-wgc-init-rollback.md`、`2026-10-01-windows-registry-buffer.md`、
`2026-10-01-windows-window-candidate-scaling.md`、`2026-10-01-windows-overlay-focus.md`。
以下基线与早期门禁记录保留各自来源 SHA；本状态更新为后继文档，不冒称文档 SHA 已执行门禁。

## Baseline

- 已刷新 origin；最新分支为 `origin/codex/recording-audio-device-selection`。
- 基线 SHA：`8b99b884f660f37c9d81ba0dc8947d13c3d3a08a`，应用版本 `0.1.20`。
- 最初工具修复分支：`codex/windows-native-review`；只承载 Windows 验证入口和合同测试修复。
  后续产品问题使用上文列出的独立分支。
- 用户确认 Ubuntu Wayland 已调试正常；本轮未重跑该环境，不扩展为 Linux 全矩阵验收。
- 基线 GitHub CI：<https://github.com/51hhh/Clippy/actions/runs/35792281966>，
  三项默认原生检查及 Ubuntu、Windows、macOS ARM/Intel 四项录屏原型检查均为 success。
  这是基线证据，不能替代后续修改 SHA 的检查或 Windows 桌面 QA。

## Requirements

1. 区分 dev、发布 tag 和最新功能分支，记录关键提交带来的实际能力及 feature 门控。
2. 在 Windows 本机运行完整前端测试、类型检查、静态合同和生产构建；文件 URL 使用系统路径转换，
   源码合同兼容 LF/CRLF；负例修改必须实际生效。vendor 和 Cargo 锁文件按仓库规则保留 LF，
   原始 SHA-256 校验不得归一化输入。保留现有安全合同的全部负例，不能删除失败测试或弱化校验。
3. 提供原生 PowerShell 门禁，覆盖 Python 纯合同、默认 Rust、vendor WGC、前端和可选 Windows
   双轨 QA 检查。缺工具、外部命令非零、显式部分检查和跳过项不得被报告为完整通过。
4. Windows 原生 CI 增加前端检查，补上只有 Ubuntu 执行前端导致的宿主路径盲区。
5. 审查混合 DPI/负坐标、普通与管理员目标粘贴、DACL/原子配置、WGC/WASAPI、设备目录、
   录屏控制窗排除、暂停/恢复和崩溃分段恢复；每个结论区分复现缺陷、静态疑点和未执行验收。
6. 保持录屏 `recording-windows-av-qa` 非默认；正式 release 的能力不因 review 提前开放。
7. 产品行为修复使用独立分支、自己的回归测试与 CHANGELOG；本分支仅验证工具变更，无用户界面行为变更。
8. OCR 质量工具的诊断目录在 Windows 使用当前用户专用、禁止宽松继承的 DACL；创建失败不得
   落回普通目录。符号链接拒绝合同不要求普通 Windows 用户具备创建真实符号链接的权限。

## Acceptance Criteria

- [x] 最新分支、完整 SHA、关键节点和基线同 SHA CI 已核对。
- [x] Windows 前端红基线已取得：73 个文件，1265 项通过、9 项失败，失败均为两组合同测试路径错误。
- [x] 两组合同测试与完整前端测试在 Windows 通过，类型、JS lint、IPC/HTML、供应链和生产入口通过。
- [x] PowerShell 门禁的缺依赖、非零退出码和部分检查不能虚报完整成功；入口文档与脚本一致。
- [x] Windows OCR 诊断目录及新建子文件的 DACL 原生检查通过；质量测试不依赖 POSIX mode 或符号链接特权。
- [x] SHA `42e52c064aba36bb7e93a5e68a0cfb54f65c5b7b` 的三项原生与四项录屏原型 CI 均 completed/success；不替代后续独立 W10 修复的 CI。
- [x] 默认及 `recording-windows-av-qa` 在 Windows 本机完成 Rust check/clippy/test。
- [ ] Windows 10/11 混合 DPI、多屏、权限、安装更新及有声录屏桌面 QA 绑定相同包/SHA。
- [ ] 已确认的产品缺陷修复后复测；尚未执行、外部工具缺失和静态疑点保留未完成状态。

## Out of Scope

- 自动合入 dev、改写历史、发布版本或启用录屏默认 feature。
- 以 Ubuntu Wayland、GitHub runner 或合成源测试替代 Windows 桌面证据。
- 重做 UI，或在 Windows review 中扩展 OCR 权重交付、智能擦除、任意脚本等其它产品需求。
- 未经确认地把静态 DPI 疑点写成已复现缺陷或做跨域坐标重构。

## Tasks

| ID | 优先级 | 工作与验收 | 状态 |
|---|---|---|---|
| W01 | P1 | 修复文件 URL 路径；保留 9 项合同负例/正例并完成 Windows 前端门禁 | 本机已通过 |
| W02 | P1 | PowerShell 门禁与 Windows CI 前端检查；缺工具/失败/部分运行严格区分 | 本机入口与 42e52c0 Windows 前端/OCR/Rust CI 已通过 |
| W03 | P1 | Rust MSVC、C++ SDK、WebView2；录屏另需 MSYS2 make/diffutils/perl/nasm、MSBuild、CMake、LLVM tools/libclang；默认与 QA 图分别验证 | 工具已安装，默认与录屏 QA 本机门禁通过 |
| W04 | P1 | 100%/125%/150% 多屏与负坐标：冻结帧、跨屏窗口候选、覆盖层、Pin、guide、长截图自动滚动、WGC 选区 | 候选/焦点见 W29/W30，原始物理边界和建窗请求见 W31；图片 Pin 来源/请求见 W32；实际 DPI/热插拔、工作区保存/恢复与工具条合同见 W33；WGC 与真机矩阵仍未验，本机单屏。长截图失败清理指针合同见 W19，真实接管待验 |
| W05 | P1 | 同权限自动粘贴一次、高完整性目标 copy-only、目标销毁/复用、用户接管；DACL 与配置连续覆盖 | 45769c9 普通权限文本/图片完整用例实际通过；管理员、销毁复用与用户接管桌面待验证。私有文件准备失败时序见 W18，富文本片段边界见 W20，首次按键前目标复核见 W23，部分 Click 失败清理见 W45 |
| W06 | P1 | QA 包设备默认/非默认/同名/拔出、双源混音、暂停恢复、控制窗排除、强杀恢复、30 分钟 A/V 漂移 | WASAPI 正常停止尾部见 W24，WGC 关闭/初始化清理见 W25/W27，双轨桥接线程回收见 W26，WGC 应用帧桥启动回滚见 W35；真实设备、混音及其余场景仍待真机验收 |
| W07 | P2 | NSIS/MSI 安装升级卸载、WebView2、自启动、托盘/快捷键、系统凭据与更新 | 官方 QA 包身份已核对，MSI 只读检查通过；NSIS 安装落盘/启动子步骤已核对，完整 MSI/升级/卸载/updater 未验收；本机自签名链不受信任，未更改信任 |
| W08 | P1 | 每个产品修复单独分支，更新对应需求/CHANGELOG；同 SHA 三平台 + 四原型 CI，回归 Ubuntu Wayland | 42e52c0、45769c9 与 WinPS 的 b2fd247 各自七项 CI 通过；后续49项产品修复本机通过，新 SHA CI、Linux 本地完整门禁及 Wayland 回归保留未完成 |
| W09 | P1 | OCR 质量工具 Windows 私有诊断目录与符号链接拒绝合同；失败关闭，检查子文件继承 | 实际 DACL/等价 SDDL 及 10 类失败关闭负例通过；本机 33 项质量合同与 42e52c0 跨平台 CI 通过 |
| W10 | P2 | 审查 webm-sys 的 C++ 编译参数在 MSVC 上产生 D9002；按真实编译器族选择 flag，保留固定来源与许可证 | 独立 WIN-WEBM-MSVC-01 / PR #14；本机完整 QA 绑定 e4ccc46，45769c9 七项 CI 与完整 QA workflow 全成功，新 Windows 包来源/哈希/签名身份已核对；真实桌面未验证 |
| W11 | P1 | 新 Windows runner 使用 CRLF 检出时的 IPC 负例与结构回归；保留两种换行的正/负合同 | 独立 CRLF checkout 1284 项通过，fe37aec Windows 前端 CI 已通过 |
| W12 | P1 | vendored xcap 保持固定 LF 字节并运行原始 SHA-256 校验；不能归一化哈希输入或跳过检查 | 独立 CRLF checkout 前端门禁 11 项通过、0 失败；字节篡改仍被拒绝，fe37aec Windows 前端 CI 已通过 |
| W13 | P1 | Windows 原生进程/locale 测试预算覆盖实测初始化；保留子进程硬超时、全部断言与普通单元测试默认预算 | CI 暴露两项 5 秒超时；限定测试组补齐预算后，Node 24.21.0 + CRLF 全前端门禁通过，fe37aec Windows 前端 CI 已通过 |
| W14 | P1 | 浏览器 OCR 捕获来源 HTML/脚本固定 LF，保留原始字节哈希和来源记录；新 CRLF clone 与篡改负例验证 | fe37aec CI 复现来源哈希失配；28186af 全新 CRLF clone 来源哈希通过，额外 LF 负例被拒绝；42e52c0 CI 通过 |
| W15 | P1 | OCR 取消/回收夹具区分启动与执行预算；慢启动、取消、后继排队、kill/wait 与许可顺序，macOS 实际 PID 回收 | 独立 OCR-PROC-CANCEL-01 / PR #15；Windows 探针与 7982866 完整七项 CI 通过，Unix 用例实际成功；继承修复的 #14 七项另作证据，桌面未验证 |
| W16 | P1 | Windows PowerShell 5.1 版本检查的引号传输；保留失败和版本边界合同 | 独立 WIN-PS-GATE-01；ef78a1f 本机完整门禁通过，文档后继 b2fd247 七项 CI 通过；新 SHA 与桌面另验 |
| W17 | P2 | 小图 Pin 完整工具栏及上下间隙；创建、缩小与恢复位置共用高度合同 | 独立 WIN-PIN-TOOLBAR-01；旧包实际失败、前端回归先失败后通过，7aa6cf6 本机完整门禁通过；修复后桌面和新 SHA CI 未验 |
| W18 | P2 | 私有文件权限准备失败不得先写内容或截断原文；真实文件故障注入与 Windows DACL | 独立 WIN-PRIVATE-WRITE-01；Windows 红绿及六项定向合同通过，f788b1f 本机完整默认/QA 门禁通过；跨账户、路径竞争及新 SHA CI 未验 |
| W19 | P2 | 自动长截图提前失败的光标恢复不能抢回用户已移动位置，查询失败关闭恢复 | 独立 WIN-LONGSHOT-CURSOR-01；同一生产 guard 红绿及八项定向合同通过，d8dff80 本机完整默认/QA 门禁通过；真实接管、X11/macOS 原生图及新 SHA CI 未验 |
| W20 | P1 | Windows CF_HTML 片段范围受实际字节与 UTF-8 边界约束，默认门禁不能遗漏依赖库合同 | 独立 WIN-CF-HTML-01；旧校验离线红基线、安全解析九项合同及 50b7778 完整本机默认/QA 门禁通过；CI 入口已接线，远程新 SHA、真实互操作和其它原生图未验 |
| W21 | P1 | Windows PNG / DIB 在整图像素分配前执行已有预算，保留合法 4K/8K 与小图像素 | 独立 WIN-CLIP-IMAGE-BUDGET-01；红基线 3 passed / 2 failed，预算七项及 531d791 完整 Windows 默认/QA 门禁通过，新 SHA CI 与桌面未验 |
| W22 | P2 | Windows DIBV5 显式像素偏移，防止小图读取失败与尾部掩盖错图 | 独立 WIN-DIBV5-PIXEL-01；红基线 2 passed / 2 failed，原 Chrome/Firefox 及新增像素/文件视图合同、25fb5d7 完整 Windows 本机默认/QA 门禁通过；真实提供者和新 SHA CI 未验 |
| W23 | P1 | Windows 首次按键前复核当前目标，不能沿用激活/后端初始化前的窗口身份与焦点 | 独立 WIN-PASTE-RECHECK-01；同一生产入口红基线 2 passed / 4 failed，六项离线合同及 14bf616 完整 Windows 本机默认/QA 门禁通过；真实接管、系统竞争、macOS 原生图和新 SHA CI 未验 |
| W24 | P1 | 正常 WASAPI Stop 保留已复制 PCM 和有限 endpoint 尾包，Pause 清空，故障关闭 | 独立 WIN-WASAPI-STOP-TAIL-01；旧控制协议红基线 9 passed / 4 failed，13 项真实 Cargo 合同与 a463c3b 完整 Windows 默认/QA 门禁通过；真实设备/混音与新 SHA CI 未验 |
| W25 | P1 | WGC Close 每轮均尝试两个资源，按成功状态幂等，失败允许 Drop 重试 | 独立 WIN-WGC-CLOSE-01；红基线 1 passed / 5 failed，六项 vendor Cargo 合同与三个原始字节篡改负例通过；03b4cb8 完整 Windows 默认/QA 门禁通过；系统最终释放、真实 WGC 与新 SHA CI 未验 |
| W26 | P2 | 双轨编码退出不因视频桥接 panic 遗漏音频 join，两条回收后保留既有错误 | 独立 REC-AV-BRIDGE-JOIN-01；真实受控线程红基线 2 passed / 2 failed，四项 QA Cargo 回归与 091b5cb 完整 Windows 默认/QA 门禁通过；共享其它宿主图、新 SHA CI、实际设备 panic 未验 |
| W27 | P2 | pool 创建后、完整 WgcRuntime 前的两处失败先 Close，成功转移所有权 | 独立 WIN-WGC-INIT-ROLLBACK-01；红基线 1 passed / 3 failed，四项 vendor Cargo 合同与 61d6823 完整 Windows 默认/QA 门禁通过；真实 API 失败、系统最终释放与新 SHA CI 未验 |
| W28 | P1 | Windows 构建号仅解析成功返回的有界字节范围，缓冲区完全初始化 | 独立 WIN-REGISTRY-BUFFER-01；安全旧单位模型 4 passed / 4 failed，八项真实 vendor 合同与三项篡改负例通过；8c6fbfe 首次严格 lint 失败已修复并保留，bb38cc6 完整门禁 30 passed / 0 failed；新 SHA CI、实际注册表与桌面待验 |
| W29 | P1 | Windows 跨屏物理窗口按每帧比例转换/裁剪，保留分数边界与 Z 顺序 | 独立 WIN-WINDOW-SCALE-01；旧函数抽取协议 2 passed / 6 failed，八项 MSVC 回归与 78bd83f 完整默认/QA 门禁通过，30 passed / 0 failed / 1 Linux smoke skipped；首次包装器退出码缺失不计通过、原记录保留，修正后同 SHA 重跑退出 0；真实多屏与新 SHA CI 未验 |
| W30 | P1 | Windows 物理光标按各帧比例选择覆盖层键盘归属，保留未知光标/无效元数据兜底 | 独立 WIN-OVERLAY-FOCUS-01；旧生产 reveal 红基线 2 passed / 6 failed，八项绿回归与 f577996 完整默认/QA 门禁通过、子进程 exit 0，30 passed / 0 failed / 1 Linux smoke skipped；原断言审计通过，真实 set_focus、原生建窗、多屏与当前 SHA CI 未验 |
| W31 | P1 | 冻结原始物理边界贯穿 Windows 覆盖层/guide、光标、窗口候选、长截图指针与重捕获身份 | 独立 WIN-NATIVE-MONITOR-01；MSVC 实际旧算法红基线 1 passed / 15 failed、十六项绿回归；首次完整门禁 28 passed / 2 lint failed 保留，修正后的 2f0e225 完整默认/QA 门禁 exit 0，30 passed / 0 failed / 1 Linux smoke skipped。实际窗口/DPI、多屏、Pin/WGC 原点身份与当前 SHA CI 未验 |
| W32 | P1 | 冻结物理来源贯穿截图/长截图及历史图片 Pin，单次原生规划与两阶段物理请求 | 独立 WIN-PIN-ORIGIN-01；旧生产输出及提取布局/请求协议红基线 3 passed / 15 failed，同组十八项 MSVC 离线回归和 888127a 完整默认/QA 门禁通过、原生子进程 exit 0；30 passed / 0 failed / 1 Linux smoke skipped。真实窗口/DPI、工作区/工具条/WGC、新 SHA CI 与其它宿主未验 |
| W33 | P1 | 原生 owner/工作区贯通 Pin 保存、恢复及客户区工具条边界 | 独立 WIN-PIN-WORKAREA-01；提取旧生产协议红基线 1 passed / 15 failed，同组十六项 MSVC 回归和 4a58101 完整默认/QA 门禁通过，原生子进程 exit 0；30 passed / 0 failed / 1 Linux smoke skipped。真实 OS owner/窗口/DPI/热插拔、多屏/Windows 10、WGC 与当前 SHA CI 未验 |
| W34 | P2 | Windows Pin 实时 DPI 渲染判据、首读/事件竞争与 caller-bound 只读查询 | 独立 WIN-PIN-LIVE-DPI-01；原 App 十二项红基线 2 passed / 10 failed，同组全绿、三项 API 与六项 MSVC 纯数据读取合同通过；e339042 完整 Windows 默认/QA 门禁确认原生子进程 exit 0，30 passed / 0 failed / 1 Linux smoke skipped。真实事件/成像、多屏/Windows 10 与新 SHA CI 未验 |
| W35 | P2 | WGC 应用帧桥启动回滚与销毁拥有取消/join，不依赖外部 sender 释放 | 独立 WIN-WGC-BRIDGE-ROLLBACK-01；提取旧协议红基线 3 passed / 4 failed，同组七项真实线程合同全绿；7922457 完整默认/QA 门禁 exit 0，30 pass / 0 fail / 1 Linux skip；实际 WGC/Close/系统释放、硬件与新 SHA CI 未验 |
| W36 | P2 | 主窗口有保存目标时不执行后备原生查询，必要后备和保存查询错误保持 | 独立 WIN-MAIN-TARGET-01；提取旧求值协议红基线 11 passed / 2 failed，旧七项全绿、新六项红 4/2，修复后十三项通过；c9d5512 完整默认/QA 门禁 exit 0，30 pass / 0 fail / 1 Linux skip；真实显示/原生错误/DPI、多屏、其它宿主与新 SHA CI 未验 |
| W37 | P1 | 录屏控制窗准备失败回滚按销毁请求返回结果结算，请求失败仍阻止替换 | 独立 WIN-CONTROL-ROLLBACK-01；提取原回滚协议红基线 22 passed / 2 failed，旧十七项全绿、新七项红 5/2，修复后二十四项通过；干净 a52ecaa 完整默认/QA 门禁原生子进程 exit 0，30/0/1（Linux skip），默认 Rust 1150、QA Rust 1207（各 5 ignored，重叠不累加），新增七项在总数内，前端 1307 passed；请求成功不是原生销毁完成，真实请求失败/捕获像素/设备/强杀、其它宿主和新 SHA CI 未验 |
| W38 | P1 | 录屏恢复合并与删除原子取得同会话所有权，删除 guard 覆盖完整 worker | 独立 REC-DELETE-OWNER-01；提取原删除协议红基线 3 passed / 3 failed，生产已提交 VP9 分段/清单删除缺口复现；同组六项通过，既有单槽/关键 manifest 合同通过；干净 0321355 完整默认/QA 门禁原生子进程 exit 0，30/0/1（Linux skip），默认 Rust 1150、QA Rust 1213（各 5 ignored，重叠不累加），新增六项仅在 QA 总数内，前端 1307 passed；默认删除不变；真实重开窗口/强杀/播放、其它宿主和新 SHA CI 未验 |
| W39 | P1 | Windows 导出用完整句柄身份提前拒绝同文件别名，保留普通导出 | 独立 WIN-EXPORT-IDENTITY-01；原生产导出直接红基线 2 passed / 4 failed（三者替换 error 5、硬链接 Ok），同组六项修复后通过，旧导出/原生错误各一项通过；干净 69cf0b4 完整默认/QA 门禁原生子进程 exit 0，30/0/1（Linux skip），默认 Rust 1157、QA Rust 1220（各 5 ignored，重叠不累加），新增六项与错误一项在各图总数内，前端 1307 passed；真实对话框/媒体、文件系统/网络盘/权限与外部竞态、其它宿主和新 SHA CI 未验 |
| W40 | P1 | 会话撤销同时失效尚在准备中的播放租约，保留其它会话与新准备 | 独立 REC-MEDIA-REVOKE-01，补齐 PX-REC-PLAYBACK-01 / 4；原 manager 红基线 7/4，四项实际返回 Ok 租约；同组六项与新增两项、旧五项共 13 passed；原合同核对，干净 6944b68 完整默认/QA Windows 门禁 child/终端 exit 0，30/0/1（Linux skip），默认 Rust 1165、QA Rust 1228（各 5 ignored，重叠不累加），新八项与旧媒体五项各图通过；前端 1307 passed。真实删除/合并/播放、哈希中间撤销时序、其它宿主与新 SHA CI 未验 |
| W41 | P1 | 播放准备响应绑定组件/请求身份，退休后释放迟到租约，保留后继与其它动作状态 | 独立 REC-PLAYBACK-LIFECYCLE-01；原 App 字节 jsdom 红基线 27/5（旧 24 全绿），同八项原字节修复后通过，定向 32 passed；原合同/十一份关联文件核对。干净 ba26a83 完整默认/QA 门禁 child/终端 exit 0，30/0/1（Linux skip），默认 Rust 1165、QA Rust 1228（各 5 ignored，重叠不累加，无新增 Rust 用例），前端 78/1315 含新八项；发现清单/源码/日志与干净检出核对，实际 backend/窗口/WebView/其它宿主与新 SHA CI 未验 |
| W42 | P1 | 结果库 ready 绑定当前 effect，退休后不再请求显示/聚焦，保留正常/错误页就绪 | 独立 REC-LIBRARY-READY-01；原 App 字节红基线 35/5（旧 32 全绿），四项旧 ready 与一次 StrictMode dev/test 两次调用复现，同八项原字节修复后通过，定向 40 passed；原合同/十三份关联文件保持。干净 1907809 完整默认/QA Windows 门禁 child/终端 exit 0，30/0/1（Linux skip），默认 Rust 1165、QA Rust 1228（各 5 ignored，重叠不累加，无新 Rust 用例），前端 79/1323 含新八项；源码/日志/发现清单与干净检出核对，实际窗口/focus/发布重放、其它宿主与新 SHA CI 未验 |
| W43 | P1 | 共用键按 Shortcut ID 继承首次注册结果，全失败返回错误，保留部分成功与旧配置容错 | WIN-SHORTCUT-SHARED-01；提取原执行协议 MSVC 红 9/6（旧五项通过），同十项原字节绿 15/0；原合同/十九份文件保持。 干净 0f793c9 完整默认/QA 门禁 30/0/1（Linux skip），默认 Rust 1175/QA Rust 1238，各 5 ignored 不累加，新十项/旧五项各图通过；前端 79/1323，实际系统/UI/其它宿主/新 SHA CI 未验 |
| W44 | P1 | Windows 富文本与替代文本共享 OpenClipboard guard，拒绝跨复制配对 | WIN-CLIP-SNAPSHOT-01；原决策红 27/4，旧 23 保持，同修正八项绿 31，guard 七项/旧 parser 九项共 16 与 vendor lint 通过。 干净 b541e87 完整 30/0/1（Linux skip），默认 Rust 1183/QA Rust 1246，各 5 ignored 不累加，新八项各图通过；独立剪贴板 31、前端 79/1323。实际系统/其它宿主/新 SHA CI 未验 |
| W45 | P1 | Windows V Click 部分失败/展开时清理 V，保留正常顺序和主要错误 | WIN-PASTE-CLEANUP-01；原注入协议红 10/6，旧六项保持，同十项原字节绿 16；原实现/十二文件/锁定 SDK 保持。 干净 f5ad5da 完整 30/0/1（Linux skip），默认 Rust 1193/QA Rust 1256，各 5 ignored 不累加，新十项各图通过；前端 79/1323。实际系统/其它宿主/新 SHA CI 未验 |
| W46 | P2 | 原需求/验收与修复Git/日志/测试保留审计，W79绑定当前49项 | WIN-NATIVE-01；W79核对49项原日志与当前同阶段用例、1428 Rust正文/111前端调用，修正汇总旧SHA关联，不计新测试；原8R/9AC/47任务保留。W78当前release文件核对通过，同SHA CI/原生QA未完成，下一步审查未来失败诊断保留 |
| W47 | P2 | 默认/录屏 QA Windows release 编译与冻结源码/产物/feature/profile 来源核对 | WIN-NATIVE-01；两图 native/wrapper/terminal 0、PE AMD64 GUI/无内嵌签名、原输入哈希和干净源码保持；release panic=abort 不借用测试 unwind 保证。不增加测试数、不启动/安装，全局剩余验收保留 |

W04–W07 使用 `docs/native-qa.md` 和 `scripts/manual-qa.mjs` 的 Windows profile。
安装包证据与本地源码构建分开，模板初始 `not_run` 不能计作通过。

## Verification

本机：Windows 11 Pro for Workstations x64，build 22000；Node 22.22.0，Python 3.12.7。
用户已授权安装工具链。Rust 1.98.1 MSVC、VS 2022 C++ Build Tools/Windows SDK、MSYS2 已安装；
WebView2 Runtime 154.0.4258.37 已核对，Rust LLVM tools 与 LLVM 23.1.2 libclang 已安装，
VS 附 CMake 3.31.6-msvc6 已补入当前会话 PATH。原生默认与录屏 QA 检查通过。

`ci-windows.ps1 -FrontendOnly`：11 项检查通过，0 失败，3 组显式跳过；这是完整前端范围，
属于整体部分门禁。Vitest 74 个文件、1277 项通过；包含 3 项真实 PowerShell 退出码合同。
Python 质量合同 31 项通过，视觉段落 3 项通过，智能擦除证据校验通过。
默认完整 Windows 范围：20 项检查通过、0 失败、2 组显式跳过；Rust 1040 项通过、5 项忽略，
check、严格 clippy、vendor WGC 严格 clippy 通过。前端追加缺依赖回归后为 74 文件、1278 项通过。
录屏 `-RecordingQa` 最终完整 Windows 范围：23 项检查通过、0 失败、1 组 Linux smoke 显式跳过。
QA Rust 1093 项通过、5 项忽略；前端最终为 74 文件、1280 项通过，含 6 项门禁退出码/前置依赖合同。
默认与 QA 测试大量重叠，不累加为独立覆盖数。Python 质量 31 项、视觉段落 3 项通过。
独立 `core.autocrlf=true` checkout 修复 W11/W12 后：完整前端范围 11 项检查通过、0 失败、
3 组显式跳过，74 文件 / 1284 项通过。主 Rust 源码保持 CRLF，xcap 和 Cargo 锁文件按属性检出 LF；
刻意在固定文件追加一个 LF 被原始 SHA-256 校验拒绝，复原后校验成功。
全新 CRLF clone 在 `47f1ee7dcd5f234d3bc5756cebe6202de2f5fc47` 首次检出即保持 vendor/锁文件 LF，
原始哈希校验成功。本机同 SHA 使用 `recording-windows-av-qa`、debug、`--no-sign` 构建 MSI/NSIS，
两包成功且为 NotSigned；绑定 SHA 的 LOCAL-BUILD.json 和 SHA256SUMS.txt 保存在 ignored target 目录。
这些仅属诊断构建证据，不计入测试通过数，也不替代官方 Native QA 包、签名或安装验收。
用户确认目前仅有当前 Windows 11 单屏，暂无多屏或 Windows 10 验收环境。
CI SHA `c9e504c` 原五项 CRLF 合同已通过，但 PowerShell/首次 locale 两项触发默认 5 秒超时。
W13 补丁仅给 Windows 原生测试组有界预算，PowerShell 子进程仍在 20 秒硬超时后失败；无重试/删断言。
与 CI 同版本的 Node 24.21.0 在 workspace 内隔离下载并验证官方 SHA-256，CRLF 前端门禁为
11 项通过、0 失败、3 组显式跳过；74 文件 / 1284 项通过。系统 Node 版本未替换。
当时 Linux 本地完整门禁、修改后 CI、官方 QA 安装包、Windows 10 和桌面交互尚未执行；后续结果见下文。

详细审查证据见 `docs/reviews/2026-10-01-windows-native-review.md`。所有新结果按层级追加，
没有实际执行的项不勾选。

W09 后续：fe37aec 的 Windows 前端 CI 通过，OCR 实际 DACL 与手写 SDDL 比较失配，Rust 未执行。
改为二进制 ACE/保护位严格核对，本机 33 项质量 + 3 项视觉段落通过；等价 SID/AI 正例和
10 类真实创建失败关闭负例均保留。当时修改后同 SHA CI 与完整 Windows 门禁仍待复验。

W09/W14 最新源码 28186af 的完整 Windows 默认门禁：20 passed / 0 failed / 2 skipped，
Rust 1040 / 5 ignored、前端 74 文件 / 1284 passed、Python 33 + 3 passed。
全新 CRLF clone 状态干净；两个浏览器语料的额外 LF 负例均 exit 2，验证器与来源登记未改。
桌面自动化运行时再次初始化失败，没有 UI 观测；Windows 11 单屏人工 QA、Windows 10、多屏保持未完成。

### 同 SHA 远程检查与 Windows QA 包

SHA `42e52c064aba36bb7e93a5e68a0cfb54f65c5b7b` 的
[run 36816386762](https://github.com/51hhh/Clippy/actions/runs/36816386762) 七项全部 completed/success。
三项规定的原生结果由仓库评估器核对 authenticated check-runs；Windows 日志为前端 74 文件 /
1284 passed、Python 33 项质量 + 3 项视觉段落、Rust 1040 passed / 5 ignored。
这是该 SHA 的 CI 证据，不把后续证据文档提交或独立 W10 SHA 写成已验证。

同 SHA 的 [Native QA run 36817675012](https://github.com/51hhh/Clippy/actions/runs/36817675012)
在三项原生检查成功后启动。Windows job 110226195519 已 success；官方 MSI/NSIS 已下载，
QA-BUILD 的完整 SHA、版本、平台、QA feature、自签名用途与 SHA256SUMS 均核对一致。
CI 临时信任环境的 Authenticode Valid/预期签名者检查通过；本机两包状态为 UnknownError，
消息为证书链终止于不受信任根，签名 thumbprint 与 CI 预期相同。未导入证书或修改本机信任。
完整包身份和哈希见审查记录；安装包构建/下载不加入测试通过数。
该 QA workflow 最终 completed/success，四个平台 bundle 与 Ubuntu 24 X11 smoke 全部成功，
六份产物 manifest 均绑定完整 SHA；这些不替代 Wayland 或 Windows 桌面验收。

W10 的 437949b CI 最终为六项 success / 一项 failure，失败是 macOS 原生 OCR 取消/回收测试
未观察到 PID 标记（1054 passed / 1 failed / 5 ignored）。该模块未由 C++ 补丁修改；原夹具
执行预算 250 ms、启动观察 10 s 存在竞态。W15 使用阶段标记、慢启动和真实回收探针单独修复，
不改生产 OCR 预算或取消语义；Windows 探针与 Unix 原生合同分别记录。
7982866 的 Ubuntu/macOS 已实际执行并通过该合同，独立 #15 的 run 36821712880 完整七项 success。
经用户授权同步 #15 后，#14 / 45769c9 的七项全部 success，范围仍六个 WebM 文件；新 SHA 官方
QA run 36823747034 全部 success，Windows 包来源/哈希/签名身份与 MSI 只读元数据已核对，
39 项模板均 not_run。不将旧包/本机门禁替代新包或桌面验收，也不将文档提交 SHA 写成源码 CI 已验证。

官方 Windows 11 模板与本地生成模板字节相同，39 项均 not_run。合成 Unicode、富文本和透明 PNG
夹具已准备且哈希核对完成；用户表示暂不能手动执行，安装、桌面与录屏真机验收继续未完成。
Windows 10、多屏/负坐标/混合 DPI、真实音频、升级/updater、Linux 本地完整门禁与 Wayland 回归
不由 CI 或包校验替代。没有合入 dev、发布或默认开放录屏。
