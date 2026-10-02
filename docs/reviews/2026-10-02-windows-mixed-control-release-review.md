# WIN-NATIVE-01 / W71 — 混音修复后的 Windows release 验证

源码 `b0b51f1196124383b9fc77cfc4ad22c21e8c3d84`，前置文档 `a2ffee80eaa4bc8053d743dba55d7e154fc3b7d0`，分支 `codex/windows-mixed-control-release-validation`。
包含 `REC-MIXED-FRAME-BOUNDARY-01` 与 `REC-MIXED-CONTROL-SKEW-01`；生产代码/测试/原期限保持。
本轮补充当前同一源码的默认和非默认录屏QA release编译及文件证据，新修复/新测试均为0。

## 续审范围和构建

只读核对混音启动、暂停、双源回滚/停止、Windows WASAPI清理与原worker所有权，尚未确认新缺陷。
原失败身份/源工厂线程、暂停清除未提交输入、两源Stop尝试与worker错误中止路径保持；
这项代码核对不能证明全部原生设备异常已验收，原有严格测试和实际文件证据继续分层记录。

两个新的独立Cargo target冻结同一干净源码，实际native child、包装器、终端均退出0。
Tauri CLI `build --ci --no-bundle --no-sign --target x86_64-pc-windows-msvc`，Windows/CI配置，
Cargo `--locked --offline -vv`、npm offline、4 jobs；QA追加`recording-windows-av-qa`和同源码CRT配置。
绑定735份输入及完整Git tree；默认和QA输入一致。
vendored libvpx可下载固定SHA-256源码归档，Cargo offline不等于全程断网。原日志和第三方warning保留。

| 变体 | EXE字节数 | SHA-256 |
| --- | ---: | --- |
| 默认 | 28746752 | `976f39c951bed4432b8e4b2a32774f2b2163715999a597c1715e7a91c0d51220` |
| 录屏QA | 31715328 | `d4c7c7b1d330e11ad7a29c8c865454592b754936fb95150ab09bd1f2fff247c1` |

实际主程序rustc参数：opt-level=s、panic=abort、lto=fat、codegen-units=1、strip=symbols。
库和二进制feature指纹：默认无recording-*；QA启用WGC/WASAPI、VP9源码构建和Opus/WebM。
QA附加基准二进制只随Cargo build编译，未运行或采集设备，不计测试通过。
两份EXE均AMD64 PE32+、Windows GUI、嵌入证书目录为空。默认录屏入口仍关闭。
release panic=abort在进程panic终止时不执行Drop，原unwind测试不能证明该终止方式的清理。

## QA运行库与证据范围

既有10份release CRT的Microsoft签名、版本、AMD64 PE、哈希/来源及EXE相邻DLL/licenses文件核对。
直接/延迟导入和递归CRT闭包验证，所需`msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`。
只读文件，未加载应用或DLL；无系统CRT机器的启动尚未验证。保存的单个EXE不含相邻DLL，
完整未打包目录由QA RESULT的output.original标明，不视为已签名/可发布安装包。
证据在`src-tauri/target/current-release-default-b0b51f1/`、`current-release-qa-b0b51f1/`，
包含PID/退出码、源码树/输入哈希、日志、编译参数、feature、PE、CRT来源/部署/导入和实际产物。

W70同源码完整Windows门禁33/0/1 Linux smoke skipped，默认1298/QA1445各5ignored，前端81/1403；
领域479和新9项（7两图/2仅QA）已含总数。原API同字节1/8基线修复后全部通过，
原完整owner40ms/1920、60ms/2880PCM与实际文件解码内容保持；原模块/期限保持。
本轮未重跑测试，46项本地修复/45项历史和门禁计数保持；构建、运行库和文件检查不计通过数。

## 未完成项

当前SHA跨平台/codec CI、其它宿主、真实WGC/WASAPI、设备切换/漂移/长时同步、桌面、
安装器/updater/无CRT启动、Win10/多屏/混合DPI仍未验。W53/W59/W63/W67历史失败根因仍未明，
原日志/失败材料和历史release文件保留；已安装457包未更新，39项桌面记录保持。
未启动应用/设备/桌面，未安装/签名/新增工具/证书/推送/PR/合入/发布。
合成源和真实codec文件不替代设备验收，原规格最后复合验收及整体任务仍未完成。

只读续审发现“暂停后直接Stop”候选：混音Stop可能生成暂停期间的静音，而原worker拒绝暂停中入队。
原API诊断尚未执行，未选择或修改生产策略，不计新修复/测试。候选保存在ignored
`windows-mixed-paused-stop-followup.json`；后续须先冻结原源/worker/完整AV合同与根错误/前缀/原文件。
