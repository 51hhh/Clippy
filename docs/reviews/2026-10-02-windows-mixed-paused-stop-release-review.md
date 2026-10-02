# WIN-NATIVE-01 / W73 — 暂停后停止修复的 Windows release 验证

源码 `02005c98ee6ac52fd46e20c878a717f314fd90cc`，前置文档 `d5c3859eb22a31518d9c01d5d8646f857d83f3cd`，分支 `codex/windows-mixed-paused-stop-release-validation`。
包含 `REC-MIXED-PAUSED-STOP-01` 及前置混音修复；生产代码/测试/原期限保持。
本轮补充同源码默认和非默认录屏QA release编译及文件证据，新修复/新测试均为0。

## 代码续审与构建

只读核对混音源、原音频worker、Windows WASAPI/有限尾包和完整AV owner五个文件，未确认新缺陷。
两源Stop均尝试，分别验证真实控制/尾包；暂停状态不制造PCM、真实尾块明确拒绝。
原worker暂停中入队严格失败，Stop仅在pipeline.finish成功后解除abort guard；原错误身份与join路径保持。
Windows Pause停止/Reset并清除未提交输入，Stop按实际endpoint容量有限排空；读取/Reset错误不记正常完成。
AV owner回收视频/音频/encoder，区别原错误和联动Aborted并核对统计/journal。
此只读审查不证明所有设备异常；代码审查文件内PID是创建时快照，实际终端退出码以构建RESULT为准。

两个新的独立Cargo target冻结同一干净源码，实际native child、包装器和终端均退出0。
Tauri CLI `build --ci --no-bundle --no-sign --target x86_64-pc-windows-msvc`，Windows/CI配置，
Cargo `--locked --offline -vv`、npm offline、4 jobs；QA追加`recording-windows-av-qa`和同源码CRT配置。
绑定738份输入及完整Git tree，默认和QA输入相同。
vendored libvpx可获取固定SHA-256源码归档，Cargo offline不等于全程断网；无安装器或系统工具下载。
原日志和第三方warning保留。QA附加基准二进制只随build编译，未运行或采集设备，不计测试。

| 变体 | EXE字节数 | SHA-256 |
| --- | ---: | --- |
| 默认 | 28746752 | `ff28029c1c8b1d11007260819a00d3fe48d34377bff0150bd3ed5a1bcb0e2dfa` |
| 录屏QA | 31715840 | `df0712d8b574d2c3ef7a9e8b9876ff902771b1ba61876d107515c73d0678bfb4` |

实际主程序rustc参数：opt-level=s、panic=abort、lto=fat、codegen-units=1、strip=symbols。
库和二进制feature指纹：默认无recording-*；QA启用WGC/WASAPI、VP9源码构建和Opus/WebM。
两份EXE均AMD64 PE32+、Windows GUI、嵌入证书目录为空；默认录屏入口仍关闭。
默认直接/延迟导入表与前置B0默认文件一致，未出现QA私有VC++ DLL；这项文件核对不证明其它Windows版本启动。
release panic=abort在进程panic终止时不执行Drop，unwind测试不能证明该终止方式的清理。

## 文件依赖与证据范围

既有10份release CRT的Microsoft签名、版本、AMD64 PE、哈希/来源及EXE相邻DLL/licenses文件核对。
直接/延迟导入和递归CRT闭包验证，所需`msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`。
只读取文件，未加载应用或DLL；无系统CRT机器的启动尚未验证。单个保存EXE不含相邻DLL，
完整未打包目录由QA RESULT的output.original标明，不视为已签名或可发布安装包。
证据在`src-tauri/target/current-release-default-02005c9/`、`current-release-qa-02005c9/`，
包含PID/实际退出码、源码树/输入哈希、日志、编译参数、feature、PE、CRT来源/部署/导入和产物。

W72同源码完整Windows门禁33/0/1 Linux smoke skipped，默认1308/QA1457各5ignored，前端81/1403；
领域491和新12项（10两图/2仅QA）已含总数。原API同字节6/6基线修复后全部通过，
原完整owner正常20ms/960PCM及真实codec文件内容保持；旧模块/期限保持。
本轮未重跑测试，47项本地修复/46项历史及门禁计数保持，构建/运行库/文件检查不计通过数。

## 未完成项

当前SHA跨平台/codec CI、其它宿主、真实WGC/WASAPI、设备切换/漂移/长时同步、桌面、
安装器/updater/无CRT启动、Win10/多屏/混合DPI仍未验。W53/W59/W63/W67历史失败根因仍未明，
原日志/失败材料与历史release保留；已安装457包未更新，39项桌面记录保持。
未启动应用/设备/桌面，未安装/签名/新增工具/证书/推送/PR/合入/发布。
合成源和实际codec文件不替代设备验收；规格最后复合验收与整体任务仍未完成。
