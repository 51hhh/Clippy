# WIN-NATIVE-01 — QA MSI来源清单修复的Windows release验证

日期2026-10-04，W86；关联WIN-QA-MSI-PROVENANCE-01，规范[WIN-QA-MSI-PROVENANCE-01](../superpowers/specs/2026-10-04-windows-qa-msi-provenance.md)。
源码`228cc935bc0767ed9906f03ff5680cc89e72febe`，前置文档`164c536dc7ed1aecf8c36a690b3187ce67af672b`；分支`codex/windows-qa-msi-provenance-release-validation`。
默认与显式录屏QA在全新独立Cargo target上原生release编译完成，本轮源码/测试/新修复均0。

## 源码和构建

W85源码完整Windows门禁33/0/1、默认1341/QA1493各5ignored、前端81/1409保持实际身份。
752份编译输入与1073份完整门禁输入绑定，同一干净Git tree，两图原输入相等。
构建先冻结源码228cc93，结束后恢复164c536的文档HEAD；原Cargo.lock/vendor/前端/资源不改。
Tauri build --ci --no-bundle --no-sign --target x86_64-pc-windows-msvc，原Windows/CI配置，
Cargo --locked --offline -vv、npm offline、4jobs。QA另显式recording-windows-av-qa及当前源码CRT配置。
Cargo离线仍允许既有vendored libvpx脚本按固定哈希获取源码归档；没有安装或更新系统工具。
默认真实终端40117、QA终端78903，原生子进程/包装器/实际终端均exit0且终结。
PROGRESS仅作历史采样，结论取RESULT、日志、实际终端退出及已保存文件审计。

| 变体 | EXE字节数 | SHA-256 |
| --- | ---: | --- |
| 默认 | 28748288 | `025e42f4ab148d504d51fe925a564e8ea579d83f6182bdcc0c047cfdd572757e` |
| 录屏QA | 31719936 | `e0295b1f5578f7685a2f35e13933d3e3796ac17c4d4be0a06992b412751ed7c8` |

两主程序实际rustc参数opt-level=s、panic=abort、lto=fat、codegen-units=1、strip=symbols；
默认无recording-*及VPX指纹，QA包含Windows音频/A-V、来源VP9及Opus/WebM。QA基准二进制
只编译、未运行；不计测试。两EXE为AMD64 PE32+ Windows GUI，内嵌证书目录为空。
默认direct/delay DLL名称集合与8889192默认产物一致，无新增QA私有VC++名称；不是函数/API
等价、启动或旧Windows兼容性证明。源码panic=abort也不借用测试unwind Drop保证。

## canonical运行库部署和有限复核

当前干净源码准备真实SDK10份CRT，原Microsoft有效签名/完整版本/AMD64/来源/原字节保持。
staging清单basename为windows-qa-vc-runtime.json，实际unbundled部署为licenses/同名文件，
原PROVENANCE.json未落盘。配置/清单哈希/源码、十份相邻DLL及递归direct/delay依赖由生产
文件验证器核对；核心依赖仍msvcp140.dll、vcruntime140.dll、vcruntime140_1.dll。
固定VPX归档SHA、实际编译命令/feature指纹/PE与原始日志保存，不加载DLL。
完整部署目录见QA RESULT.output.original；独立保存的EXE不含相邻运行库，不能单独当完整QA包。

本轮沿准备器、文件验证器、build.rs、Cargo、基础/Windows/CI配置共7份文件作有限复核。
canonical源名/目标名一致，原输出路径/reparse、签名/完整版本/哈希/依赖、干净源码、配置
发布先后条件保持；payload仍以EXE相邻目录检查。此范围没有确认额外缺陷，不证明全项目正确。
原17基础许可证内容及MSI的三个basename差异未修改，W85记录仍保持各自范围。

## 证据边界

W85修复QA安装包的资源源码为228cc93、EXE编译为8889192，本轮没有重写那两个包的原来源。
本轮新release明确以228cc93编译，下一步才可据新EXE核对同源码NSIS/MSI文件；未打包/签名/安装。
current-release-default-228cc93、current-release-qa-228cc93、current-release-closure-228cc93、
isolated-release-228cc93位于主仓库src-tauri/target；原888/00f/D05构建、W85包及失败日志均保留。
本轮修复/新增用例/通过数0，52代码/部署修复与51历史不变；84定向合同及新4项已含1409总数。

同SHA CI仍not_run；没有新网络查询、推送或workflow，W85当前CI-gap与W83旧888实际查询
各保留原身份。其它宿主、真实首包/暂停恢复/设备/默认路由/声音/长时同步、WGC/焦点/DPI、
Win10/多屏混合DPI/负坐标、无CRT系统启动、安装升级卸载/updater/WebView2运行均未验。
旧457包桌面39项2passed/1failed/36not_run不含后续52修复，保持停止；历史失败根因仍未明。
原8R/9AC/47Tasks与末两条全局AC、QA CRT及canonical规范末项复合AC继续未完成。
无应用/设备/DLL加载、桌面、安装/信任/系统变更、PR/合入/发布；全局WIN-NATIVE-01未完成。

W87补充：当前编译/资源同为228cc93的默认/QA四个NSIS/MSI文件核对完成，QA两包
canonical清单/十份CRT生产文件验证均0；原EXE仅包格式三字节变化，W85旧包来源不改。
实际四包NotSigned，未运行/安装/签名/增计测试；同SHA CI、跨宿主与Native QA仍未验。
原未完成AC保持；见 [W87包文件](2026-10-04-windows-current-package-file-review.md)。
