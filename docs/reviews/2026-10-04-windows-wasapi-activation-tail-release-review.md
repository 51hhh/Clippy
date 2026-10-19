# WIN-NATIVE-01 — WASAPI激活包尾修复的Windows release验证

日期2026-10-04，W84；关联WIN-WASAPI-ACTIVATION-TAIL-01，[规范](../superpowers/specs/2026-10-03-wasapi-activation-tail.md)。
被测修复源码`88891927654717a3d7524c6d94648daca4ad102a`；前置文档`ed29035c965271a55e83a0f3260af224afd5d024`；分支`codex/windows-wasapi-activation-tail-release-validation`。
同一干净源码的默认版和显式录屏QA版release编译及文件来源验证完成，本轮没有产品或测试改动。

## 有限源码复核

复核Windows包复制/释放和控制hook、PCM拆块/裁切、平台feature和转发、worker恢复/封尾、
混音启动、Cargo/release配置、Windows/CI配置、build入口及CRT文件校验共11份文件。
原GetBuffer帧数在copy_packet成功或错误后仍完整ReleaseBuffer；只裁切已归一化的有主PCM。
删除完整旧块，仅跨界块更换有界样本后缀，原序号/包末尾及后续块保留；固定20ms块限制额外分配。
启动/恢复仍采用原单调控制下界，平台转发；混音只有两个源都有下界才从较早起点重启网格。
worker恢复先调用平台hook，再更新公共pipeline；正常停止先排空尾块再finish。
Windows原生模块只进入audio feature，完整A/V源接线还需组合QA；默认产品录屏入口保持关闭。
另沿PCM时间线/视频epoch/块拆分/Opus入口复核四份文件：分数样本持续时长取整可留下正向1ns
媒体间隙，原pipeline保留该值，100ns容差只处理重叠；Opus按累计真实帧数吸收小于一样本的
边界舍入，追加原PCM而不按gap字段虚构静音。既有1ns接受/100us间隙及重叠拒绝用例已在
W83同源码QA1493内通过，不是新测试或额外运行。可选搜索曾使用两处不存在的旧编码路径，
改用rg --files定位实际av_encoder_worker/mux路径；只读搜索exit1不计native构建或测试失败。
此范围没有确认额外缺陷；源码推理不证明COM/驱动/实际声音或全项目正确。
release panic=abort不执行unwind Drop，W80测试目录保留也不证明进程强杀恢复。

## 编译与来源

两个全新独立default/qa Cargo target，冻结同一干净Git tree；752份编译输入与完整门禁1073份
原字节输入绑定，两组输入一致。Cargo.lock/vendor不归一化，前端/资源保持，构建后恢复原文档HEAD。
Tauri build --ci --no-bundle --no-sign --target x86_64-pc-windows-msvc，Windows及CI配置；
Cargo --locked --offline -vv、npm offline、4jobs。QA显式recording-windows-av-qa和同源码CRT资源配置。
Cargo离线不保证vendored libvpx不获取固定哈希源码归档，没有新工具安装或系统更改。
默认终端45619、QA终端17699，实际native子进程/包装器/终端均exit0，均已终结。

| 变体 | EXE字节数 | SHA-256 |
| --- | ---: | --- |
| 默认 | 28748288 | `eacca42a3b4d8425707d28f0291d8d9159e364531de0747b9288ca90b9ba3895` |
| 录屏QA | 31719936 | `422492c981140593865ebc5eb1509b0911e7f2009214bedc8bacaca145edc170` |

真实主程序rustc参数为opt-level=s、panic=abort、lto=fat、codegen-units=1、strip=symbols；
库/二进制feature指纹保存。默认无recording-*或VPX构建指纹；QA包括Windows audio/AV、来源VP9和Opus/WebM。
QA基准二进制只编译，未运行/采集，不计测试。两份EXE为AMD64 PE32+ Windows GUI、证书目录为空。
PE及feature是文件证据，不是启动、兼容性或设备结果。

## 依赖与计数

默认直接/延迟DLL名称集合与00f40cc默认文件相同，没有新增QA私有VC++名称；不是导入函数等价或启动证明。
QA10份既有Microsoft CRT的签名/版本/AMD64/来源/哈希/license、相邻部署及递归依赖核对，实际需要
msvcp140.dll, vcruntime140.dll, vcruntime140_1.dll。原始部署目录见QA RESULT.output.original，单独保存的EXE不包含相邻DLL。
固定VPX来源归档SHA、CRT原清单、rustc命令、fingerprint、PE与日志保存；没有加载DLL或打包/签名/安装。
PROGRESS是历史采样；终态取RESULT和实际终端退出。

W83同源码完整Windows门禁保持33 passed / 0 failed / 1 Linux smoke skipped：默认1341/QA1493各5ignored，
前端81文件/1405，QA录屏523包含于1493。新增七项回归两图14次执行已含总数，重叠图不累加。
门禁已通过且源码不变，不重复测试。本轮修复/新增用例/通过计数均0；51产品修复/50历史、W80诊断分层保留。
证据：current-release-default-8889192、current-release-qa-8889192、current-release-closure-8889192和isolated-release-8889192。

## 未完成边界

同SHA七项CI仍not_run；W83当前8889192查询422/no commit及457控制成功保持实际身份/时间，
本轮没有新网络查询、推送或workflow。旧00f/D05 release与其它SHA CI均保留原范围。
真实首包/暂停恢复/WASAPI设备/默认路由/权限/声音、WGC/焦点/DPI、长时同步、其它宿主、
Win10/多屏混合DPI/负坐标、无开发CRT启动、安装升级卸载/updater仍未完成。旧457包39项桌面
记录2passed/1failed/36not_run保持，不含后续51项修复；W53/W59/W63/W67历史根因仍未明。
原8R/9AC/47Tasks及末两条全局AC、本规范末项复合AC保持未完成。桌面保持停止，无应用/设备
控制、安装/证书/系统更改、PR/合入/发布。全局WIN-NATIVE-01仍未完成。
