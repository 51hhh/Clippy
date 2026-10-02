# WIN-NATIVE-01 / W76 — 混音停止预算源码的 Windows release 验证

源码 `2bb12f9eeb04f98217189c197ecafef83b326604`，前置文档 `3559ee55a6561aeccf2fe721dd7998a4d66ae041`，分支 `codex/windows-mixed-stop-ready-bound-release-validation`。
包含REC-MIXED-STOP-READY-BOUND-01与全部前置修复；本轮新源码/测试/修复0，原模块和期限保持。

## 编译与只读审查

只读复核混音器、音频worker、平台转发、Windows WASAPI和完整AV owner五个文件，未确认新缺陷。
Stop先分别验证原生时刻和全部有限输入批/预算，再冻结最终水位；输出每批最多48000帧，派生Stop覆盖取整末尾。
worker读取至空批后finish，后续批错误和背压保持abort/join；平台转发不隐式吞批，Windows有限排尾仍受endpoint容量约束。
暂停中实际尾部、两源Stop尝试、原错误身份、AV统计/journal结算保持。此代码审查不证明全部真实设备路径。

两个新的独立Cargo target从同一干净冻结源码完成默认/非默认QA release编译，native/包装器/实际终端exit0。
绑定完整Git tree和742份相同输入；Tauri build --ci --no-bundle --no-sign --target x86_64-pc-windows-msvc，Windows/CI配置，
Cargo --locked --offline -vv、npm offline、4 jobs；QA显式recording-windows-av-qa及同源码CRT配置。
vendored libvpx固定SHA256源码归档可获取，Cargo offline不等于全程断网；未下载安装器/系统工具。
原第三方warning和全部日志保留。QA基准二进制只随build编译，未运行或采集设备，不计测试。

| 变体 | EXE字节数 | SHA-256 |
| --- | ---: | --- |
| 默认 | 28746752 | `7876f9212f5f7bb892d7aadb1fdcba80c9892a4bcaaefb3fa5ac35989a74f694` |
| 录屏QA | 31716864 | `c5d3cdcf48d3781f3f0ccb9b57ce06a1671ce77f247123c9a08598d8422ba0f3` |

主程序真实rustc参数opt-level=s、panic=abort、lto=fat、codegen-units=1、strip=symbols；库/二进制feature指纹各自核对。
默认不含recording-*，QA启用WGC/WASAPI、VP9源码构建与Opus/WebM；默认录屏入口仍关闭。
两份EXE均AMD64 PE32+ Windows GUI、证书目录为空。release panic=abort终止时不执行unwind Drop，测试profile不能替代此终止方式验收。

## 文件依赖与证据范围

默认直接/延迟导入与前置02005c9默认PE一致，未新增QA私有VC++导入；不证明Win10或缺开发环境的启动。
QA既有10份Microsoft CRT签名/版本/AMD64/哈希/来源、相邻部署与license、直接/延迟/递归导入闭包核对，所需msvcp140.dll, vcruntime140.dll, vcruntime140_1.dll。
只读文件，未加载应用/DLL。单个保存EXE不包含旁边DLL；完整目录见QA RESULT.output.original，不视为已签名或可发布安装包。
证据在current-release-default-2bb12f9、current-release-qa-2bb12f9、current-release-closure-2bb12f9和isolated-release-2bb12f9。
构建PID/日志/退出码、源码输入/编译参数/feature/PE/CRT全部保留；PROGRESS中的未退出值仅为历史采样，最终以RESULT/实际终端为准。

W75同源码完整Windows门禁33/0/1 Linux smoke skipped，默认1316/QA1467各5ignored、前端81/1403，领域501/0保持。
新10项已含总数（8两图/2仅QA），两图不累加；原API九项2/7对照修复后通过，另一天间隔安全用例仅绿运行。
完整合成codec文件60ms/2880有效PCM、后续批错误interrupted、原完整模块/期限保持；本轮不重跑已通过测试，构建/文件不计新通过数。
48项本地修复/47历史保持。已安装457包、39项桌面记录2 passed/1 failed/36 not_run和旧release均保持历史身份。

## 尚未完成

当前2bb12f9提交GitHub API422不可取得，已知45769c9提交查询成功作为访问正例；combined status和PR-triggered首分页run为空仅作辅助，不能独自证明所有CI不存在。
没有当前SHA三native/四原型CI成功证据，也没有推送/触发CI。
其它宿主/Wayland、真实WGC/WASAPI设备/切换/漂移/长时同步/桌面、安装升级卸载/updater、无开发CRT启动、Win10/多屏/混合DPI/负坐标仍未验。
W53/W59/W63/W67原失败根因仍未明。没有应用/设备/桌面操作、安装、签名、证书/新增工具/推送/PR/合入/发布。
规格最后复合验收和全局任务仍未完成。
